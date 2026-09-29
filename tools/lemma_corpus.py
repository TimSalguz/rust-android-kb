#!/usr/bin/env python3
"""Running text as numbers, for the lemma model (tools/lemma_vectors.py).

Usage: tools/lemma_corpus.py SOURCE... --out data/lemma

SOURCE: Leipzig `*.tar.gz` (their `-sentences.txt`) or Tatoeba
`*_sentences.tsv.bz2` (stock-character sentences skipped, as in
build_bigrams.py). Every 50th sentence is held out — for Tatoeba the ids
tools/eval_phrase.py measures on — for measuring, never for learning.

Writes to `--out`:
- `forms.tsv`: `form<TAB>count<TAB>lemma<TAB>POS`, the form's id its line
  number from 0; the lemma is pymorphy3's likeliest reading of the form
  spelled with its ё (`--yo`: ее is read as её, the same lemma); POS `-`
  where OpenCorpora doesn't know it.
- `train.npy`, `heldout.npy`: int32 form ids of the sentences one after the
  other; −1 ends a sentence, −2 stands for what isn't a Russian word (a
  number, a Latin word, punctuation) — the words either side of it are not
  neighbours.
"""
import argparse
import array
import bz2
import multiprocessing
import os
import re
import sys
import tarfile

import numpy as np
import pymorphy3

HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
TOKEN = re.compile(r"[а-яё]+(?:-[а-яё]+)*|[^\s]")
END, BREAK = -1, -2

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--out", default="data/lemma")
ap.add_argument("--yo", default="data/yo.tsv", help="е-spellings of ё-words (tools/yo_words.py)")
args = ap.parse_args()
os.makedirs(args.out, exist_ok=True)


def sentences(path):
    """(held out?, text) for each sentence of a source."""
    if path.endswith(".bz2"):
        with bz2.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) == 3 and p[0].isdigit() and p[1] == "rus" and not STOCK.search(p[2]):
                    yield int(p[0]) % HOLDOUT == 0, p[2]
        return
    with tarfile.open(path) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for i, raw in enumerate(tar.extractfile(member)):
            yield i % HOLDOUT == 0, raw.decode("utf-8", "replace").rstrip("\n").split("\t", 1)[-1]


ids = {}
counts = []
train, held = array.array("i"), array.array("i")
for path in args.sources:
    n = 0
    for out_held, text in sentences(path):
        out = held if out_held else train
        for tok in TOKEN.findall(text.lower()):
            if tok[0] in "абвгдеёжзийклмнопрстуфхцчшщъыьэюя":
                i = ids.setdefault(tok, len(ids))
                if i == len(counts):
                    counts.append(0)
                if not out_held:
                    counts[i] += 1
                out.append(i)
            elif out and out[-1] != BREAK:
                out.append(BREAK)
        out.append(END)
        n += 1
    print(f"{path}: {n} sentences, {len(ids)} forms, {len(train)} + {len(held)} tokens",
          file=sys.stderr)
np.save(f"{args.out}/train.npy", np.frombuffer(train, dtype=np.int32))
np.save(f"{args.out}/heldout.npy", np.frombuffer(held, dtype=np.int32))
del train, held

forms = list(ids)
yo = {}
if os.path.exists(args.yo):
    with open(args.yo, encoding="utf-8") as f:
        yo = dict(line.rstrip("\n").split("\t") for line in f)


def lemmas_of(chunk):
    m = pymorphy3.MorphAnalyzer()
    out = []
    for w in chunk:
        p = m.parse(yo.get(w, w))
        best = p[0] if p else None
        out.append((best.normal_form, str(best.tag.POS or "-")) if best else (w, "-"))
    return out


chunks = [forms[i:i + 20000] for i in range(0, len(forms), 20000)]
with open(f"{args.out}/forms.tsv", "w", encoding="utf-8") as f, \
        multiprocessing.get_context("fork").Pool(4) as pool:
    at = 0
    for part in pool.imap(lemmas_of, chunks):
        for lemma, pos in part:
            f.write(f"{forms[at]}\t{counts[at]}\t{lemma}\t{pos}\n")
            at += 1
print(f"{len(forms)} forms written", file=sys.stderr)
