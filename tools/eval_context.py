#!/usr/bin/env python3
"""Context-model benchmark on held-out sentences (a Leipzig archive not used
for the bigrams): next-word prediction top-1/top-3, and typo correction
top-1 with vs without the previous word.

Usage: BIGRAMS_FST=bigrams.fst tools/eval_context.py dict.fst HELDOUT [--n 1000]
(HELDOUT: a Leipzig .tar.gz, or a Tatoeba *_sentences.tsv.bz2 — its held-out ids)
"""
import argparse
import bz2
import os
import random
import re
import subprocess
import tarfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("dict")
ap.add_argument("heldout")
ap.add_argument("--n", type=int, default=1000)
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--bin", default=f"{ROOT}/target/release/kbdemo")
args = ap.parse_args()
rng = random.Random(args.seed)

with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}

# Adjacent word pairs (only a single space between them) from the sentences.
def sentences():
    """A Leipzig archive's sentences, or the held-out part of a Tatoeba export
    (ids divisible by 50, never used by tools/build_bigrams.py)."""
    if args.heldout.endswith(".bz2"):
        with bz2.open(args.heldout, "rt", encoding="utf-8") as f:
            for line in f:
                parts = line.rstrip("\n").split("\t")
                if len(parts) == 3 and parts[0].isdigit() and int(parts[0]) % 50 == 0:
                    yield parts[2]
        return
    with tarfile.open(args.heldout) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(member):
            yield raw.decode("utf-8", "replace").split("\t", 1)[-1]


pairs = []
for text in sentences():
    for a, b in re.findall(r"(?<![\wё-])([а-яё]+) (?=([а-яё]+)(?![\wё-]))", text.lower()):
        if a in lexicon and b in lexicon and len(b) >= 2:
            pairs.append((a, b))
rng.shuffle(pairs)
pairs = pairs[: args.n]

# Typos: a neighbor-key substitution or a dropped letter (phone ЙЦУКЕН).
rows = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"]
pos = {c: (x + (1 if y == 2 else 0), y) for y, r in enumerate(rows) for x, c in enumerate(r)}
def neighbors(c):
    if c not in pos: return []
    x, y = pos[c]
    return [d for d, (dx, dy) in pos.items() if d != c and (dx - x) ** 2 + (dy - y) ** 2 <= 1.3]
def typo(w):
    for _ in range(20):
        i = rng.randrange(len(w))
        if rng.random() < 0.6 and neighbors(w[i]):
            t = w[:i] + rng.choice(neighbors(w[i])) + w[i + 1:]
        elif len(w) > 3:
            t = w[:i] + w[i + 1:]
        else:
            continue
        if t != w and t not in lexicon:
            return t
    return None

env = dict(os.environ, DICT_FST=args.dict, KB_PROFILE="phone")
# Next-word prediction.
pred = subprocess.run([args.bin, "--predict"] + [a for a, _ in pairs], capture_output=True, text=True, env=env).stdout.splitlines()
t1 = t3 = 0
for (a, b), line in zip(pairs, pred):
    words = [x.strip().split(" ")[0] for x in line.split("→", 1)[1].split(",") if x.strip()]
    t1 += words[:1] == [b]
    t3 += b in words[:3]
print(f"next word:  top-1 {100 * t1 / len(pairs):.1f}%  top-3 {100 * t3 / len(pairs):.1f}%  ({len(pairs)} pairs)")

# Typo correction with and without the previous word.
cases = [(a, b, typo(b)) for a, b in pairs]
cases = [c for c in cases if c[2]]
def top1(queries):
    out = subprocess.run([args.bin, "--query"] + queries, capture_output=True, text=True, env=env).stdout
    blocks = [bl for bl in re.split(r"\n\n", out.strip()) if bl]
    res = []
    for bl in blocks:
        m = re.findall(r"^\s+→\s(\S+)", bl, re.M)
        res.append(m[0] if m else None)
    return res
plain = top1([t for _, _, t in cases])
ctx = top1([f"{a}|{t}" for a, _, t in cases])
ok_p = sum(p == b for (_, b, _), p in zip(cases, plain))
ok_c = sum(c == b for (_, b, _), c in zip(cases, ctx))
print(f"correction: top-1 without context {100 * ok_p / len(cases):.1f}%  with context {100 * ok_c / len(cases):.1f}%  ({len(cases)} typos)")
