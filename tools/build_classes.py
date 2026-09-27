#!/usr/bin/env python3
"""Grammar classes for the context model.

Usage: tools/build_classes.py SOURCE... [--lexicon data/lexicon.tsv]
            --classes data/classes.tsv --class-tags data/class_tags.tsv
            [--readings data/word_readings.tsv data/readings.tsv] > data/tag_pairs.tsv

Each Russian word of the lexicon that OpenCorpora knows (via pymorphy3) gets a
class: the set of its likely readings — part of speech, gender, number, case
(person, tense for verbs) — keeping readings with at least MIN_SCORE of the
probability (so «вод» is the genitive plural of «вода», not a rare masculine
noun). `--classes` gets `word<TAB>class id` (ids by frequency, from 1),
`--class-tags` `class id<TAB>tag id,tag id…` (at most 8 readings, each a tag:
POS with gender, number, case…, ids by frequency, from 1).

`--readings` keeps every reading of each word, however rare OpenCorpora
counts it («такой» is the feminine genitive most of the time, but also
masculine; «ним» 0.8% instrumental, «все» 0.8% adjective), for the phrase
grammar (kbcore::gram), which must not take a rare reading for a mistake:
`word<TAB>set id`, then `set id<TAB>tag|tag…` (ids by frequency, from 1).

Then, over running text (SOURCE: Tatoeba `*_sentences.tsv.bz2` — held-out ids
and stock-character sentences skipped, as in build_bigrams.py — or Leipzig
`*.tar.gz` sentences), which tag follows which: two words with only a space
between them, both classed, each reading counting 1/(its word's readings).
stdout gets `tag<TAB>tag<TAB>(PMI + 8)×1000`, PMI shrunk toward 0 for rare
pairs. The keyboard takes the best-fitting pair of readings (a word fits if
any of its readings does — «полью» is also a rare noun, which mustn't count
against the verb): «пользовательский ввод» — an adjective, masculine singular
nominative, then a masculine singular noun — fits; «пользовательский вод»
(genitive plural) does not, though neither pair of words was ever seen.
"""
import argparse
import bz2
import math
import re
import sys
import tarfile
from collections import Counter

import pymorphy3

MIN_SCORE = 0.15
MIN_COUNT = 3
HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
TOKEN = re.compile(r"[а-яё-]*[а-яё][а-яё-]*|\s+|.")
WORD = re.compile(r"[а-яё-]*[а-яё][а-яё-]*")

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--lexicon", default="data/lexicon.tsv")
ap.add_argument("--classes", required=True)
ap.add_argument("--class-tags", required=True)
ap.add_argument("--readings", nargs=2, metavar=("WORD_SETS", "SETS"),
                help="every reading of each word, for the phrase grammar")
args = ap.parse_args()

morph = pymorphy3.MorphAnalyzer()


def coarse(tag):
    parts = [tag.POS or "X"]
    for g in ("gender", "number", "case", "person", "tense"):
        v = getattr(tag, g)
        if v:
            parts.append(v)
    return ",".join(parts)


MAX_READINGS = 8


def class_key(word):
    """The word's likely readings (its class), and all of them."""
    parses = morph.parse(word)
    if not parses or not morph.word_is_known(word):
        return None, None
    ranked = sorted(parses, key=lambda p: -p.score)
    tags, full = [], []
    for p in ranked:
        t = coarse(p.tag)
        if (p.score >= MIN_SCORE or not tags) and t not in tags:
            tags.append(t)
        if t not in full:
            full.append(t)
    tags = sorted(tags[:MAX_READINGS])
    return ("|".join(tags) if tags else None), "|".join(sorted(full))


# Classes of the lexicon's Russian words.
word_class, word_full = {}, {}
sizes = Counter()
with open(args.lexicon, encoding="utf-8") as f:
    for line in f:
        w = line.split("\t", 1)[0]
        if not WORD.fullmatch(w):
            continue
        k, full = class_key(w)
        if k:
            word_class[w] = k
            word_full[w] = full
            sizes[k] += 1
ids = {k: i + 1 for i, (k, _) in enumerate(sizes.most_common())}
with open(args.classes, "w", encoding="utf-8") as f:
    for w in sorted(word_class):
        f.write(f"{w}\t{ids[word_class[w]]}\n")
# Tags (single readings), ids by how many words have them.
tag_words = Counter()
for k, n in sizes.items():
    for t in k.split("|"):
        tag_words[t] += n
# A byte per tag: the 254 commonest keep their own id, the rarest share 255.
tag_id = {t: min(i + 1, 255) for i, (t, _) in enumerate(tag_words.most_common())}
class_tags = {ids[k]: sorted({tag_id[t] for t in k.split("|")}) for k in ids}
with open(args.class_tags, "w", encoding="utf-8") as f:
    for c in sorted(class_tags):
        f.write(f"{c}\t{','.join(map(str, class_tags[c]))}\n")
if args.readings:
    full_ids = {k: i + 1 for i, (k, _) in enumerate(Counter(word_full.values()).most_common())}
    with open(args.readings[0], "w", encoding="utf-8") as f:
        for w in sorted(word_full):
            f.write(f"{w}\t{full_ids[word_full[w]]}\n")
    with open(args.readings[1], "w", encoding="utf-8") as f:
        for k, i in sorted(full_ids.items(), key=lambda x: x[1]):
            f.write(f"{i}\t{k}\n")
print(f"{len(word_class)} words in {len(ids)} classes over {len(tag_id)} tags", file=sys.stderr)


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


uni, pairs = Counter(), Counter()  # by tag, fractional
for path in args.sources:
    n = 0
    for text in sentences(path):
        prev = None
        for tok in TOKEN.findall(text.lower()):
            if WORD.fullmatch(tok):
                k = word_class.get(tok)
                c = class_tags[ids[k]] if k else None
                if c:
                    for t in c:
                        uni[t] += 1 / len(c)
                    if prev:
                        w = 1 / (len(prev) * len(c))
                        for a in prev:
                            for b in c:
                                pairs[(a, b)] += w
                    n += 1
                prev = c
            elif tok != " ":
                prev = None
    print(f"{path}: {n} classed words", file=sys.stderr)

total = sum(uni.values()) or 1
left = Counter()
for (a, b), c in pairs.items():
    left[a] += c
kept = 0
out = sys.stdout
for (a, b), c in sorted(pairs.items()):
    if c < 1.0:
        continue
    pmi = math.log(c / left[a]) - math.log(uni[b] / total)
    pmi *= c / (c + 5.0)
    out.write(f"{a}\t{b}\t{round((max(-8.0, min(8.0, pmi)) + 8.0) * 1000)}\n")
    kept += 1
print(f"{kept} tag pairs", file=sys.stderr)
