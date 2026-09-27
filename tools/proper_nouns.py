#!/usr/bin/env python3
"""Words the keyboard capitalizes by itself: `word<TAB>1` (Capitalized) or
`word<TAB>2` (ALL CAPS) — lowercase keys, only words of the lexicon.

Usage: tools/proper_nouns.py [--lexicon data/lexicon.tsv] [--scowl DIR] > data/proper.tsv

Russian: OpenCorpora forms (via pymorphy3) whose every reading is a name,
surname, patronymic, place, organisation or trademark — москва, ивана,
петербурге; not вера, роза, орёл, путина (also ordinary words). Proper
abbreviations go in capitals (сша, мгу). The е spelling of a ё form counts
too (королев).
English: SCOWL's capitalized lists up to level 70 (upper, proper names,
abbreviations) minus words that also exist in lowercase (brown, doctor,
congress stay as typed) and two-letter ones, plus «I» and its contractions.
"""
import argparse
import glob
import re
import sys

import pymorphy3

ap = argparse.ArgumentParser()
ap.add_argument("--lexicon", default="data/lexicon.tsv")
ap.add_argument("--scowl", default=None)
args = ap.parse_args()

with open(args.lexicon, encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}

CAPITAL, UPPER = 1, 2
casing = {}

# Russian.
PROPER = {"Name", "Surn", "Patr", "Geox", "Orgn", "Trad"}
proper, common, abbr = set(), set(), set()
for form, tag, _lemma, _para, _idx in pymorphy3.MorphAnalyzer().dictionary.iter_known_words():
    form = form.lower()
    grammemes = tag.grammemes
    if grammemes & PROPER:
        proper.add(form)
        if "Abbr" in grammemes:
            abbr.add(form)
    else:
        common.add(form)
# Compared without ё: «нее» is the everyday spelling of «неё», whatever river
# or name has a form «Нее».
common_e = {f.replace("ё", "е") for f in common}
for form in proper - common:
    how = UPPER if form in abbr else CAPITAL
    for w in {form, form.replace("ё", "е")}:
        if w in lexicon and w.replace("ё", "е") not in common_e:
            casing[w] = how

# English.
scowl = args.scowl or (sorted(glob.glob("data/scowl/scowl-*/final")) or [None])[-1]
if scowl:
    def levels(kind):
        return [p for p in glob.glob(f"{scowl}/*{kind}.*")
                if re.search(r"\.(\d+)$", p) and int(p.rsplit(".", 1)[1]) <= 70]

    def read(paths):
        out = set()
        for p in paths:
            with open(p, encoding="latin-1") as f:
                out.update(line.strip() for line in f if line.strip())
        return out

    lower = {w for w in read(levels("-words") + levels("-contractions")) if w == w.lower()}
    for w in read(levels("-upper") + levels("-proper-names") + levels("-abbreviations")):
        key = w.lower()
        # Two letters (Al, NA, OK) are too often something else in a chat.
        if (w == key or key in lower or key not in lexicon or len(key) <= 2
                or not re.fullmatch(r"[a-z'-]+", key)):
            continue
        if w == w.upper() and len(w) >= 2:
            casing.setdefault(key, UPPER)
        elif w[0].isupper() and w[1:] == w[1:].lower():
            casing.setdefault(key, CAPITAL)
for w in ("i", "i'm", "i'll", "i've", "i'd"):
    casing[w] = CAPITAL

out = sys.stdout
for w in sorted(casing):
    out.write(f"{w}\t{casing[w]}\n")
print(f"{len(casing)} words ({sum(v == UPPER for v in casing.values())} in capitals)", file=sys.stderr)
