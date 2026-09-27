#!/usr/bin/env python3
"""Observations for the context rules (docs/rules-format.md, section 2).

Usage: tools/export_observations.py SOURCE... [--confusions data/confusions.tsv]
            [--classes data/classes.tsv] [--cap 20000] | bzip2 > data/observations.tsv.bz2

For every occurrence, in running text, of a word that belongs to a confusion
pair: `set_id<TAB>truth<TAB>feature…`, one record per pair the word is in
(capped at `--cap` records per pair and word, the first ones met). SOURCE:
Tatoeba `*_sentences.tsv.bz2` (held-out ids and stock-character sentences
skipped) or Leipzig `*.tar.gz` sentences.

Features (docs/rules-format.md): w±1, w-2, t±1 (grammemes of the neighbor's likely
readings, OpenCorpora via pymorphy3), c±1 (its grammar class id from
data/classes.tsv, as the keyboard stores it), bos, eos, p-1.
"""
import argparse
import bz2
import re
import sys
import tarfile
from collections import Counter, defaultdict

import os

try:
    import pymorphy3
except ImportError:  # only the Russian tags need it
    pymorphy3 = None

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--confusions", default="data/confusions.tsv")
ap.add_argument("--classes", default="data/classes.tsv")
ap.add_argument("--cap", type=int, default=20000)
args = ap.parse_args()

HOLDOUT = 50
MIN_SCORE = 0.15
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*|Tom|Tom's|Tomás|Mary|Mary's|Maria|Marias|María|Marie|Boston)\b")
# Letters of kbcore::alphabet::CHARSET.
TOKEN = re.compile(r"[a-zß-öø-ÿœа-яё'-]*[a-zß-öø-ÿœа-яё][a-zß-öø-ÿœа-яё'-]*|[^\s]")

pairs_of = defaultdict(list)  # word → [(set id, other)]
with open(args.confusions, encoding="utf-8") as f:
    for i, line in enumerate(f):
        ws = line.rstrip("\n").split("\t")
        for w in ws:
            pairs_of[w].append(i)
# Grammar classes and tags are Russian only (OpenCorpora): without the
# classes file (other languages) only the word features are exported.
word_class = {}
if os.path.exists(args.classes):
    with open(args.classes, encoding="utf-8") as f:
        for line in f:
            w, _, c = line.rstrip("\n").partition("\t")
            word_class[w] = c

morph = pymorphy3.MorphAnalyzer() if word_class else None
tag_cache = {}


def tags(word):
    """Grammemes of the word's likely readings (Russian words only)."""
    if word not in tag_cache:
        out = set()
        if morph and re.search("[а-яё]", word) and morph.word_is_known(word):
            for p in morph.parse(word):
                if p.score >= MIN_SCORE:
                    t = p.tag
                    out.update(x for x in (t.POS, t.gender, t.number, t.case, t.person, t.tense) if x)
        tag_cache[word] = sorted(out)
    return tag_cache[word]


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


def is_word(t):
    return t[0].isalpha() or t[0] in "'-"


seen = Counter()
out = sys.stdout
written = 0
for path in args.sources:
    for text in sentences(path):
        toks = TOKEN.findall(text.lower())
        for i, tok in enumerate(toks):
            if not is_word(tok) or tok not in pairs_of:
                continue
            todo = [sid for sid in pairs_of[tok] if seen[(sid, tok)] < args.cap]
            if not todo:
                continue
            feats = []
            prev = toks[i - 1] if i > 0 else None
            nxt = toks[i + 1] if i + 1 < len(toks) else None
            if prev is None:
                feats.append("bos")
            elif not is_word(prev):
                feats.append(f"p-1={prev}")
            else:
                feats.append(f"w-1={prev}")
                feats += [f"t-1={t}" for t in tags(prev)]
                if prev in word_class:
                    feats.append(f"c-1={word_class[prev]}")
                if i > 1 and is_word(toks[i - 2]):
                    feats.append(f"w-2={toks[i - 2]}")
            if nxt is None or nxt in ".!?":
                feats.append("eos")
            elif is_word(nxt):
                feats.append(f"w+1={nxt}")
                feats += [f"t+1={t}" for t in tags(nxt)]
                if nxt in word_class:
                    feats.append(f"c+1={word_class[nxt]}")
            line = "\t".join(feats)
            for sid in todo:
                seen[(sid, tok)] += 1
                out.write(f"{sid}\t{tok}\t{line}\n")
                written += 1
    print(f"{path}: {written} records so far", file=sys.stderr)
