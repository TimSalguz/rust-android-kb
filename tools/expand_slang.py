#!/usr/bin/env python3
"""Inflect slang by analogy: every `word<TAB>count<TAB>model` line of
data/slang.tsv gets all the forms of `model` (an OpenCorpora word inflected
the same way), e.g. `кайфовый 1500 новый` → кайфового, кайфовыми, кайфово,
кайфовее… Prints `form<TAB>count`; the lemma keeps its count, other forms
get a fifth of it. Needs `pip install pymorphy3 pymorphy3-dicts-ru`.

Usage: tools/expand_slang.py data/slang.tsv > data/slang_forms.tsv
"""
import os
import sys

import pymorphy3

morph = pymorphy3.MorphAnalyzer()
out = {}
for line in open(sys.argv[1], encoding="utf-8"):
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
    for f in forms:
        form = base + f[len(stem):]
        c = count if form == word else max(1, count // 5)
        out[form] = max(out.get(form, 0), c)
for form, c in sorted(out.items()):
    print(f"{form}\t{c}")
