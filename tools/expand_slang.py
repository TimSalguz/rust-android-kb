#!/usr/bin/env python3
"""Inflect slang by analogy: every `word<TAB>count<TAB>model` line of
data/slang.tsv gets all the forms of `model` (an OpenCorpora word inflected
the same way), e.g. `кайфовый 1500 новый` → кайфового, кайфовыми, кайфово,
кайфовее… Prints `form<TAB>count`; the lemma keeps its count, and each other
form gets it in the share its model's form has in running text (`--freq`:
«новы» is 63 in 27283 «новый», so «кайфовы» 3 in 1500 «кайфовый», not a
rival of the full form); without the counts, a fifth. Needs
`pip install pymorphy3 pymorphy3-dicts-ru`.

Usage: tools/expand_slang.py data/slang.tsv [--freq ru_full.txt] > data/slang_forms.tsv
"""
import argparse
import os
import sys

import pymorphy3

ap = argparse.ArgumentParser()
ap.add_argument("slang")
ap.add_argument("--freq", help="`word count` lines: the model's forms' counts")
args = ap.parse_args()
freq = {}
if args.freq:
    with open(args.freq, encoding="utf-8") as f:
        for line in f:
            w, _, c = line.strip().partition(" ")
            if c.isdigit():
                freq[w] = int(c)
morph = pymorphy3.MorphAnalyzer()
out = {}
for line in open(args.slang, encoding="utf-8"):
    if line.startswith("#"):
        continue
    parts = line.rstrip("\n").split("\t")
    if len(parts) < 3 or not parts[2]:
        continue
    word, count, model = parts[0], int(parts[1]), parts[2]
    parse = next((p for p in morph.parse(model) if p.normal_form == model), None)
    if parse is None:
        print(f"skip {word}: {model!r} is not a dictionary lemma", file=sys.stderr)
        continue
    # The model's stem: what its lemma shares with its forms (ignoring forms
    # built with a prefix, like по-новее).
    forms = [f.word for f in parse.lexeme if f.word[:2] == model[:2]]
    stem = os.path.commonprefix(forms + [model])
    suffix = model[len(stem):]
    if not word.endswith(suffix):
        print(f"skip {word}: doesn't end like {model} (-{suffix})", file=sys.stderr)
        continue
    base = word[: len(word) - len(suffix)]
    lemma_uses = freq.get(model, 0)
    for f in forms:
        form = base + f[len(stem):]
        if form == word:
            c = count
        elif lemma_uses:
            # The share the model's form has, capped at the lemma's own count.
            c = max(1, round(count * min(1.5, (freq.get(f, 0) + 0.5) / lemma_uses)))
        else:
            c = max(1, count // 5)
        out[form] = max(out.get(form, 0), c)
for form, c in sorted(out.items()):
    print(f"{form}\t{c}")
