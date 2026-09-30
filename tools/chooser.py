#!/usr/bin/env python3
"""The chooser: a small transformer that reads the sentence so far and
re-weighs the keyboard's candidates for the word being typed — calibrated,
choosing among what the search found rather than writing words.

Usage: tools/chooser.py [--data data/chooser] [--epochs 6] [--out data/chooser/model.pt]
                        [--export target/apk/assets/chooser.bin]

Data: tools/chooser_data.py (each word typed through the keyboard with
noisy taps, its candidates with their costs). For each word:

    logit(c) = −τ·cost(c) + f(context, c)

cost the keyboard's own (keys, the word's odds, pairs, lemmas, grammar), f
what the sentence adds: a transformer over up to 12 words before it (each
its lemma's vector — the same, frozen, as the phone's lemmas.bin — plus its
grammar class and place) read by a query token; f = its output · the
candidate's lemma vector and grammar class, plus a linear term on the
candidate's features (the edit up to EDIT_CAP, its odds, no lemma of its
own) — all known wherever the keyboard weighs a word, so f joins every
weighing (kbcore::chooser, `Engine::weigh`).
f = 0 is the keyboard as it is: the chooser learns only what the keyboard
misses. Trained by cross-entropy among the candidates (the word meant, ё
folded), so the probabilities are calibrated.

Reports on the validation sentences: how often the first candidate is the
word meant, the keyboard's order against the chooser's, by noise (careful:
any change is a false correction), and calibration.
"""
import argparse
import math
import os
import random
import struct
import sys
import time

import numpy as np
import torch
import torch.nn as nn

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CTX = 12
K = 8
FEATS = 4
# An edit counts up to this much in the features (the search's bound on f).
EDIT_CAP = 10.0

ap = argparse.ArgumentParser()
ap.add_argument("--data", default=f"{ROOT}/data/chooser")
ap.add_argument("--lemmas", default=f"{ROOT}/target/apk/assets/lemmas.bin")
ap.add_argument("--epochs", type=int, default=6)
ap.add_argument("--batch", type=int, default=512)
ap.add_argument("--lr", type=float, default=2e-3)
ap.add_argument("--d", type=int, default=64)
ap.add_argument("--layers", type=int, default=2)
ap.add_argument("--limit", type=int, default=0, help="training examples (0: all)")
ap.add_argument("--out", default=f"{ROOT}/data/chooser/model.pt")
ap.add_argument("--export")
ap.add_argument("--threads", type=int, default=6)
ap.add_argument("--graph", help="dependency parses (data/parse): the chooser also learns what the "
                "word being typed links to among the words before it, and reads that word")
ap.add_argument("--graph-weight", type=float, default=0.5)
ap.add_argument("--careful", type=float, default=2.0,
                help="weight of the careful words (a false correction costs more than a miss)")
args = ap.parse_args()
torch.set_num_threads(args.threads)
torch.manual_seed(1)
random.seed(1)
t0 = time.time()


def log(msg):
    print(f"[{time.time() - t0:6.0f}s] {msg}", file=sys.stderr, flush=True)


# The phone's lemma vectors (kbcore::lemmas), dequantized.
blob = open(args.lemmas, "rb").read()
assert blob[:4] == b"KBLM"
V, D = struct.unpack("<II", blob[8:16])
nc, nt = V + 2, V + 1
at = 16


def take(n, dtype, shape):
    global at
    a = np.frombuffer(blob, dtype=dtype, count=int(np.prod(shape)), offset=at).reshape(shape)
    at += a.nbytes
    return a


ctx_q = take(nc * D, np.int8, (nc, D))
ctx_s = take(nc, np.float32, (nc,))
take(nc, np.float32, (nc,))  # log Z
take(nc, np.float32, (nc,))  # most
tgt_q = take(nt * D, np.int8, (nt, D))
tgt_s = take(nt, np.float32, (nt,))
ctx_vec = torch.tensor(ctx_q.astype(np.float32) * ctx_s[:, None])
tgt_vec = torch.tensor(tgt_q.astype(np.float32) * tgt_s[:, None])
UNK, START = V, V + 1

lemma_of = {}
with open(f"{ROOT}/data/lemma/ids.tsv", encoding="utf-8") as f:
    for line in f:
        w, i = line.rstrip("\n").split("\t")
        lemma_of[w] = int(i)
class_of = {}
with open(f"{ROOT}/data/classes.tsv", encoding="utf-8") as f:
    for line in f:
        w, c = line.rstrip("\n").split("\t")
        class_of[w] = int(c)
N_CLASS = max(class_of.values()) + 1
counts = {}
with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) > 1:
            counts[p[0]] = int(p[1])
TOTAL = sum(counts.values())
FLOOR = math.log(TOTAL) - math.log(0.5)
log(f"{V} lemmas × {D}, {len(lemma_of)} words with a lemma, {N_CLASS} classes")


def fold(w):
    return w.replace("ё", "е")


def lemma(w):
    if w in lemma_of:
        return lemma_of[w]
    return UNK


def unigram(w):
    c = counts.get(w, 0)
    return math.log(TOTAL) - math.log(c) if c else FLOOR


parses = {}
if args.graph:
    for split in ("valid", "train"):
        path = f"{args.graph}/{split}.tsv"
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) >= 4:
                    parses[p[1]] = [int(h) for h in p[3].split()]


def links(heads, t):
    """The words before word t (0-based) it links to: its head, its dependents."""
    return [i for i in range(t) if heads[t] == i + 1 or heads[i] == t + 1]


N_ROLES = 5


def read(path, limit=0):
    """Examples: (noise, context words, typed, candidates, label, the word's
    place in its sentence, what it links to before it — None: no parse, the
    grammar's role of each context word — tools' `roles` example, a
    `.roles` file beside the data)."""
    rows = []
    sentence, start = [], 0
    roles_path = path[: -len(".tsv")] + ".roles"
    roles_f = open(roles_path, encoding="utf-8") if args.graph and os.path.exists(roles_path) else None

    def close():
        heads = parses.get(" ".join(sentence))
        ok = heads is not None and len(heads) == len(sentence)
        for i in range(start, len(rows)):
            r = rows[i]
            t = r[6]
            rows[i] = r[:5] + (t, links(heads, t) if ok and t < len(heads) else None, r[5])

    with open(path, encoding="utf-8") as f:
        for line in f:
            role_line = roles_f.readline() if roles_f else ""
            p = line.rstrip("\n").split("\t")
            if len(p) < 5 or not p[4]:
                continue
            noise, before, meant, typed, cands = p
            roles = [int(x) for x in role_line.split()][-CTX:]
            if not before.strip() and typed:
                if sentence:
                    close()
                sentence, start = [], len(rows)
            pos = len(sentence)
            if typed:
                sentence.append(meant)
            sent = before
            for ch in ".!?":
                sent = sent.split(ch)[-1]
            words = [w for w in sent.lower().split() if w][-CTX:]
            cs = []
            for c in cands.split("|")[:K]:
                w, cost, edit = c.rsplit(":", 2)
                cs.append((w, float(cost), float(edit)))
            label = next((i for i, c in enumerate(cs) if fold(c[0]) == fold(meant)), -1)
            rows.append((noise, words, typed, cs, label, roles, pos))
            if limit and len(rows) >= limit:
                break
    if sentence:
        close()
    return rows


def tensors(rows):
    n = len(rows)
    cl = torch.full((n, CTX), 0, dtype=torch.long)
    cc = torch.zeros((n, CTX), dtype=torch.long)
    cm = torch.ones((n, CTX), dtype=torch.bool)  # True: padding
    kl = torch.full((n, K), UNK, dtype=torch.long)
    kc = torch.zeros((n, K), dtype=torch.long)
    kf = torch.zeros((n, K, FEATS))
    cost = torch.zeros((n, K))
    km = torch.ones((n, K), dtype=torch.bool)
    y = torch.zeros(n, dtype=torch.long)
    wt = torch.ones(n)
    ptr = torch.zeros((n, CTX + 1))
    parsed = torch.zeros(n, dtype=torch.bool)
    cr = torch.zeros((n, CTX), dtype=torch.long)
    for i, (noise, words, typed, cs, label, t, linked, roles) in enumerate(rows):
        if words and len(roles) == len(words):
            cr[i, CTX - len(words):] = torch.tensor(roles)
        if noise == "careful":
            wt[i] = args.careful
        seq = [(START, 0)] if not words else []
        seq += [(lemma(w), class_of.get(w, 0)) for w in words]
        seq = seq[-CTX:]
        for j, (l, c) in enumerate(seq):
            cl[i, CTX - len(seq) + j] = l
            cc[i, CTX - len(seq) + j] = c
            cm[i, CTX - len(seq) + j] = False
        if linked is not None:
            parsed[i] = True
            # Sentence word k sits at slot CTX − len(words) + (k − (t − len(words))).
            slots = [CTX - t + k for k in linked if k >= t - len(words)] if words else []
            for sl in slots:
                ptr[i, sl] = 1.0 / len(slots)
            if not slots:
                ptr[i, CTX] = 1.0
        best = cs[0][1]
        for j, (w, c, e) in enumerate(cs):
            kl[i, j] = lemma(w)
            kc[i, j] = class_of.get(w, 0)
            kf[i, j] = torch.tensor([min(e, EDIT_CAP), unigram(w) / 10, float(w not in lemma_of),
                                     float(typed == "")])
            cost[i, j] = c
            km[i, j] = False
        y[i] = max(label, 0)
    return cl, cc, cm, kl, kc, kf, cost, km, y, wt, ptr, parsed, cr


class Chooser(nn.Module):
    def __init__(self, d, layers):
        super().__init__()
        self.ctx_vec = nn.Parameter(ctx_vec, requires_grad=False)
        self.tgt_vec = nn.Parameter(tgt_vec, requires_grad=False)
        self.ctx_proj = nn.Linear(D, d)
        self.ctx_class = nn.Embedding(N_CLASS, d)
        self.pos = nn.Embedding(CTX + 1, d)
        self.query = nn.Parameter(torch.zeros(d))
        layer = nn.TransformerEncoderLayer(d, 4, 2 * d, dropout=0.1, batch_first=True,
                                           norm_first=True, activation="relu")
        self.enc = nn.TransformerEncoder(layer, layers, enable_nested_tensor=False)
        self.norm = nn.LayerNorm(d)
        self.head = nn.Linear(d, d)
        self.tgt_proj = nn.Linear(D, d)
        self.tgt_class = nn.Embedding(N_CLASS, d)
        self.feat = nn.Linear(FEATS, 1)
        self.tau = nn.Parameter(torch.tensor(1.0))
        self.graph = bool(args.graph)
        if self.graph:
            # What the word links to: the query points at words before it;
            # what it points at is read into the candidate's score.
            self.ptr_q = nn.Linear(d, d)
            self.ptr_k = nn.Linear(d, d)
            self.linked = nn.Linear(d, d)
            # The grammar's roles: what each word is to the word coming, and
            # the chance of each role that the word links to it.
            self.role = nn.Embedding(N_ROLES, d)
            self.role_chance = nn.Parameter(torch.zeros(N_ROLES))
        self.pointer = None
        self.roles = None

    def residual(self, cl, cc, cm, kl, kc, kf):
        n, d = cl.shape[0], self.query.shape[0]
        x = self.ctx_proj(self.ctx_vec[cl]) + self.ctx_class(cc)
        if self.graph and self.roles is not None:
            x = x + self.role(self.roles)
        x = torch.cat([x, self.query.expand(n, 1, -1)], dim=1) + self.pos.weight
        mask = torch.cat([cm, torch.zeros((n, 1), dtype=torch.bool)], dim=1)
        out = self.norm(self.enc(x, src_key_padding_mask=mask))
        h = self.head(out[:, -1])
        c = self.tgt_proj(self.tgt_vec[kl]) + self.tgt_class(kc)
        f = (c @ h.unsqueeze(-1)).squeeze(-1) / math.sqrt(d) + self.feat(kf).squeeze(-1)
        if self.graph:
            s = (self.ptr_k(out) @ self.ptr_q(out[:, -1]).unsqueeze(-1)).squeeze(-1) / math.sqrt(d)
            if self.roles is not None:
                s = s + torch.cat([self.role_chance[self.roles],
                                   torch.zeros((n, 1))], dim=1)
            s = s.masked_fill(mask, -1e9)
            self.pointer = torch.log_softmax(s, dim=1)
            g = self.linked((self.pointer.exp().unsqueeze(-1) * out).sum(1))
            f = f + (c @ g.unsqueeze(-1)).squeeze(-1) / math.sqrt(d)
        return f

    def forward(self, cl, cc, cm, kl, kc, kf, cost, km):
        z = -self.tau * cost + self.residual(cl, cc, cm, kl, kc, kf)
        return z.masked_fill(km, -1e9)


def evaluate(model, rows, t):
    model.eval()
    with torch.no_grad():
        z = torch.cat([model(*[x[i:i + 4096] for x in t[:8]]) for i in range(0, len(rows), 4096)])
    model.train()
    p = torch.softmax(z, dim=1)
    pick = p.argmax(1)
    by = {}
    ll, hits, conf = 0.0, [], []
    for i, (noise, _, typed, cs, label, *_) in enumerate(rows):
        s = by.setdefault(noise + ("" if typed else " next"), [0, 0, 0, 0, 0])
        s[0] += 1
        if label < 0:
            continue
        s[1] += 1
        s[2] += label == 0
        s[3] += int(pick[i]) == label
        s[4] += label == 0 and int(pick[i]) != 0
        ll -= math.log(max(float(p[i, label]), 1e-9))
        conf.append(float(p[i, pick[i]]))
        hits.append(int(pick[i]) == label)
    n = sum(s[1] for s in by.values())
    # Calibration: confidence against accuracy in 10 bins.
    conf, hits = np.array(conf), np.array(hits)
    ece = sum(abs(conf[m].mean() - hits[m].mean()) * m.sum()
              for b in range(10) if (m := (conf >= b / 10) & (conf < (b + 1) / 10)).any()) / len(conf)
    lines = [f"log-loss {ll / n:.4f}, calibration error {ece:.4f}"]
    if model.graph:
        # How often the query points at a word the parse links it to.
        with torch.no_grad():
            model.eval()
            hit = tot = 0
            for i in range(0, len(rows), 4096):
                model(*[x[i:i + 4096] for x in t[:8]])
                ptr, parsed = t[10][i:i + 4096], t[11][i:i + 4096]
                pick = model.pointer.argmax(1)
                ok = parsed & (ptr[:, CTX] == 0)
                hit += int((ptr[torch.arange(len(pick)), pick] > 0)[ok].sum())
                tot += int(ok.sum())
            model.train()
        lines.append(f"  the word's link before it found: {hit / max(tot, 1):.1%} of {tot}")
    for noise in sorted(by):
        s = by[noise]
        lines.append(f"  {noise:>12}: {s[0]} words, meant among candidates {s[1] / s[0]:.1%}; "
                     f"first: keyboard {s[2] / s[1]:.2%} → chooser {s[3] / s[1]:.2%} "
                     f"(keyboard's right first changed: {s[4]})")
    return "\n".join(lines)


train_rows = read(f"{args.data}/train.tsv", args.limit)
valid_rows = read(f"{args.data}/valid.tsv")
log(f"{len(train_rows)} training words, {len(valid_rows)} validation")
train_rows = [r for r in train_rows if r[4] >= 0]
tt = tensors(train_rows)
vt = tensors(valid_rows)
log(f"tensors ready ({len(train_rows)} with the word meant among the candidates)")

model = Chooser(args.d, args.layers)
print("before:", evaluate(model, valid_rows, vt).replace("chooser", "τ-scaled"), flush=True)
opt = torch.optim.AdamW([p for p in model.parameters() if p.requires_grad], lr=args.lr,
                        weight_decay=1e-4)
steps = args.epochs * math.ceil(len(train_rows) / args.batch)
sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=args.lr, total_steps=steps)
lossf = nn.CrossEntropyLoss(reduction="none")


def valid_loss(model):
    model.eval()
    with torch.no_grad():
        z = torch.cat([model(*[x[i:i + 4096] for x in vt[:8]]) for i in range(0, len(valid_rows), 4096)])
    model.train()
    ok = torch.tensor([r[4] >= 0 for r in valid_rows])
    return float(lossf(z[ok], vt[8][ok]).mean())


best = (math.inf, None)
for epoch in range(1, args.epochs + 1):
    perm = torch.randperm(len(train_rows))
    total = 0.0
    for i in range(0, len(perm), args.batch):
        b = perm[i:i + args.batch]
        z = model(*[x[b] for x in tt[:8]])
        loss = (lossf(z, tt[8][b]) * tt[9][b]).sum() / tt[9][b].sum()
        if model.graph:
            parsed = tt[11][b]
            if parsed.any():
                ptr_loss = -(tt[10][b] * model.pointer).sum(1)[parsed].mean()
                loss = loss + args.graph_weight * ptr_loss
        opt.zero_grad()
        loss.backward()
        opt.step()
        sched.step()
        total += loss.item() * len(b)
    vl = valid_loss(model)
    log(f"epoch {epoch}: train loss {total / len(train_rows):.4f}, valid {vl:.4f}, τ {float(model.tau):.3f}")
    print(f"epoch {epoch}:", evaluate(model, valid_rows, vt), flush=True)
    if vl < best[0]:
        best = (vl, {k: v.clone() for k, v in model.state_dict().items()})
model.load_state_dict(best[1])
torch.save(model.state_dict(), args.out)
log(f"wrote {args.out} (valid {best[0]:.4f})")


def export(model, path):
    """The weights for kbcore::chooser (little-endian f32, this order)."""
    sd = model.state_dict()
    enc = [f"enc.layers.{i}." for i in range(args.layers)]
    names = ["ctx_proj.weight", "ctx_proj.bias", "ctx_class.weight", "pos.weight", "query"]
    for e in enc:
        names += [e + n for n in ("norm1.weight", "norm1.bias", "self_attn.in_proj_weight",
                                  "self_attn.in_proj_bias", "self_attn.out_proj.weight",
                                  "self_attn.out_proj.bias", "norm2.weight", "norm2.bias",
                                  "linear1.weight", "linear1.bias", "linear2.weight",
                                  "linear2.bias")]
    names += ["norm.weight", "norm.bias", "head.weight", "head.bias", "tgt_proj.weight",
              "tgt_proj.bias", "tgt_class.weight", "feat.weight", "feat.bias", "tau"]
    with open(path + ".part", "wb") as f:
        f.write(b"KBCH" + struct.pack("<8I", 1, args.d, args.layers, 4, 2 * args.d, D,
                                      N_CLASS, CTX) + struct.pack("<I", FEATS))
        n = 0
        for name in names:
            a = sd[name].detach().cpu().numpy().astype("<f4").ravel()
            f.write(a.tobytes())
            n += a.size
    os.replace(path + ".part", path)
    log(f"wrote {path}: {n} numbers, {os.path.getsize(path) / 1e6:.2f} MB")


if args.export:
    export(model, args.export)
