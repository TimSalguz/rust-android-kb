#!/usr/bin/env python3
"""Lemma vectors: which lemma comes after which, as a low-rank model.

Usage: tools/lemma_vectors.py [--dir data/lemma] [--vocab 50000] [--dims 32]
                              [--epochs 60] [--out data/lemma/vectors32.npz]

From tools/lemma_corpus.py's numbered text. The previous word's lemma c
(or START: a sentence's first word, the first after a number or a comma)
and the next word's lemma w:

    P(w | c) = softmax_w(u_c · v_w + b_w)

over the `--vocab` commonest lemmas and UNK (every other lemma, one class).
u and v are the lemma vectors — u as the context, v as what comes — and the
product is a smooth PMI: lemmas never seen together still get a fair chance
from what their neighbours share. Learned by full softmax on the pair counts
(Adam), so the probabilities are calibrated: the keyboard's decisions are
thresholds.

Measured on the held-out sentences against the unigram and the full table of
lemma pairs (interpolated Kneser–Ney — every pair ever seen, the size to
beat): cross-entropy in bits per word, top-1 and top-10 of the next lemma,
also with the vectors rounded to a byte a number (what the phone would keep).
"""
import argparse
import math
import os
import sys
import time

os.environ.setdefault("OPENBLAS_NUM_THREADS", "4")
import numpy as np  # noqa: E402
import scipy.sparse as sp  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--dir", default="data/lemma")
ap.add_argument("--vocab", type=int, default=50000)
ap.add_argument("--dims", type=int, default=32)
ap.add_argument("--epochs", type=int, default=60)
ap.add_argument("--lr", type=float, default=0.01)
ap.add_argument("--batch", type=int, default=1024)
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--out")
args = ap.parse_args()
rng = np.random.default_rng(args.seed)
t0 = time.time()


def log(msg):
    print(f"[{time.time() - t0:6.0f}s] {msg}", file=sys.stderr, flush=True)


# Forms → lemmas; lemma ids by training count.
form_lemma, lemma_count, pos_of = [], {}, {}
with open(f"{args.dir}/forms.tsv", encoding="utf-8") as f:
    for line in f:
        form, count, lemma, pos = line.rstrip("\n").split("\t")
        form_lemma.append(lemma)
        lemma_count[lemma] = lemma_count.get(lemma, 0) + int(count)
        pos_of.setdefault(lemma, pos)
lemmas = sorted(lemma_count, key=lambda l: -lemma_count[l])[: args.vocab]
V = len(lemmas)
UNK, START = V, V + 1
lemma_id = {l: i for i, l in enumerate(lemmas)}
form_to = np.array([lemma_id.get(l, UNK) for l in form_lemma], dtype=np.int64)
log(f"{len(form_lemma)} forms, {len(lemma_count)} lemmas, {V} kept "
    f"({sum(lemma_count[l] for l in lemmas) / sum(lemma_count.values()):.1%} of the words)")


def pairs(path):
    """(context, target) → count over a numbered text, as a sparse matrix."""
    t = np.load(path).astype(np.int64)
    words = np.flatnonzero(t >= 0)
    prev = np.where(words > 0, t[np.maximum(words - 1, 0)], -1)
    ctx = np.where(prev >= 0, form_to[np.maximum(prev, 0)], START)
    tgt = form_to[t[words]]
    code, n = np.unique(ctx * (V + 1) + tgt, return_counts=True)
    return sp.csr_matrix((n.astype(np.float64), (code // (V + 1), code % (V + 1))),
                         shape=(V + 2, V + 1))


C = pairs(f"{args.dir}/train.npy")
H = pairs(f"{args.dir}/heldout.npy").tocoo()
N = C.sum()
log(f"train {N:.0f} words, {C.nnz} distinct lemma pairs; held out {H.sum():.0f} words")
hc, ht, hn = H.row, H.col, H.data
H_total = hn.sum()


def report(name, logp, top1=None, top10=None, extra=""):
    bits = -(hn * logp).sum() / H_total / math.log(2)
    s = f"{name:<28} {bits:6.3f} bits  ppl {2 ** bits:7.1f}"
    if top1 is not None:
        s += f"  top-1 {top1:5.1%}  top-10 {top10:5.1%}"
    print(s + extra, flush=True)


# The unigram.
uni = np.asarray(C.sum(axis=0)).ravel() + 0.5
uni /= uni.sum()
order = np.argsort(-uni[:V])
known = ht < V
report("unigram", np.log(uni[ht]),
       (hn * (ht == order[0])).sum() / H_total,
       (hn * np.isin(ht, order[:10])).sum() / H_total)

# Every pair seen: interpolated Kneser–Ney, D = 0.75.
D = 0.75
row_n = np.asarray(C.sum(axis=1)).ravel()
row_types = np.diff(C.indptr)
cont = np.asarray((C > 0).sum(axis=0)).ravel() + 0.5
cont /= cont.sum()
Ch = np.asarray(C[hc, ht]).ravel()
with np.errstate(divide="ignore", invalid="ignore"):
    lam = np.where(row_n[hc] > 0, D * row_types[hc] / np.maximum(row_n[hc], 1), 1.0)
    p_kn = np.where(row_n[hc] > 0, np.maximum(Ch - D, 0) / np.maximum(row_n[hc], 1), 0) + lam * cont[ht]
# top-k of the table: the context's likeliest seen continuations, then the rest by cont.
def kn_top(c, k):
    lo, hi = C.indptr[c], C.indptr[c + 1]
    cols, vals = C.indices[lo:hi], C.data[lo:hi]
    s = np.maximum(vals - D, 0) / max(row_n[c], 1) + D * (hi - lo) / max(row_n[c], 1) * cont[cols]
    best = cols[np.argsort(-s)]
    best = best[best < V][:k]
    if len(best) < k:
        rest = [w for w in order[: k * 2] if w not in set(best)]
        best = np.concatenate([best, rest[: k - len(best)]])
    return best


def topk_rate(top_of):
    hit1 = hit10 = 0.0
    for c in np.unique(hc):
        m = hc == c
        best = top_of(c)
        hit1 += (hn[m] * (ht[m] == best[0])).sum()
        hit10 += (hn[m] * np.isin(ht[m], best[:10])).sum()
    return hit1 / H_total, hit10 / H_total


t1, t10 = topk_rate(lambda c: kn_top(c, 10))
report("pairs, all seen (KN)", np.log(p_kn), t1, t10,
       f"  — {C.nnz} pairs ≈ {C.nnz * 5 / 1e6:.0f} MB at 5 bytes a pair")

# The low-rank model.
d = args.dims
U = (rng.standard_normal((V + 2, d)) * 0.1).astype(np.float32)
W = (rng.standard_normal((V + 1, d)) * 0.1).astype(np.float32)
b = np.log(uni).astype(np.float32)
params = [U, W, b]
m1 = [np.zeros_like(p) for p in params]
m2 = [np.zeros_like(p) for p in params]
beta1, beta2, eps = 0.9, 0.999, 1e-8
rows = np.flatnonzero(row_n > 0)
Cf = C.astype(np.float32).tocsr()
step = 0


def logits(ctx_ids, U, W, b):
    z = U[ctx_ids] @ W.T + b
    z -= z.max(axis=1, keepdims=True)
    return z - np.log(np.exp(z).sum(axis=1, keepdims=True))


def held_logp(U, W, b):
    out = np.empty(len(hc))
    top1 = top10 = 0.0
    uc = np.unique(hc)
    for i in range(0, len(uc), 2048):
        cs = uc[i:i + 2048]
        lp = logits(cs, U, W, b)
        top = np.argpartition(-lp[:, :V], 10, axis=1)[:, :10]
        first = np.argmax(lp[:, :V], axis=1)
        where = {c: j for j, c in enumerate(cs)}
        m = np.isin(hc, cs)
        idx = np.flatnonzero(m)
        j = np.array([where[c] for c in hc[idx]])
        out[idx] = lp[j, ht[idx]]
        top1 += (hn[idx] * (first[j] == ht[idx])).sum()
        top10 += (hn[idx] * (top[j] == ht[idx][:, None]).any(axis=1)).sum()
    return out, top1 / H_total, top10 / H_total


best = (math.inf, None)
for epoch in range(1, args.epochs + 1):
    rng.shuffle(rows)
    loss = 0.0
    for i in range(0, len(rows), args.batch):
        bi = rows[i:i + args.batch]
        cb = Cf[bi]
        n_b = row_n[bi].astype(np.float32)
        lp = logits(bi, U, W, b)
        G = np.exp(lp) * n_b[:, None]
        cb = cb.tocoo()
        G[cb.row, cb.col] -= cb.data
        loss -= (cb.data * lp[cb.row, cb.col]).sum()
        G /= N
        gU = G @ W
        gW = G.T @ U[bi]
        gb = G.sum(axis=0)
        step += 1
        c1, c2 = 1 - beta1 ** step, 1 - beta2 ** step
        for k, g, idx in ((0, gU, bi), (1, gW, slice(None)), (2, gb, slice(None))):
            m1[k][idx] = beta1 * m1[k][idx] + (1 - beta1) * g
            m2[k][idx] = beta2 * m2[k][idx] + (1 - beta2) * g * g
            params[k][idx] -= args.lr * (m1[k][idx] / c1) / (np.sqrt(m2[k][idx] / c2) + eps)
    if epoch % 5 == 0 or epoch == args.epochs:
        lp, t1, t10 = held_logp(U, W, b)
        bits = -(hn * lp).sum() / H_total / math.log(2)
        log(f"epoch {epoch}: train {loss / N / math.log(2):.3f} bits, held out {bits:.3f} bits")
        if bits < best[0]:
            best = (bits, (U.copy(), W.copy(), b.copy()))
        elif bits > best[0] + 0.01:
            break

U, W, b = best[1]
lp, t1, t10 = held_logp(U, W, b)
size = ((V + 2) + (V + 1)) * d + (V + 1) * 2
report(f"lemma vectors, {d} dims", lp, t1, t10, f"  — {size / 1e6:.1f} MB at a byte a number")


def quantized(x):
    s = np.abs(x).max(axis=1, keepdims=True) / 127 + 1e-12
    return (np.round(x / s) * s).astype(np.float32)


lp_q, t1, t10 = held_logp(quantized(U), quantized(W), b)
report(f"  … rounded to bytes", lp_q, t1, t10)

# Vectors for the rest, explicit pairs for the strong links: each pair seen
# gets a correction to the table's odds, r = log P_KN − log P_vec; the pairs
# that gain the most (count × r) are kept, the rest of the row renormalized.
codes, gains, rs, lvs = [], [], [], []
for i in range(0, len(rows), 1024):
    bi = np.sort(rows[i:i + 1024])
    lpb = logits(bi, U, W, b)
    cb = Cf[bi].tocoo()
    c = bi[cb.row]
    lv = lpb[cb.row, cb.col]
    n_c = row_n[c]
    kn = np.log(np.maximum(cb.data - D, 0) / n_c + D * row_types[c] / n_c * cont[cb.col])
    r = kn - lv
    codes.append(c * (V + 1) + cb.col)
    gains.append(cb.data * r)
    rs.append(r)
    lvs.append(lv)
codes, gains, rs, lvs = map(np.concatenate, (codes, gains, rs, lvs))
h_code = hc * (V + 1) + ht
for K in (100_000, 300_000, 1_000_000):
    keep = np.argsort(-gains)[:K]
    keep = keep[gains[keep] > 0]
    kc = codes[keep]
    o = np.argsort(kc)
    kc, kr, klv = kc[o], rs[keep][o], lvs[keep][o]
    z = np.ones(V + 2)
    np.add.at(z, kc // (V + 1), np.exp(klv) * (np.exp(kr) - 1))
    at = np.minimum(np.searchsorted(kc, h_code), len(kc) - 1)
    hit = kc[at] == h_code
    lp_h = lp + np.where(hit, kr[at], 0.0) - np.log(z[hc])
    report(f"  + {len(kc) // 1000}k explicit pairs", lp_h, extra=f"  — +{len(kc) * 4 / 1e6:.1f} MB at 4 bytes a pair")

# Neighbours, to see what the vectors hold.
Wn = W[:V] / (np.linalg.norm(W[:V], axis=1, keepdims=True) + 1e-9)
Un = U[:V] / (np.linalg.norm(U[:V], axis=1, keepdims=True) + 1e-9)
for w in ("кошка", "хлеб", "поезд", "врач", "любить", "красный", "быстро", "в", "охуенный"):
    if w in lemma_id:
        i = lemma_id[w]
        near = np.argsort(-(Wn @ Wn[i]))[1:9]
        nxt = np.argsort(-(U[i] @ W[:V].T + b[:V]))[:8]
        print(f"  {w}: like {' '.join(lemmas[j] for j in near)} | then {' '.join(lemmas[j] for j in nxt)}",
              flush=True)
if args.out:
    np.savez_compressed(args.out, U=U, W=W, b=b, lemmas=np.array(lemmas))
    log(f"wrote {args.out}")
