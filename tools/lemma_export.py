#!/usr/bin/env python3
"""The lemma vectors (tools/lemma_vectors.py) for the keyboard.

Usage: tools/lemma_export.py data/lemma/vectors16.npz [--dir data/lemma]
                             --bin lemmas.bin --ids data/lemma/ids.tsv

`--ids`: `word<TAB>lemma id` for every dictionary word (data/lemma/words.tsv)
whose lemma has vectors — for `index-builder --lemmas` (context model keys
`0, 25, word`). A Russian word left out is UNK (a lemma too rare for its own
vectors); the ids go by the lemmas' alphabetical order.

`--bin` (little-endian; kbcore::lemmas reads it):

    b"KBLM", u32 version 1, u32 V (lemmas; UNK is V), u32 d
    context rows (V + 2: the lemmas, UNK, START — a sentence's first word):
        i8 [V+2][d] vectors, f32 [V+2] scales,
        f32 [V+2] log Z (so the V+1 odds after it sum to 1),
        f32 [V+2] the most PMI any lemma gets after it
    target rows (V + 1):
        i8 [V+1][d] vectors, f32 [V+1] scales,
        f32 [V+1] bias less log P(lemma), f32 [V+1] log P(lemma)
    u8 [V][8] each lemma's stem: the letters (kbcore::alphabet ids, 0 after
        the end) its forms share with it — where its forms lie in the
        context model's `0, 25` keys, for the words a lemma predicts

so that PMI(c, l) = log P(l | c) − log P(l) = s_c·s_l·(u_c · v_l) + bias_l −
log Z_c: the keyboard's P(form | previous) = P(form)·e^PMI(lemma of the
previous word, lemma of the form) — the form's own odds (the dictionary's
prior) moved by what the lemmas say.
"""
import argparse
import struct

import numpy as np

ap = argparse.ArgumentParser()
ap.add_argument("vectors")
ap.add_argument("--dir", default="data/lemma")
ap.add_argument("--bin", required=True)
ap.add_argument("--ids", required=True)
args = ap.parse_args()

z = np.load(args.vectors)
U, W, b, lemmas = z["U"], z["W"], z["b"], [str(x) for x in z["lemmas"]]
V, d = len(lemmas), U.shape[1]
# Alphabetical ids: a node of the dictionary covers a run of them.
order = sorted(range(V), key=lambda i: lemmas[i])
new = np.empty(V, dtype=np.int64)
new[order] = np.arange(V)
ctx_perm = np.concatenate([order, [V, V + 1]])
tgt_perm = np.concatenate([order, [V]])
U, W, b = U[ctx_perm], W[tgt_perm], b[tgt_perm]
lemmas = [lemmas[i] for i in order]
lemma_id = {l: i for i, l in enumerate(lemmas)}

# The lemmas' own odds, as the training counted them (UNK: the rest).
count = np.zeros(V + 1)
with open(f"{args.dir}/forms.tsv", encoding="utf-8") as f:
    for line in f:
        _, n, lemma, _ = line.rstrip("\n").split("\t")
        count[lemma_id.get(lemma, V)] += int(n)
log_p = np.log((count + 0.5) / (count + 0.5).sum())


def quantize(x):
    s = np.abs(x).max(axis=1) / 127 + 1e-12
    return np.round(x / s[:, None]).astype(np.int8), s.astype(np.float32)


qu, su = quantize(U)
qw, sw = quantize(W)
Wq = qw.astype(np.float32) * sw[:, None]
bias = (b - log_p).astype(np.float32)
logz = np.empty(V + 2, dtype=np.float32)
most = np.empty(V + 2, dtype=np.float32)
for i in range(0, V + 2, 2048):
    u = qu[i:i + 2048].astype(np.float32) * su[i:i + 2048, None]
    s = u @ Wq.T + b
    m = s.max(axis=1, keepdims=True)
    lz = (m + np.log(np.exp(s - m).sum(axis=1, keepdims=True))).ravel()
    logz[i:i + 2048] = lz
    most[i:i + 2048] = (u @ Wq.T + bias).max(axis=1) - lz

# Each dictionary word with its lemma id; each lemma's stem: the prefix it
# shares with its forms (those sharing two letters or more with it — not
# «людей» for «человек»), at most 8 letters.
stem = {}
kept = 0
with open(f"{args.dir}/words.tsv", encoding="utf-8") as f, open(args.ids, "w", encoding="utf-8") as out:
    for line in f:
        word, lemma = line.rstrip("\n").split("\t")
        if lemma not in lemma_id:
            continue
        out.write(f"{word}\t{lemma_id[lemma]}\n")
        kept += 1
        n = 0
        while n < min(len(word), len(lemma)) and word[n] == lemma[n]:
            n += 1
        if n >= 2:
            stem[lemma] = min(stem.get(lemma, len(lemma)), n)


def letter_id(c):
    """kbcore::alphabet::char_to_id for the letters of a Russian lemma."""
    if "а" <= c <= "я":
        return 62 + ord(c) - ord("а")
    return {"ё": 94, "-": 2, "'": 1}[c]


stems = np.zeros((V, 8), dtype=np.uint8)
for l, i in lemma_id.items():
    for j, c in enumerate(l[: min(stem.get(l, len(l)), 8)]):
        stems[i, j] = letter_id(c)
with open(args.bin, "wb") as f:
    f.write(b"KBLM" + struct.pack("<III", 1, V, d))
    for a in (qu, su, logz, most, qw, sw, bias, log_p.astype(np.float32), stems):
        f.write(np.ascontiguousarray(a).tobytes())
print(f"{V} lemmas × {d}: {args.bin}; {kept} words with a lemma id")
