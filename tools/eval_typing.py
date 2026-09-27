#!/usr/bin/env python3
"""Whole-keyboard benchmark: type held-out everyday sentences (Tatoeba ids
divisible by 50, never used for the context model) through the real IME
(`examples/typetext.rs`) and compare what lands in the field with what was
meant.

- exact taps: how many correctly typed words the keyboard changed (every
  change is a false correction);
- noisy taps (Gaussian, σ = --jitter key widths): how many words come out
  right, against how many taps hit the right key.

Usage: tools/eval_typing.py [--n 400] [--jitter 0.35] [--window 1.5 inf]
Needs `cargo build --release -p kbime --example typetext` and the assets of
`android/build.sh` in target/apk/assets.
"""
import argparse
import bz2
import os
import re
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--n", type=int, default=400)
ap.add_argument("--jitter", type=float, default=0.35)
ap.add_argument("--window", nargs="+", default=["1.5", "inf"])
ap.add_argument("--known", nargs="+", default=[None], help="KNOWN_SLIP values to try")
ap.add_argument("--sentences", default=f"{ROOT}/data/tatoeba/rus_sentences.tsv.bz2")
ap.add_argument("--show", type=int, default=5, help="print this many changed sentences")
args = ap.parse_args()

STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}

lines = []
with bz2.open(args.sentences, "rt", encoding="utf-8") as f:
    for line in f:
        parts = line.rstrip("\n").split("\t")
        if len(parts) < 3 or not parts[0].isdigit() or int(parts[0]) % 50 or STOCK.search(parts[2]):
            continue
        words = re.findall(r"[а-яё]+", parts[2].lower().replace("ё", "е"))
        if 3 <= len(words) <= 10 and all(w in lexicon for w in words):
            lines.append(" ".join(words))
        if len(lines) >= args.n:
            break

assets = f"{ROOT}/target/apk/assets"
cmd = [f"{ROOT}/target/release/examples/typetext", f"{assets}/dict.fst", f"{assets}/bigrams.fst"]
if os.path.exists(f"{assets}/casing.fst"):
    cmd.append(f"{assets}/casing.fst")


def run(jitter, window, known, inside=False):
    env = dict(os.environ, JITTER=str(jitter), WINDOW=window, INSIDE="1" if inside else "0")
    if known is not None:
        env["KNOWN"] = known
    out = subprocess.run(cmd, input="\n".join(lines) + "\n", capture_output=True, text=True, env=env)
    return [o.strip().lower().replace("ё", "е").split() for o in out.stdout.splitlines()]


total = sum(len(l.split()) for l in lines)
print(f"{len(lines)} sentences, {total} words")
for window, known in [(w, k) for w in args.window for k in args.known]:
    exact = run(0.0, window, known)
    changed = sum(
        sum(a != b for a, b in zip(l.split(), o)) + abs(len(l.split()) - len(o))
        for l, o in zip(lines, exact)
    )
    diffs = [(l, " ".join(o)) for l, o in zip(lines, exact) if l.split() != o]
    for l, o in diffs[: args.show]:
        print(f"    {l}  →  {o}")
    # Careful typing: off-center taps, always inside the key meant.
    careful = run(args.jitter, window, known, inside=True)
    false_fix = sum(
        sum(a != b for a, b in zip(l.split(), o)) + abs(len(l.split()) - len(o))
        for l, o in zip(lines, careful)
    )
    noisy = run(args.jitter, window, known)
    right = sum(sum(a == b for a, b in zip(l.split(), o)) for l, o in zip(lines, noisy) if len(o) == len(l.split()))
    print(f"window {window:>4} known {known or '-':>4}: changed — exact taps {changed}, "
          f"careful taps {false_fix} ({100 * false_fix / total:.2f}%); "
          f"jitter {args.jitter} — {100 * right / total:.1f}% words right")
