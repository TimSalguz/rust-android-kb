#!/usr/bin/env python3
"""The student parser: the chances of every link of a sentence's graph —
which word each word hangs on, and how — on the keyboard's own structures
(each word's lemma vector from lemmas.bin, its grammar class, the
grammemes of all its readings), taught by the parses of data/parse.

Usage: tools/graph_parser.py [--causal] [--epochs 5] [--out data/graph/parser.pt]

The graph keeps every link the grammar allows (tools/graph_grammar.py: the
true parse is in it for 99.8% of sentences); the parser only weighs them.
`--causal`: as the keyboard reads a sentence, word by word — each word sees
the words before it only, and its head is one of them, the root, or a word
still to come; and a word waiting for its head, how it will hang on it (a
subject for its predicate, an adjective for its noun, an adverbial) — what
it must agree with the word to come in.

Measured on the held-out sentences against their parses: the head ranked
first (UAS; with the relation, LAS), the true head among the first three,
and how well the chances are calibrated.
"""
import argparse
import math
import os
import struct
import sys
import time

import numpy as np
import torch
import torch.nn as nn

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--parse", default=f"{ROOT}/data/parse")
ap.add_argument("--lemmas", default=f"{ROOT}/target/apk/assets/lemmas.bin")
ap.add_argument("--causal", action="store_true")
ap.add_argument("--epochs", type=int, default=5)
ap.add_argument("--batch", type=int, default=512)
ap.add_argument("--lr", type=float, default=1e-3)
ap.add_argument("--d", type=int, default=128)
ap.add_argument("--layers", type=int, default=4)
ap.add_argument("--limit", type=int, default=0)
ap.add_argument("--out")
args = ap.parse_args()
dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
torch.manual_seed(1)
t0 = time.time()


def log(msg):
    print(f"[{time.time() - t0:5.0f}s] {msg}", file=sys.stderr, flush=True)


# The phone's lemma vectors (context rows), dequantized.
blob = open(args.lemmas, "rb").read()
V, D = struct.unpack("<II", blob[8:16])
nc = V + 2
q = np.frombuffer(blob, dtype=np.int8, count=nc * D, offset=16).reshape(nc, D).astype(np.float32)
s = np.frombuffer(blob, dtype=np.float32, count=nc, offset=16 + nc * D)
ctx_vec = torch.tensor(q * s[:, None])
UNK = V
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
sets = {}
with open(f"{ROOT}/data/readings.tsv", encoding="utf-8") as f:
    for line in f:
        i, tags = line.rstrip("\n").split("\t")
        sets[int(i)] = tags.split("|")
word_set = {}
with open(f"{ROOT}/data/word_readings.tsv", encoding="utf-8") as f:
    for line in f:
        w, i = line.rstrip("\n").split("\t")
        word_set[w] = int(i)
grammemes = sorted({g for tags in sets.values() for t in tags for g in t.split(",")})
G = {g: i for i, g in enumerate(grammemes)}
bag_of_word, bag_ids, bag_rows = {}, {}, [np.zeros(len(G), dtype=np.float32)]


def bag(word):
    """The number of the set of grammemes of all the word's readings (its
    ambiguity kept); 0: none known."""
    if word not in bag_of_word:
        key = tuple(sorted({G[g] for t in sets.get(word_set.get(word, -1), []) for g in t.split(",")}))
        if key and key not in bag_ids:
            bag_ids[key] = len(bag_rows)
            v = np.zeros(len(G), dtype=np.float32)
            v[list(key)] = 1.0
            bag_rows.append(v)
        bag_of_word[word] = bag_ids.get(key, 0)
    return bag_of_word[word]


RELS = {}


def read(path, limit=0):
    rows = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) < 5:
                continue
            words = p[1].split()
            heads = [int(h) for h in p[3].split()]
            rels = [RELS.setdefault(r.split(":")[0], len(RELS)) for r in p[4].split()]
            rows.append((words, heads, rels))
            if limit and len(rows) >= limit:
                break
    return rows


L = 13  # the root and at most 12 words


def tensors(rows):
    n = len(rows)
    lem = torch.full((n, L), UNK, dtype=torch.long)
    cls = torch.zeros((n, L), dtype=torch.long)
    bags = torch.zeros((n, L), dtype=torch.long)
    mask = torch.ones((n, L), dtype=torch.bool)
    head = torch.zeros((n, L), dtype=torch.long)
    rel = torch.zeros((n, L), dtype=torch.long)
    mask[:, 0] = False
    for i, (words, heads, rels) in enumerate(rows):
        for j, w in enumerate(words[: L - 1]):
            lem[i, j + 1] = lemma_of.get(w, UNK)
            cls[i, j + 1] = class_of.get(w, 0)
            bags[i, j + 1] = bag(w)
            mask[i, j + 1] = False
            head[i, j + 1] = heads[j]
            rel[i, j + 1] = rels[j]
    return lem, cls, bags, mask, head, rel


class Parser(nn.Module):
    def __init__(self, d, layers):
        super().__init__()
        self.ctx_vec = nn.Parameter(ctx_vec, requires_grad=False)
        self.lemma = nn.Linear(D, d)
        self.cls = nn.Embedding(N_CLASS, d)
        self.gram = nn.Linear(len(G), d)
        self.register_buffer("bag_rows", torch.tensor(np.stack(bag_rows)))
        self.pos = nn.Embedding(L, d)
        self.root = nn.Parameter(torch.zeros(d))
        layer = nn.TransformerEncoderLayer(d, 4, 2 * d, dropout=0.1, batch_first=True, norm_first=True)
        self.enc = nn.TransformerEncoder(layer, layers, enable_nested_tensor=False)
        self.norm = nn.LayerNorm(d)
        self.dep = nn.Sequential(nn.Linear(d, d), nn.ReLU(), nn.Linear(d, d))
        self.hd = nn.Sequential(nn.Linear(d, d), nn.ReLU(), nn.Linear(d, d))
        self.later = nn.Parameter(torch.zeros(d))  # causal: the head is still to come
        self.rel = nn.Sequential(nn.Linear(2 * d, d), nn.ReLU(), nn.Linear(d, len(RELS)))
        # Causal: a word waiting, its relation to the head still to come.
        self.wait_rel = nn.Sequential(nn.Linear(d, d), nn.ReLU(), nn.Linear(d, len(RELS)))

    def forward(self, lem, cls, bags, mask):
        n = lem.shape[0]
        x = self.lemma(self.ctx_vec[lem]) + self.cls(cls) + self.gram(self.bag_rows[bags])
        x[:, 0] = self.root
        x = x + self.pos.weight
        att = None
        if args.causal:
            att = torch.triu(torch.ones(L, L, dtype=torch.bool, device=lem.device), diagonal=1)
        h = self.norm(self.enc(x, mask=att, src_key_padding_mask=mask))
        dep, hd = self.dep(h), self.hd(h)
        s = dep @ hd.transpose(1, 2) / math.sqrt(dep.shape[-1])  # [n, dependent, head]
        bad = mask.unsqueeze(1).expand(-1, L, -1).clone()
        bad |= torch.eye(L, dtype=torch.bool, device=lem.device)
        if args.causal:
            # A head before the word, the root, or "later" (column 0 stands for
            # the root; "later" is appended).
            bad |= torch.triu(torch.ones(L, L, dtype=torch.bool, device=lem.device), diagonal=1)
            later = (dep @ self.later) / math.sqrt(dep.shape[-1])
            s = torch.cat([s.masked_fill(bad, -1e9), later.unsqueeze(-1)], dim=-1)
        else:
            s = s.masked_fill(bad, -1e9)
        return s, h


def gold_heads(head):
    """Causal: a head after the word becomes "later" (index L)."""
    if not args.causal:
        return head
    pos = torch.arange(L, device=head.device).unsqueeze(0)
    return torch.where(head > pos, torch.full_like(head, L), head)


def evaluate(model, t):
    model.eval()
    tot = uas = las = top3 = 0
    waiting = wait_ok = 0
    conf, hit = [], []
    with torch.no_grad():
        for i in range(0, t[0].shape[0], 2048):
            b = [x[i:i + 2048].to(dev) for x in t]
            lem, cls, bags, mask, head, rel = b
            s, h = model(lem, cls, bags, mask)
            g = gold_heads(head)
            p = torch.softmax(s, dim=-1)
            pick = p.argmax(-1)
            real = ~mask
            real[:, 0] = False
            top = p.topk(3, dim=-1).indices
            hidx = pick.clamp(max=L - 1)
            rl = model.rel(torch.cat([h, torch.gather(h, 1, hidx.unsqueeze(-1).expand(-1, -1, h.shape[-1]))], -1)).argmax(-1)
            ok = (pick == g) & real
            if args.causal:
                w = (g == L) & real
                waiting += int(w.sum())
                wait_ok += int(((model.wait_rel(h).argmax(-1) == rel) & w).sum())
            tot += int(real.sum())
            uas += int(ok.sum())
            las += int((ok & (rl == rel)).sum())
            top3 += int(((top == g.unsqueeze(-1)).any(-1) & real).sum())
            conf.append(p.max(-1).values[real].cpu())
            hit.append(ok[real].cpu())
    model.train()
    conf, hit = torch.cat(conf).numpy(), torch.cat(hit).numpy().astype(float)
    ece = sum(abs(conf[m].mean() - hit[m].mean()) * m.sum()
              for b in range(10) if (m := (conf >= b / 10) & (conf < (b + 1) / 10)).any()) / len(conf)
    waits = f"; the relation of a word waiting {wait_ok / waiting:.2%} ({waiting})" if waiting else ""
    return (f"head first {uas / tot:.2%}, with the relation {las / tot:.2%}, "
            f"among the first three {top3 / tot:.2%}; calibration error {ece:.4f}{waits}")


train = read(f"{args.parse}/train.tsv", args.limit)
valid = read(f"{args.parse}/valid.tsv")
log(f"{len(train)} training sentences, {len(valid)} held out, {len(RELS)} relations, {len(G)} grammemes; {dev}")
tt = tensors(train)
vt = tensors(valid)
log("tensors ready")
model = Parser(args.d, args.layers).to(dev)
opt = torch.optim.AdamW([p for p in model.parameters() if p.requires_grad], lr=args.lr, weight_decay=1e-4)
steps = args.epochs * math.ceil(len(train) / args.batch)
sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=args.lr, total_steps=steps)
ce = nn.CrossEntropyLoss(reduction="none")
for epoch in range(1, args.epochs + 1):
    perm = torch.randperm(len(train))
    total = 0.0
    for i in range(0, len(perm), args.batch):
        idx = perm[i:i + args.batch]
        lem, cls, bags, mask, head, rel = [x[idx].to(dev) for x in tt]
        s, h = model(lem, cls, bags, mask)
        g = gold_heads(head)
        real = ~mask
        real[:, 0] = False
        arc = ce(s.reshape(-1, s.shape[-1]), g.reshape(-1)).reshape(g.shape)
        gh = head.clamp(max=L - 1)
        rl = model.rel(torch.cat([h, torch.gather(h, 1, gh.unsqueeze(-1).expand(-1, -1, h.shape[-1]))], -1))
        rloss = ce(rl.reshape(-1, rl.shape[-1]), rel.reshape(-1)).reshape(rel.shape)
        if args.causal:
            wl = model.wait_rel(h)
            wloss = ce(wl.reshape(-1, wl.shape[-1]), rel.reshape(-1)).reshape(rel.shape)
            rloss = rloss + wloss * (g == L)
        loss = ((arc + rloss) * real).sum() / real.sum()
        opt.zero_grad()
        loss.backward()
        opt.step()
        sched.step()
        total += loss.item() * len(idx)
    log(f"epoch {epoch}: loss {total / len(train):.4f}")
    print(f"epoch {epoch} ({'as typed' if args.causal else 'whole sentence'}): {evaluate(model, vt)}", flush=True)
params = sum(p.numel() for p in model.parameters() if p.requires_grad)
print(f"{params} parameters (the lemma vectors, {nc}×{D}, shared and frozen)", flush=True)
if args.out:
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    torch.save({"model": model.state_dict(), "rels": RELS, "grammemes": grammemes}, args.out)
    log(f"wrote {args.out}")
