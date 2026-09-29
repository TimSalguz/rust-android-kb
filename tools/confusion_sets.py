#!/usr/bin/env python3
"""Confusion sets for the context rules (docs/rules-format.md, section 1).

Usage: tools/confusion_sets.py [--lexicon data/lexicon.tsv] [--top 80000]
           [--rows qwertzuiopü asdfghjklöä yxcvbnm] > data/confusions.tsv

Frequent words that one slip turns into each other on the phone keyboard — a
letter replaced by a neighboring key (из/их, а/в, не/ее) — or that draw the
same gesture path (a doubled letter: ввод/вод; н/нн: раненый/раненный), or
that differ in an accent only, which people often leave out (el/él,
esta/está, ou/où); then spelling pairs (к/ко, с/со…, -тся/-ться, не/ни). One
set per such pair. `--rows` gives the layout of a language pack (its rows, centered);
by default ЙЦУКЕН and QWERTY.
"""
import unicodedata
import argparse
import re
import sys
from collections import defaultdict

ap = argparse.ArgumentParser()
ap.add_argument("--lexicon", default="data/lexicon.tsv")
ap.add_argument("--top", type=int, default=80000,
                help="words; with both scripts in the lexicon, as many Cyrillic and half as many Latin")
ap.add_argument("--rows", nargs="+", help="the layout's letter rows, top to bottom")
args = ap.parse_args()

# Phone layouts: rows with their horizontal offsets (in keys).
LAYOUTS = [
    (["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"], [0.0, 0.0, 1.0]),
    (["qwertyuiop", "asdfghjkl", "zxcvbnm"], [0.0, 0.5, 1.5]),
]
if args.rows:
    widest = max(len(r) for r in args.rows)
    LAYOUTS = [(args.rows, [(widest - len(r)) / 2 for r in args.rows])]


def neighbors():
    near = defaultdict(set)
    for rows, offs in LAYOUTS:
        pos = {c: (x + offs[r], r) for r, row in enumerate(rows) for x, c in enumerate(row)}
        for a, (ax, ay) in pos.items():
            for b, (bx, by) in pos.items():
                if a != b and abs(ay - by) <= 1 and abs(ax - bx) <= 1.0 + (0.01 if ay == by else 0.6):
                    near[a].add(b)
    return near


NEAR = neighbors()
# Letters of kbcore::alphabet::CHARSET.
WORD = re.compile(r"[a-zß-öø-ÿœа-яё'-]+")


def bare(ch):
    """The letter without its accent (é → e), for Latin letters; ё stays
    (the lexicon spells every ё word with е too)."""
    if ch in "œæß" or ch >= "\u0400":
        return ch
    return unicodedata.normalize("NFD", ch)[0]

words = []
with open(args.lexicon, encoding="utf-8") as f:
    for line in f:
        w, _, c = line.rstrip("\n").partition("\t")
        if WORD.fullmatch(w):
            words.append((int(c or 0), w))
# The top words of each language (Latin and Cyrillic counts differ in scale).
words.sort(reverse=True)
cyr = [w for _, w in words if re.search("[а-яё]", w)][: args.top]
lat = [w for _, w in words if not re.search("[а-яё]", w)][: args.top // 2 if cyr else args.top]
top = cyr + lat
rank = {w: i for i, w in enumerate(top)}
known = set(top)

edges = defaultdict(set)
for w in top:
    for i, ch in enumerate(w):
        for n in NEAR.get(ch, ()):  # one neighbor-key slip
            v = w[:i] + n + w[i + 1 :]
            if v in known:
                edges[w].add(v)
        b = bare(ch)
        if b != ch:  # the accent left out
            v = w[:i] + b + w[i + 1 :]
            if v in known:
                edges[w].add(v)
                edges[v].add(w)
        if i + 1 < len(w) and w[i + 1] == ch:  # a doubled letter drawn once
            v = w[:i] + w[i + 1 :]
            if v in known:
                edges[w].add(v)
                edges[v].add(w)

# One set per pair: a word may be confused with several others (в: а, я, ы),
# each pair gets its own rules. Frequent pairs first.
pairs = sorted(
    {tuple(sorted((w, v), key=rank.get)) for w in top for v in edges[w]},
    key=lambda p: (rank[p[0]] + rank[p[1]], p),
)
for a, b in pairs:
    print(f"{a}\t{b}")
print(f"{len(pairs)} pairs over the top {len(top)} words", file=sys.stderr)

# Spelling pairs, after the slips (their ids stay as they were): a
# preposition and its form with о before some consonant clusters (к тебе, ко
# мне — the next word decides), -тся / -ться (он учится, хочет учиться), не /
# ни (ни разу, не раз).
PREPOSITIONS = [("в", "во"), ("с", "со"), ("к", "ко"), ("о", "об"), ("об", "обо"), ("о", "обо"),
                ("из", "изо"), ("от", "ото"), ("над", "надо"), ("под", "подо"),
                ("перед", "передо"), ("без", "безо"), ("не", "ни")]
seen = set(pairs)
spelling = []
for a, b in PREPOSITIONS:
    if a in known and b in known:
        spelling.append((a, b))
for w in top:
    if w.endswith("тся") and re.search("[а-яё]", w):
        v = w[:-3] + "ться"
        if v in known:
            spelling.append(tuple(sorted((w, v), key=rank.get)))
spelling = [p for p in dict.fromkeys(spelling) if p not in seen and (p[1], p[0]) not in seen]
for a, b in spelling:
    print(f"{a}\t{b}")
print(f"{len(spelling)} spelling pairs", file=sys.stderr)
