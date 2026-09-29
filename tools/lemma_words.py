#!/usr/bin/env python3
"""The lemma of every Russian word of the dictionary, read as
tools/lemma_corpus.py reads the corpus (pymorphy3's likeliest reading of the
word spelled with its ё).

Usage: tools/lemma_words.py [--words data/word_readings.tsv] [--yo data/yo.tsv]
                            > data/lemma/words.tsv

stdout: `word<TAB>lemma`.
"""
import argparse
import multiprocessing
import os
import sys

import pymorphy3

ap = argparse.ArgumentParser()
ap.add_argument("--words", default="data/word_readings.tsv")
ap.add_argument("--yo", default="data/yo.tsv")
args = ap.parse_args()

yo = {}
if os.path.exists(args.yo):
    with open(args.yo, encoding="utf-8") as f:
        yo = dict(line.rstrip("\n").split("\t") for line in f)
with open(args.words, encoding="utf-8") as f:
    words = [line.split("\t", 1)[0].rstrip("\n") for line in f]


def lemmas_of(chunk):
    m = pymorphy3.MorphAnalyzer()
    out = []
    for w in chunk:
        p = m.parse(yo.get(w, w))
        out.append(f"{w}\t{p[0].normal_form if p else w}\n")
    return "".join(out)


chunks = [words[i:i + 20000] for i in range(0, len(words), 20000)]
with multiprocessing.get_context("fork").Pool(4) as pool:
    for part in pool.imap(lemmas_of, chunks):
        sys.stdout.write(part)
print(f"{len(words)} words", file=sys.stderr)
