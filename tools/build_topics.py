#!/usr/bin/env python3
"""Sense classes: which words go together in a sentence, compressed into
classes of lemmas — so the keyboard can tell «кошка» from «крошка» in a
sentence about pets, with a byte per class pair instead of a table of words.

Usage: tools/build_topics.py SOURCE... [--word-readings data/word_readings.tsv]
           [--vocab 20000] [--contexts 2000] [--classes 512]
           --words data/topic_words.tsv > data/topic_pairs.tsv

1. The Russian words of the sentences (Tatoeba `*_sentences.tsv.bz2`,
   held-out ids skipped; Leipzig `*.tar.gz`) are taken to their lemmas
   (pymorphy3's likeliest reading); nouns, verbs, adjectives and adverbs
   count, the rest (prepositions, pronouns…) don't say what a text is about.
2. For the `--vocab` commonest lemmas: how often each comes in a sentence
   with each of the `--contexts` commonest (positive PMI), reduced by a
   randomized SVD to 64 numbers per lemma.
3. Those grouped into `--classes` classes (spherical k-means): «кошка»,
   «собака», «котёнок» together; «хлеб», «суп», «сыр» together.
4. How much likelier two classes share a sentence than by chance (PMI,
   shrunk toward 0 when rarely seen): stdout `class<TAB>class<TAB>pmi`, the
   pairs worth a byte (|PMI| ≥ 0.3, seen ≥ 20 times).

`--words`: `word<TAB>class` for every word of `--word-readings` whose lemma
has a class (all its forms share it).

Needs numpy.
"""
import argparse
import bz2
import math
import multiprocessing
import re
import sys
import tarfile
from collections import Counter

import numpy as np
import pymorphy3

HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
WORD = re.compile(r"[а-яё]+(?:-[а-яё]+)*")
CONTENT = {"NOUN", "VERB", "INFN", "ADJF", "ADJS", "ADVB", "PRTF", "PRTS", "GRND", "COMP"}
SHRINK = 20.0
MIN_PAIR = 20
MIN_SAYS = 0.3

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--word-readings", default="data/word_readings.tsv")
ap.add_argument("--vocab", type=int, default=20000)
ap.add_argument("--contexts", type=int, default=2000)
ap.add_argument("--classes", type=int, default=512)
ap.add_argument("--dims", type=int, default=64)
ap.add_argument("--words", required=True)
ap.add_argument("--seed", type=int, default=1)
args = ap.parse_args()
rng = np.random.default_rng(args.seed)
morph = pymorphy3.MorphAnalyzer()
lemma_of = {}


def lemma(w):
    """The lemma of a content word (None for the rest), cached."""
    if w not in lemma_of:
        p = morph.parse(w)
        best = p[0] if p else None
        lemma_of[w] = best.normal_form if best and best.tag.POS in CONTENT else None
    return lemma_of[w]


def sentences(path):
    if path.endswith(".bz2"):
        with bz2.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) == 3 and p[0].isdigit() and int(p[0]) % HOLDOUT and not STOCK.search(p[2]):
                    yield p[2]
        return
    with tarfile.open(path) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(member):
            yield raw.decode("utf-8", "replace").split("\t", 1)[-1]


# Each sentence as its set of content lemmas.
bags = []
freq = Counter()
for path in args.sources:
    for text in sentences(path):
        bag = {lemma(w) for w in WORD.findall(text.lower())} - {None}
        if len(bag) >= 2:
            bags.append(bag)
            freq.update(bag)
    print(f"{path}: {len(bags)} sentences, {len(lemma_of)} words seen", file=sys.stderr)

vocab = [w for w, _ in freq.most_common(args.vocab)]
index = {w: i for i, w in enumerate(vocab)}
ctx = {w: i for i, w in enumerate(vocab[: args.contexts])}
counts = np.zeros((len(vocab), len(ctx)), dtype=np.float32)
for bag in bags:
    rows = [index[w] for w in bag if w in index]
    cols = [ctx[w] for w in bag if w in ctx]
    if rows and cols:
        counts[np.ix_(rows, cols)] += 1.0
np.fill_diagonal(counts[: len(ctx)], 0.0)
total = counts.sum()
row = counts.sum(axis=1, keepdims=True) + 1e-9
col = counts.sum(axis=0, keepdims=True) + 1e-9
ppmi = np.maximum(np.log(counts * total / (row * col) + 1e-12), 0.0).astype(np.float32)
del counts
# Randomized SVD: the top directions of the PPMI matrix.
k = args.dims + 16
omega = rng.standard_normal((ppmi.shape[1], k)).astype(np.float32)
y = ppmi @ omega
y = ppmi @ (ppmi.T @ y)
q, _ = np.linalg.qr(y)
u_b, s, _ = np.linalg.svd(q.T @ ppmi, full_matrices=False)
emb = (q @ u_b)[:, : args.dims] * np.sqrt(s[: args.dims])
emb /= np.linalg.norm(emb, axis=1, keepdims=True) + 1e-9
# Spherical k-means, seeded by k-means++ on a sample.
kk = args.classes
centers = [emb[rng.integers(len(emb))]]
sample = emb[rng.choice(len(emb), min(len(emb), 8000), replace=False)]
for _ in range(kk - 1):
    d = 1.0 - np.max(sample @ np.array(centers).T, axis=1)
    p = np.maximum(d, 0) ** 2
    centers.append(sample[rng.choice(len(sample), p=p / p.sum())])
centers = np.array(centers)
for it in range(30):
    assign = np.argmax(emb @ centers.T, axis=1)
    for c in range(kk):
        members = emb[assign == c]
        if len(members):
            m = members.sum(axis=0)
            centers[c] = m / (np.linalg.norm(m) + 1e-9)
cls = {w: int(assign[i]) + 1 for i, w in enumerate(vocab)}
sizes = Counter(cls.values())
print(f"{len(vocab)} lemmas in {len(sizes)} classes (largest {max(sizes.values())})", file=sys.stderr)
for c in list(sizes)[:0]:
    pass

# How much likelier two classes share a sentence than by chance.
single, pair = Counter(), Counter()
n = 0
for bag in bags:
    cs = sorted({cls[w] for w in bag if w in cls})
    if len(cs) < 2:
        continue
    n += 1
    single.update(cs)
    for i, a in enumerate(cs):
        for b in cs[i + 1:]:
            pair[(a, b)] += 1
kept = 0
for (a, b), c in sorted(pair.items()):
    if c < MIN_PAIR:
        continue
    pmi = math.log(c * n / (single[a] * single[b])) * c / (c + SHRINK)
    if abs(pmi) >= MIN_SAYS:
        print(f"{a}\t{b}\t{pmi:.3f}")
        kept += 1
print(f"{kept} class pairs of {len(pair)}", file=sys.stderr)

# Every word of the dictionary whose lemma has a class.
words = [l.split("\t", 1)[0] for l in open(args.word_readings, encoding="utf-8")]


def lemmas_of(chunk):
    m = pymorphy3.MorphAnalyzer()
    out = []
    for w in chunk:
        p = m.parse(w)
        out.append((w, p[0].normal_form if p else w))
    return out


chunks = [words[i:i + 20000] for i in range(0, len(words), 20000)]
labelled = 0
with open(args.words, "w", encoding="utf-8") as f, multiprocessing.get_context("fork").Pool(4) as pool:
    for part in pool.imap(lemmas_of, chunks):
        for w, l in part:
            if l in cls:
                f.write(f"{w}\t{cls[l]}\n")
                labelled += 1
print(f"{labelled} words with a class", file=sys.stderr)
# A few classes, to see what they hold.
by = {}
for w, c in cls.items():
    by.setdefault(c, []).append(w)
for w in ("кошка", "хлеб", "поезд", "врач", "любить"):
    if w in cls:
        print(f"  {w}: {' '.join(by[cls[w]][:15])}", file=sys.stderr)
