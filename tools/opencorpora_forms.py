#!/usr/bin/env python3
"""Dump every word form of the OpenCorpora dictionary as `form<TAB>lemma`.

Usage: tools/opencorpora_forms.py > data/opencorpora_forms.tsv
Needs `pip install pymorphy3 pymorphy3-dicts-ru` (the OpenCorpora dictionary,
CC BY-SA, packaged for pymorphy). Forms outside the keyboard alphabet
(digits, Latin) are skipped.
"""
import re
import sys

import pymorphy3

WORD = re.compile(r"[а-яё'-]*[а-яё][а-яё'-]*")

seen = set()
out = sys.stdout
for form, _tag, lemma, _para, _idx in pymorphy3.MorphAnalyzer().dictionary.iter_known_words():
    form = form.lower()
    if form in seen or not WORD.fullmatch(form):
        continue
    seen.add(form)
    out.write(f"{form}\t{lemma.lower()}\n")
print(f"{len(seen)} forms", file=sys.stderr)
