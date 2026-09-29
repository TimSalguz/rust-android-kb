#!/usr/bin/env python3
"""Words written without their ё: `еще` for `ещё`, `пошел` for `пошёл` — a form
with е that is only a spelling of the form with ё, never a word of its own.

Usage: tools/yo_words.py [--lexicon data/lexicon.tsv] > data/yo.tsv

For each lexicon word with ё whose е-spelling is in the lexicon too: kept
when OpenCorpora (pymorphy3) knows the е-form only as the ё-word — every
parse of it is a ё-word (`еще` → ещё). Left out where the е-form is a word
itself: небе (небо) / нёбе (нёбо), все / всё, осел / осёл, узнаем / узнаём —
there the context has to decide. stdout: `е-form<TAB>ё-form`.
"""
import argparse
import sys

import pymorphy3

ap = argparse.ArgumentParser()
ap.add_argument("--lexicon", default="data/lexicon.tsv")
args = ap.parse_args()

words = set()
with open(args.lexicon, encoding="utf-8") as f:
    for line in f:
        words.add(line.split("\t", 1)[0])
morph = pymorphy3.MorphAnalyzer()
kept = left = 0
for w in sorted(words):
    if "ё" not in w:
        continue
    e = w.replace("ё", "е")
    if e not in words:
        continue
    parses = morph.parse(e)
    if parses and all(p.word != e and p.word.replace("ё", "е") == e for p in parses):
        print(f"{e}\t{w}")
        kept += 1
    else:
        left += 1
print(f"{kept} е-spellings of ё-words; {left} left to the context (the е-form a word too)",
      file=sys.stderr)
