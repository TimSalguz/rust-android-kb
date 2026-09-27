#!/usr/bin/env python3
"""Government frames and agreement weights for the phrase grammar
(kbcore::gram), learned from running text.

Usage: tools/build_frames.py SOURCE... [--word-readings data/word_readings.tsv]
           [--readings data/readings.tsv] [--min 100] > data/frames.tsv

Every word of a phrase (punctuation ends one) is read against the words
before it the way the keyboard reads them: back over the adjectives before
it (and «и» between them) to the word that governs them — a preposition, a
verb, a noun — past adverbs and particles. For each governing form seen at
least `--min` times: how much likelier than anywhere the next word is a noun
phrase in each case, or something else (a preposition, a conjunction, an
adverb) — `ln(P(outcome | governor) / P(outcome))`; and, after adjectives,
how much likelier the next word agrees with them than not. Kept when it
says something (a log ratio of at least 0.3 somewhere): the frames are the
rules, the rest is the default.

SOURCE: Tatoeba `*_sentences.tsv.bz2` (held-out ids skipped, as in
build_bigrams.py) or Leipzig `*.tar.gz`.

stdout: `governor<TAB>other,nomn,gent,datv,accs,ablt,loct` (log ratios,
nats), and `@attr<TAB>other,agree,disagree,ln P(other)` for the words after
adjectives (the last: how often anything but a noun phrase comes anywhere,
to tell a frame's cases apart among noun phrases alone).
"""
import argparse
import bz2
import math
import re
import sys
import tarfile
from collections import Counter, defaultdict

HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
TOKEN = re.compile(r"[а-яё-]*[а-яё][а-яё-]*|\s+|.")
WORD = re.compile(r"[а-яё-]*[а-яё][а-яё-]*")
CASES = ["nomn", "gent", "datv", "accs", "ablt", "loct"]
FOLD = {"gen2": "gent", "acc2": "accs", "loc2": "loct", "voct": None}
ATTRIBUTE = {"ADJF", "PRTF"}
NOMINAL = {"NOUN", "ADJF", "PRTF", "NPRO", "NUMR"}
GENDERS = {"masc", "femn", "neut", "ms-f"}
ALPHA = 20.0  # smoothing toward the default, in cases
MIN_SAYS = 0.3

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--word-readings", default="data/word_readings.tsv")
ap.add_argument("--readings", default="data/readings.tsv")
ap.add_argument("--min", type=int, default=100)
args = ap.parse_args()


class Reading:
    __slots__ = ("pos", "case", "number", "gender")

    def __init__(self, tag):
        parts = tag.split(",")
        self.pos = parts[0]
        self.case = self.number = self.gender = None
        for g in parts[1:]:
            if g in CASES:
                self.case = g
            elif g in FOLD:
                self.case = FOLD[g]
            elif g in ("sing", "plur"):
                self.number = g
            elif g in GENDERS:
                self.gender = g


sets = {}
with open(args.readings, encoding="utf-8") as f:
    for line in f:
        i, tags = line.rstrip("\n").split("\t")
        sets[i] = tuple(Reading(t) for t in tags.split("|"))
readings = {}
with open(args.word_readings, encoding="utf-8") as f:
    for line in f:
        w, i = line.rstrip("\n").split("\t")
        readings[w] = sets[i]


def attributive(rs):
    return any(r.pos in ATTRIBUTE and r.case for r in rs)


def strict(rs):
    return bool(rs) and all(r.pos in ATTRIBUTE for r in rs)


def nominal(rs):
    return bool(rs) and all(r.pos in NOMINAL and r.case for r in rs)


def agree(a, b):
    if not a.case or a.case != b.case or not a.number or a.number != b.number:
        return False
    if a.number == "plur" or not a.gender or not b.gender:
        return True
    return a.gender == b.gender or "ms-f" in (a.gender, b.gender)


def walk(words, rs):
    """Back from the end: (governor index or None, the strict attributes)."""
    attrs = []
    collected = 0
    for k in range(len(words) - 1, max(-1, len(words) - 6), -1):
        w, r = words[k], rs[k]
        if w.startswith("котор"):
            return None, attrs
        if collected == len(words) - 1 - k and r and attributive(r):
            nxt = attrs[-1] if attrs else None
            as_attr = [x for x in r if x.pos in ATTRIBUTE]
            if nxt and not any(agree(x, y) for x in as_attr for y in nxt):
                return None, attrs
            if strict(r):
                attrs.append(r)
            collected += 1
            continue
        if collected and w in ("и", "или"):
            collected += 1
            continue
        if not collected and r and all(x.pos in ("ADVB", "PRCL") for x in r):
            collected += 1
            continue
        return k, attrs
    return None, attrs


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


# Outcomes: 0 something else, 1… a noun phrase in CASES[i - 1] (fractional
# over the cases the word may be in).
base = [0.0] * 7
gov = defaultdict(lambda: [0.0] * 7)
attr = Counter()  # other / agree / disagree after adjectives, and in general
seen = 0
for path in args.sources:
    for text in sentences(path):
        phrase_w, phrase_r = [], []
        for tok in TOKEN.findall(text.lower()):
            if not WORD.fullmatch(tok):
                if tok.strip():
                    phrase_w, phrase_r = [], []
                continue
            rs = readings.get(tok)
            if phrase_w and rs:
                g, attrs = walk(phrase_w, phrase_r)
                outcome = [0.0] * 7
                if nominal(rs):
                    fits = [r for r in rs if all(any(agree(a, r) for a in at) for at in attrs)]
                    cases = {r.case for r in (fits or rs)}
                    for c in cases:
                        outcome[1 + CASES.index(c)] = 1 / len(cases)
                    if attrs:
                        attr["agree" if fits else "disagree"] += 1
                else:
                    outcome[0] = 1.0
                    if attrs:
                        attr["other"] += 1
                if attrs:
                    attr["n"] += 1
                for i in range(7):
                    base[i] += outcome[i]
                if g is not None:
                    row = gov[phrase_w[g]]
                    for i in range(7):
                        row[i] += outcome[i]
                seen += 1
            phrase_w.append(tok)
            phrase_r.append(rs or ())
    print(f"{path}: {seen} words read", file=sys.stderr)

total = sum(base)
p_base = [b / total for b in base]
kept = 0
for g, row in sorted(gov.items()):
    n = sum(row)
    if n < args.min:
        continue
    lr = [math.log((row[i] + ALPHA * p_base[i]) / (n + ALPHA) / p_base[i]) for i in range(7)]
    if max(abs(x) for x in lr) < MIN_SAYS:
        continue
    print(g + "\t" + ",".join(f"{x:.3f}" for x in lr))
    kept += 1
# After adjectives: other words, agreeing ones and not, against their share
# anywhere (agreeing: any noun phrase; not: all but never, so a floor).
n = attr["n"] or 1
p_nominal = 1 - p_base[0]
print("@attr\t" + ",".join(f"{x:.3f}" for x in (
    math.log((attr["other"] + 1) / n / p_base[0]),
    math.log((attr["agree"] + 1) / n / p_nominal),
    math.log((attr["disagree"] + 1) / n / p_nominal),
    math.log(p_base[0]),
)))
print(f"{kept} governors of {len(gov)}; after adjectives {attr['n']} words: "
      f"{attr['other']} other, {attr['agree']} agree, {attr['disagree']} don't", file=sys.stderr)
