#!/usr/bin/env python3
"""Word priors factored as lemma × form, so that the dictionary automaton
shares the endings of whole paradigms again.

Usage: tools/factor_priors.py LEMMAS.tsv [--lexicon data/lexicon.tsv]
           [--classes data/classes.tsv] [--readings data/word_readings.tsv]
           [--exceptions 30000] [--quantum 50] > data/priors.tsv

A word's prior is `−ln P(word) × 1000`. Stored exactly, it is different for
every form, and the ending of «любила» carries another remainder than that of
«носила» — the automaton can't merge them. Factored,

    −ln P(form) ≈ −ln P(lemma) − ln P(slot)

where the slot is the form's grammar (its class and reading set) and
P(slot) is how often a lemma's use takes that form, pooled over all lemmas
with it. The first part sits on the stem, the second is the same for the
same ending in every paradigm — both rounded to steps of `--quantum` apart,
so the sums repeat exactly. The exceptions keep their own prior: the
`--exceptions` forms where the factoring costs the corpus the most — uses ×
error, the nats a wrong prior loses over the text («его», «дома» — at home);
an exception breaks its paradigm's shared tail, so it has to be worth its
bytes (minimum description length). So do
words without a lemma (other languages, unknown to OpenCorpora).

Rare forms gain from it too: a form never seen in the corpus gets its
lemma's share instead of the floor of all unseen words.

stdout: `word<TAB>prior` for the index-builder's `--priors`.
"""
import argparse
import math
import sys
from collections import defaultdict

ap = argparse.ArgumentParser()
ap.add_argument("lemmas")
ap.add_argument("--lexicon", default="data/lexicon.tsv")
ap.add_argument("--classes", default="data/classes.tsv")
ap.add_argument("--readings", default="data/word_readings.tsv")
ap.add_argument("--exceptions", type=int, default=30000)
ap.add_argument("--quantum", type=int, default=50)
args = ap.parse_args()


def column(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            w, v = line.rstrip("\n").split("\t")[:2]
            out[w] = v
    return out


counts = {}
with open(args.lexicon, encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        try:
            counts[p[0]] = int(p[1])
        except (IndexError, ValueError):
            counts[p[0]] = 0
total = max(sum(counts.values()), 1)
log_total = math.log(total)


def exact(w):
    return log_total - math.log(counts.get(w, 0) + 0.5)


lemma = column(args.lemmas)
cls = column(args.classes)
sets = column(args.readings)
slot = {w: (cls.get(w, "0"), sets.get(w, "0")) for w in lemma}

lemma_count = defaultdict(int)
for w, l in lemma.items():
    lemma_count[l] += counts.get(w, 0)
# How often a lemma's use takes each slot, pooled over the lemmas with it.
used, offered = defaultdict(float), defaultdict(float)
for w, l in lemma.items():
    used[slot[w]] += counts.get(w, 0)
    offered[slot[w]] += lemma_count[l]
share = {s: (used[s] + 0.5) / (offered[s] + 1.0) for s in used}

q = args.quantum / 1000


def step(x):
    return round(x / q) * q


factored = {
    w: step(log_total - math.log(lemma_count[lemma[w]] + 0.5)) + step(-math.log(share[slot[w]]))
    for w in lemma
}
# What each factored prior costs the text: its uses × its error.
loss = {w: counts.get(w, 0) * abs(f - exact(w)) for w, f in factored.items()}
exceptions = set(sorted(loss, key=loss.get, reverse=True)[: args.exceptions])
kept_loss = sum(loss[w] for w in exceptions)
uses = sum(counts.get(w, 0) for w in factored)
for w in counts:
    prior = factored[w] if w in factored and w not in exceptions else exact(w)
    print(f"{w}\t{max(0, round(prior * 1000))}")
print(f"{len(lemma_count)} lemmas, {len(share)} slots; {len(exceptions)} exceptions take back "
      f"{100 * kept_loss / max(sum(loss.values()), 1):.0f}% of the loss; the rest off by "
      f"{(sum(loss.values()) - kept_loss) / max(uses, 1):.3f} nats per use", file=sys.stderr)
