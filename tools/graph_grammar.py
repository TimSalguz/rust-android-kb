#!/usr/bin/env python3
"""The graph of all readings of a sentence: which links between its words
the grammar allows — learned from parsed text, in the keyboard's own
readings of words (every reading of an ambiguous word kept).

Usage: tools/graph_grammar.py [--parse data/parse] [--sig coarse|fine]
                              [--min 2] [--out data/graph/links.tsv]

A link is `dependent reading → head reading, relation, direction,
distance` (UD relations, as Stanza parses SynTagRus-style). Every link a
parse of the training sentences shows is counted once for each pair of the
two words' readings (split among them); the links seen at least `--min`
times are the ones the grammar allows — with equal chances: the graph keeps
them all, the chooser weighs them. A word with several readings («стали»:
a noun in five cases, a verb) takes part in every link any of them allows.

Measured on the held-out sentences against their parses: how many of the
words' true links the graph holds (with the relation, and the head alone),
how many heads it leaves each word, and in how many sentences the whole
true parse is in the graph — the one reading of the sentence it must never
lose.

`--sig`: what of a reading a link looks at — `coarse` (part of speech,
case), `fine` (and number, gender).
"""
import argparse
import collections
import math
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--parse", default=f"{ROOT}/data/parse")
ap.add_argument("--sig", default="coarse", choices=["coarse", "fine"])
ap.add_argument("--min", type=float, nargs="+", default=[0.5, 2, 5, 20])
ap.add_argument("--limit", type=int, default=0)
ap.add_argument("--out")
args = ap.parse_args()
t0 = time.time()


def log(msg):
    print(f"[{time.time() - t0:5.0f}s] {msg}", file=sys.stderr, flush=True)


CASES = {"nomn", "gent", "datv", "accs", "ablt", "loct", "voct", "gen2", "acc2", "loc2"}
NUMBERS = {"sing", "plur"}
GENDERS = {"masc", "femn", "neut"}
sets = {}
with open(f"{ROOT}/data/readings.tsv", encoding="utf-8") as f:
    for line in f:
        i, tags = line.rstrip("\n").split("\t")
        sets[int(i)] = tags.split("|")
word_set = {}
with open(f"{ROOT}/data/word_readings.tsv", encoding="utf-8") as f:
    for line in f:
        w, i = line.rstrip("\n").split("\t")
        word_set[w] = int(i)


def sig(tag):
    parts = tag.split(",")
    pos = parts[0]
    case = next((p for p in parts if p in CASES), "-")
    if args.sig == "coarse":
        return (pos, case)
    num = next((p for p in parts if p in NUMBERS), "-")
    gen = next((p for p in parts if p in GENDERS), "-")
    return (pos, case, num, gen)


sig_cache = {}


def sigs(word):
    """The word's readings as link signatures (each once)."""
    if word not in sig_cache:
        s = word_set.get(word)
        sig_cache[word] = sorted({sig(t) for t in sets.get(s, [])}) if s else [("UNK",)]
    return sig_cache[word]


def dist(d):
    d = abs(d)
    return 1 if d == 1 else 2 if d == 2 else 3 if d <= 5 else 4


def read(path):
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) < 5:
                continue
            words = p[1].split()
            heads = [int(h) for h in p[3].split()]
            rels = [r.split(":")[0] for r in p[4].split()]
            yield words, heads, rels


# Count the links of the training parses.
count = collections.Counter()
n = 0
for words, heads, rels in read(f"{args.parse}/train.tsv"):
    ss = [sigs(w) for w in words]
    for i, (h, r) in enumerate(zip(heads, rels)):
        if h == 0:
            for a in ss[i]:
                count[(a, "ROOT", r, 0, 0)] += 1 / len(ss[i])
            continue
        j = h - 1
        d = 1 if j > i else -1
        share = 1 / (len(ss[i]) * len(ss[j]))
        for a in ss[i]:
            for b in ss[j]:
                count[(a, b, r, d, dist(j - i))] += share
    n += 1
    if args.limit and n >= args.limit:
        break
log(f"{n} training sentences, {len(count)} link kinds seen")

# Index: for a pair of signatures and a direction/distance, the relations allowed.
valid = list(read(f"{args.parse}/valid.tsv"))
log(f"{len(valid)} held-out sentences")
for th in args.min:
    allowed = collections.defaultdict(set)
    for (a, b, r, d, k), c in count.items():
        if c >= th:
            allowed[(a, b, d, k)].add(r)
    words_n = rel_hit = head_hit = heads_total = options_total = full = 0
    typed_prev = typed_prev_hit = 0
    for words, heads, rels in valid:
        ss = [sigs(w) for w in words]
        whole = True
        for i, (h, r) in enumerate(zip(heads, rels)):
            words_n += 1
            # Every head the grammar allows this word, with its relations.
            opts = {}
            for j in range(len(words)):
                if j == i:
                    continue
                d = 1 if j > i else -1
                rs = set()
                for a in ss[i]:
                    for b in ss[j]:
                        rs |= allowed.get((a, b, d, dist(j - i)), set())
                if rs:
                    opts[j + 1] = rs
            rs = set()
            for a in ss[i]:
                rs |= allowed.get((a, "ROOT", 0, 0), set())
            if rs:
                opts[0] = rs
            heads_total += len(opts)
            options_total += sum(len(v) for v in opts.values())
            if h in opts:
                head_hit += 1
                if r in opts[h]:
                    rel_hit += 1
                else:
                    whole = False
            else:
                whole = False
            # For the keyboard: a word whose head came before it.
            if 0 < h <= i:
                typed_prev += 1
                typed_prev_hit += h in opts and r in opts[h]
        full += whole
    print(f"min {th:>4}: kinds {sum(len(v) for v in allowed.values()):>6}; "
          f"true link held {rel_hit / words_n:.2%} (head {head_hit / words_n:.2%}); "
          f"whole parse held in {full / len(valid):.1%} of sentences; "
          f"heads per word {heads_total / words_n:.2f}, head·relation options {options_total / words_n:.2f}; "
          f"head before the word held {typed_prev_hit / max(typed_prev, 1):.2%}", flush=True)
if args.out:
    th = args.min[0]
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        for (a, b, r, d, k), c in sorted(count.items(), key=lambda x: -x[1]):
            if c >= th:
                f.write(f"{','.join(a)}\t{b if b == 'ROOT' else ','.join(b)}\t{r}\t{d}\t{k}\t{c:.1f}\n")
    log(f"wrote {args.out}")
