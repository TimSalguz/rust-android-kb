#!/usr/bin/env python3
"""Where commas go, learned from running text: how often a comma stands
between two words — by the word after it («что», «который», «но»), the word
before it («конечно», «например»), and the pair where it is common enough to
say more than the two apart.

Usage: tools/build_commas.py SOURCE... [--min 50] > data/commas.tsv
       tools/build_commas.py --eval HELDOUT.tsv.bz2 data/commas.tsv

SOURCE: Tatoeba `*_sentences.tsv.bz2` (held-out ids skipped) or Leipzig
`*.tar.gz`. stdout: `>word<TAB>logit` (before the word), `<word<TAB>logit`
(after it), `word word<TAB>logit` (the pair), `@<TAB>logit` (anywhere): log
odds of a comma, the keyboard adds up the evidence of the two words over the
base odds, the pair when it has its own.

`--eval`: on the held-out Tatoeba sentences, how many of the commas it
would put and how many of those are right, at a few confidence levels.
"""
import argparse
import bz2
import math
import re
import sys
import tarfile
from collections import Counter

HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
TOKEN = re.compile(r"[а-яё]+(?:-[а-яё]+)*|[,.;:!?—–()«»\"]")
WORD = re.compile(r"[а-яё]+(?:-[а-яё]+)*")

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="*")
ap.add_argument("--min", type=int, default=50)
ap.add_argument("--eval", nargs=2, metavar=("HELDOUT", "COMMAS"))
args = ap.parse_args()


def sentences(path, heldout=False):
    if path.endswith(".bz2"):
        with bz2.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) != 3 or not p[0].isdigit() or STOCK.search(p[2]):
                    continue
                if (int(p[0]) % HOLDOUT == 0) == heldout:
                    yield p[2]
        return
    with tarfile.open(path) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(member):
            yield raw.decode("utf-8", "replace").split("\t", 1)[-1]


def gaps(text):
    """(word, next word, comma between) for the word pairs of a sentence
    with nothing or only a comma between them."""
    toks = TOKEN.findall(text.lower())
    for i in range(len(toks) - 1):
        a = toks[i]
        if not WORD.fullmatch(a):
            continue
        b = toks[i + 1]
        if WORD.fullmatch(b):
            yield a, b, False
        elif b == "," and i + 2 < len(toks) and WORD.fullmatch(toks[i + 2]):
            yield a, toks[i + 2], True


def logit(k, n, prior, weight=5.0):
    p = (k + weight * prior) / (n + weight)
    return math.log(p / (1 - p))


if args.eval:
    table = {}
    with open(args.eval[1], encoding="utf-8") as f:
        for line in f:
            k, v = line.rstrip("\n").split("\t")
            table[k] = float(v)
    base = table.get("@", -2.0)

    def odds(a, b):
        if f"{a} {b}" in table:
            return table[f"{a} {b}"]
        return base + (table.get(f">{b}", base) - base) + (table.get(f"<{a}", base) - base)

    rows = [(odds(a, b), c) for text in sentences(args.eval[0], heldout=True) for a, b, c in gaps(text)]
    commas = sum(c for _, c in rows)
    print(f"{len(rows)} gaps, {commas} commas")
    for p in (0.5, 0.7, 0.85, 0.95):
        t = math.log(p / (1 - p))
        put = [c for o, c in rows if o >= t]
        right = sum(put)
        print(f"  sure ≥ {p:.2f}: puts {len(put)}, right {right} ({100 * right / max(len(put), 1):.1f}%), "
              f"finds {100 * right / max(commas, 1):.1f}% of the commas")
    sys.exit()

before, after, pair = Counter(), Counter(), Counter()
n_before, n_after, n_pair = Counter(), Counter(), Counter()
total = with_comma = 0
for path in args.sources:
    for text in sentences(path):
        for a, b, c in gaps(text):
            total += 1
            with_comma += c
            n_before[b] += 1
            n_after[a] += 1
            n_pair[(a, b)] += 1
            if c:
                before[b] += 1
                after[a] += 1
                pair[(a, b)] += 1
    print(f"{path}: {total} gaps, {with_comma} commas", file=sys.stderr)
base_p = with_comma / total
base = math.log(base_p / (1 - base_p))
print(f"@\t{base:.3f}")
kept = 0
for side, counts, ns in ((">", before, n_before), ("<", after, n_after)):
    for w, n in ns.items():
        if n < args.min:
            continue
        v = logit(counts[w], n, base_p)
        if abs(v - base) >= 0.5:
            print(f"{side}{w}\t{v:.3f}")
            kept += 1
# A pair of its own when it says more than its two words apart.
for (a, b), n in n_pair.items():
    if n < args.min:
        continue
    v = logit(pair[(a, b)], n, base_p)
    guess = base
    if n_before[b] >= args.min:
        guess += logit(before[b], n_before[b], base_p) - base
    if n_after[a] >= args.min:
        guess += logit(after[a], n_after[a], base_p) - base
    if abs(v - guess) >= 1.0:
        print(f"{a} {b}\t{v:.3f}")
        kept += 1
print(f"base rate {100 * base_p:.1f}%; {kept} entries", file=sys.stderr)
