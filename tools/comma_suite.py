#!/usr/bin/env python3
"""The commas' suite: each sentence of tests/commas-ru.tsv typed through the
real keyboard (examples/typetext.rs: taps at the key centers, the commas
the keyboard is sure of put in as typed, the sentence's proofreading at its
final mark) without its commas, and set against how it should come out —
by kind: sentences right, commas missed, commas put where none belong.

Usage: tools/comma_suite.py [--assets target/apk/assets] [--show] [--known]
Needs `cargo build --release -p kbime --example typetext` and the assets
android/build.sh makes (sentence.bin among them). Exit status 1 when a
sentence comes out wrong that isn't among the known ones
(tests/commas-ru.known: what the keyboard doesn't get right yet — a
sentence that comes right is said so, to take it out); `--known` writes
that list anew.
"""
import argparse
import collections
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--assets", default=f"{ROOT}/target/apk/assets")
ap.add_argument("--show", action="store_true", help="print every sentence, not only the wrong ones")
ap.add_argument("--known", action="store_true", help="write tests/commas-ru.known anew")
opts = ap.parse_args()

cases = []
for line in open(f"{ROOT}/tests/commas-ru.tsv", encoding="utf-8"):
    if line.startswith("#") or "\t" not in line:
        continue
    kind, sentence = line.rstrip("\n").split("\t", 1)
    cases.append((kind, sentence))

WORD = re.compile(r"[^\W_]+(?:-[^\W_]+)*")


def read(text):
    """The words (lowercase, ё as е) and whether a comma follows each."""
    words, commas = [], []
    for m in WORD.finditer(text):
        if words:
            commas[-1] = "," in text[prev_end:m.start()]
        words.append(m.group(0).lower().replace("ё", "е"))
        commas.append(False)
        prev_end = m.end()
    return words, commas


typed = "\n".join(re.sub(r" +", " ", s.replace(",", "")) for _, s in cases) + "\n"
a = opts.assets
env = dict(os.environ, COMMAS="1", NOSPACE="1")
out = subprocess.run([f"{ROOT}/target/release/examples/typetext", f"{a}/dict.fst", f"{a}/bigrams.fst",
                      f"{a}/casing.fst"], input=typed, capture_output=True, text=True, env=env, check=True)
got_lines = out.stdout.rstrip("\n").split("\n")
assert len(got_lines) == len(cases), (len(got_lines), len(cases))

KNOWN = f"{ROOT}/tests/commas-ru.known"
known = set()
if os.path.exists(KNOWN):
    known = {l.rstrip("\n") for l in open(KNOWN, encoding="utf-8") if l.strip() and not l.startswith("#")}
tally = collections.OrderedDict()
wrong = 0
failed = []
for (kind, want), got in zip(cases, got_lines):
    t = tally.setdefault(kind, [0, 0, 0, 0, 0])  # sentences, right, missed, extra, words changed
    t[0] += 1
    ww, wc = read(want)
    gw, gc = read(got)
    if ww != gw:
        t[4] += 1
        wrong += 1
        failed.append(want)
        print(f"  words  [{kind}] {want}  →  {got}")
        continue
    missed = sum(1 for x, y in zip(wc, gc) if x and not y)
    extra = sum(1 for x, y in zip(wc, gc) if y and not x)
    t[2] += missed
    t[3] += extra
    if missed or extra:
        wrong += 1
        failed.append(want)
        print(f"  {'missed' if missed else 'extra '} [{kind}] {want}  →  {got}")
    else:
        t[1] += 1
        if opts.show:
            print(f"  ok     [{kind}] {got}")
print()
print(f"{'kind':<20} {'right':>9} {'missed':>7} {'extra':>6} {'words':>6}")
for kind, (n, ok, missed, extra, changed) in tally.items():
    print(f"{kind:<20} {ok:>4}/{n:<4} {missed:>7} {extra:>6} {changed:>6}")
n = sum(t[0] for t in tally.values())
print(f"{'all':<20} {n - wrong:>4}/{n:<4} {sum(t[2] for t in tally.values()):>7} "
      f"{sum(t[3] for t in tally.values()):>6} {sum(t[4] for t in tally.values()):>6}")
if opts.known:
    with open(KNOWN, "w", encoding="utf-8") as f:
        f.write("# What the keyboard doesn't get right yet (tools/comma_suite.py --known).\n")
        f.writelines(w + "\n" for w in failed)
    print(f"wrote {KNOWN}: {len(failed)}")
    sys.exit(0)
new = [w for w in failed if w not in known]
fixed = [w for w in known if w not in failed]
for w in fixed:
    print(f"  fixed now (take it out of the known): {w}")
for w in new:
    print(f"  NEW: {w}")
sys.exit(1 if new else 0)
