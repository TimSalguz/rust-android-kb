#!/usr/bin/env python3
"""What the keyboard chose among, for the chooser (tools/chooser.py) to learn
from: sentences typed through the whole keyboard (typetext) with noisy taps,
each word's candidates recorded (`DUMP`).

Usage: tools/chooser_data.py [--n 120000] [--procs 6] --out data/chooser

Tatoeba's Russian sentences (`data/tatoeba/rus_sentences.tsv.bz2`) prepared
as tools/eval_phrase.py prepares its own — lowercase, е for ё, no commas,
3–12 words, every word in the dictionary — but never its held-out ones (id a
multiple of 50) nor the stock characters' sentences. Ids ≡ 25 mod 50 go to
`valid`, the rest to `train`. Each shard is typed with its own noise: taps
off by σ = 0.3, 0.4 or 0.5 key widths, or careful (inside the key meant —
every change a false correction: the chooser must learn to leave those).

Writes `--out/{train,valid}.tsv`: `noise<TAB>text before<TAB>word meant<TAB>
typed<TAB>word:cost:edit|…` (typetext's DUMP; noise σ, or `careful`; typed
empty: the words expected before the word's first letter). The keyboard
types without its chooser (`w_chooser=0`): the chooser learns what it
misses.
"""
import argparse
import bz2
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
# (JITTER, INSIDE) of each shard in turn.
NOISE = [("0.4", "0"), ("0.3", "0"), ("0.5", "0"), ("0.4", "0"), ("0.3", "1"), ("0.5", "0")]

ap = argparse.ArgumentParser()
ap.add_argument("--n", type=int, default=120000, help="training sentences")
ap.add_argument("--valid", type=int, default=6000)
ap.add_argument("--procs", type=int, default=6)
ap.add_argument("--out", default=f"{ROOT}/data/chooser")
args = ap.parse_args()
os.makedirs(args.out, exist_ok=True)

with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}
train, valid = [], []
with bz2.open(f"{ROOT}/data/tatoeba/rus_sentences.tsv.bz2", "rt", encoding="utf-8") as f:
    for line in f:
        parts = line.rstrip("\n").split("\t")
        if len(parts) < 3 or not parts[0].isdigit() or STOCK.search(parts[2]):
            continue
        i = int(parts[0])
        if i % 50 == 0:
            continue
        text = parts[2].lower().replace("ё", "е")
        if re.search(r"[^а-я .!?-]", text):
            continue
        words = re.findall(r"[а-я]+(?:-[а-я]+)*", text)
        if not (3 <= len(words) <= 12 and all(w in lexicon for w in words)):
            continue
        (valid if i % 50 == 25 else train).append(" ".join(words))
print(f"{len(train)} training sentences, {len(valid)} validation", file=sys.stderr)
train, valid = train[: args.n], valid[: args.valid]

assets = f"{ROOT}/target/apk/assets"
# The keyboard without the chooser: it learns what the keyboard misses.
typetext = os.environ.get("TYPETEXT", f"{ROOT}/target/release/examples/typetext")
cmd = [typetext, f"{assets}/dict.fst", f"{assets}/bigrams.fst", f"{assets}/casing.fst"]
for name, sents in (("valid", valid), ("train", train)):
    procs = []
    per = (len(sents) + args.procs - 1) // args.procs
    for k in range(args.procs):
        part = sents[k * per:(k + 1) * per]
        jitter, inside = NOISE[k % len(NOISE)]
        env = dict(os.environ, JITTER=jitter, INSIDE=inside, SEED=str(k + 1),
                   DUMP=f"{args.out}/{name}.{k}.tsv", KB_SET="w_chooser=0")
        lines = f"{args.out}/{name}.{k}.txt"
        with open(lines, "w", encoding="utf-8") as f:
            f.write("\n".join(part) + "\n")
        with open(lines, encoding="utf-8") as stdin:
            p = subprocess.Popen(["nice", "-n", "10"] + cmd, stdin=stdin,
                                 stdout=subprocess.DEVNULL, env=env)
        procs.append((p, lines))
    for p, lines in procs:
        p.wait()
        os.remove(lines)
    with open(f"{args.out}/{name}.tsv", "w", encoding="utf-8") as out:
        for k in range(args.procs):
            jitter, inside = NOISE[k % len(NOISE)]
            noise = "careful" if inside == "1" else jitter
            shard = f"{args.out}/{name}.{k}.tsv"
            with open(shard, encoding="utf-8") as f:
                for line in f:
                    out.write(f"{noise}\t{line}")
            os.remove(shard)
    print(f"{name}: written", file=sys.stderr)
