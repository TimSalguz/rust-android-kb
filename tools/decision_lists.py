#!/usr/bin/env python3
"""Baseline context rules (docs/rules-format.md, section 3): decision lists
over what the keyboard's language model can't see — the next word (`w+1`)
and its grammar class (`c+1`).

Usage: bzcat data/observations.tsv.bz2 | tools/decision_lists.py \\
           [--confusions data/confusions.tsv] > data/rules.tsv

For each confusion pair and feature: the log likelihood ratio
`ln P(feature | word) / P(feature | other)` (add-½ smoothing) — evidence to
add to the keyboard's own odds, which already weigh how frequent each word
is. A rule is kept when its feature was seen MIN_COUNT times with the pair,
MIN_FAVORED times with the word it favors (a rare word — año/ano — must not
win on two or three sightings), and the ratio is at least MIN_LLR; a pair
keeps its MAX_RULES strongest.
"""
import argparse
import math
import sys
from collections import defaultdict

MIN_COUNT = 10
MIN_FAVORED = 5
MIN_LLR = 1.0
MAX_RULES = 20
CLAMP = 6.0
USE = ("w+1=", "c+1=")

ap = argparse.ArgumentParser()
ap.add_argument("--confusions", default="data/confusions.tsv")
args = ap.parse_args()

members = []
with open(args.confusions, encoding="utf-8") as f:
    for line in f:
        members.append(line.rstrip("\n").split("\t"))

total = defaultdict(lambda: defaultdict(int))  # set → word → records
feat = defaultdict(lambda: defaultdict(lambda: defaultdict(int)))  # set → feature → word → n
for line in sys.stdin:
    parts = line.rstrip("\n").split("\t")
    if len(parts) < 2:
        continue
    sid, truth = int(parts[0]), parts[1]
    total[sid][truth] += 1
    for f in parts[2:]:
        if f.startswith(USE):
            feat[sid][f][truth] += 1

rules = 0
out = sys.stdout
for sid in sorted(feat):
    ws = members[sid]
    if len(ws) != 2 or any(total[sid][w] == 0 for w in ws):
        continue
    a, b = ws
    na, nb = total[sid][a], total[sid][b]
    kept = []
    for f, by in feat[sid].items():
        ca, cb = by.get(a, 0), by.get(b, 0)
        if ca + cb < MIN_COUNT:
            continue
        llr = math.log((ca + 0.5) / (na + 1.0)) - math.log((cb + 0.5) / (nb + 1.0))
        favored = ca if llr > 0 else cb
        if abs(llr) >= MIN_LLR and favored >= MIN_FAVORED:
            kept.append((abs(llr), f, a if llr > 0 else b))
    kept.sort(reverse=True)
    for strength, f, w in kept[:MAX_RULES]:
        out.write(f"{sid}\t{f}\t{w}\t{min(strength, CLAMP):.3f}\n")
        rules += 1
print(f"{rules} rules for {len(feat)} pairs", file=sys.stderr)
