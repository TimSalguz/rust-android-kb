#!/usr/bin/env python3
"""Real-word slips: type held-out everyday sentences (Tatoeba ids divisible by
50) through the real IME (`examples/typetext.rs`) with one word swapped for
its confusion partner (data/confusions.tsv: «ввод» typed as «вод», «не» as
«на») and count how often the keyboard puts the meant word back — and, on the
same sentences typed right, how many words it changes.

A one-letter swap is typed just past the border between the two keys (the
finger meant one and caught the other), anything else at the key centers.
Every KB_SET value is typed separately, e.g. the rules off and on:

Usage: tools/eval_realword.py [--n 600] [--set w_rules=0 w_rules=1]
Needs `cargo build --release -p kbime --example typetext` and the assets of
`android/build.sh` in target/apk/assets.
"""
import argparse
import bz2
import os
import random
import re
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--n", type=int, default=600)
ap.add_argument("--set", nargs="+", default=["w_rules=0", "w_rules=1"])
ap.add_argument("--window", default="1.5")
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--sentences", default=f"{ROOT}/data/tatoeba/rus_sentences.tsv.bz2")
ap.add_argument("--show", type=int, default=8, help="print this many sentences the settings disagree on")
args = ap.parse_args()
rng = random.Random(args.seed)

STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}
partners = {}
with open(f"{ROOT}/data/confusions.tsv", encoding="utf-8") as f:
    for line in f:
        a, b = line.rstrip("\n").split("\t")[:2]
        partners.setdefault(a, []).append(b)
        partners.setdefault(b, []).append(a)

cases = []  # (meant words, position, typed line)
with bz2.open(args.sentences, "rt", encoding="utf-8") as f:
    for line in f:
        parts = line.rstrip("\n").split("\t")
        if len(parts) < 3 or not parts[0].isdigit() or int(parts[0]) % 50 or STOCK.search(parts[2]):
            continue
        words = re.findall(r"[а-яё]+", parts[2].lower().replace("ё", "е"))
        if not (3 <= len(words) <= 10 and all(w in lexicon for w in words)):
            continue
        spots = [i for i, w in enumerate(words) if w in partners]
        if not spots:
            continue
        i = rng.choice(spots)
        meant, slip = words[i], rng.choice(partners[words[i]])
        diff = [j for j, (a, b) in enumerate(zip(meant, slip)) if a != b]
        if len(meant) == len(slip) and len(diff) == 1:
            j = diff[0]
            typed = f"{slip[:j]}[{slip[j]}/{meant[j]}]{slip[j + 1:]}"
        else:
            typed = slip
        cases.append((words, i, " ".join(words[:i] + [typed] + words[i + 1:])))
        if len(cases) >= args.n:
            break

assets = f"{ROOT}/target/apk/assets"
cmd = [f"{ROOT}/target/release/examples/typetext", f"{assets}/dict.fst", f"{assets}/bigrams.fst"]
if os.path.exists(f"{assets}/casing.fst"):
    cmd.append(f"{assets}/casing.fst")


def run(lines, kb_set):
    env = dict(os.environ, JITTER="0", WINDOW=args.window, KB_SET=kb_set)
    out = subprocess.run(cmd, input="\n".join(lines) + "\n", capture_output=True, text=True, env=env)
    return [o.strip().lower().replace("ё", "е").split() for o in out.stdout.splitlines()]


def changed(meant, got, skip=None):
    if len(meant) != len(got):
        return max(1, abs(len(meant) - len(got)))
    return sum(a != b for k, (a, b) in enumerate(zip(meant, got)) if k != skip)


total = sum(len(w) for w, _, _ in cases)
print(f"{len(cases)} sentences, {total} words, one slipped word in each")
results = {}
for kb_set in args.set:
    slipped = run([t for _, _, t in cases], kb_set)
    clean = run([" ".join(w) for w, _, _ in cases], kb_set)
    fixed = sum(len(g) == len(w) and g[i] == w[i] for (w, i, _), g in zip(cases, slipped))
    others = sum(changed(w, g, skip=i) for (w, i, _), g in zip(cases, slipped))
    false = sum(changed(w, g) for (w, _, _), g in zip(cases, clean))
    results[kb_set] = slipped
    print(f"{kb_set:>12}: slip put right {fixed} ({100 * fixed / len(cases):.1f}%), "
          f"other words changed {others}; typed right — changed {false} ({100 * false / total:.2f}%)")

if len(args.set) == 2 and args.show:
    a, b = (results[s] for s in args.set)
    shown = 0
    for (w, i, t), x, y in zip(cases, a, b):
        if x != y and shown < args.show:
            print(f"    {t}\n      {args.set[0]}: {' '.join(x)}\n      {args.set[1]}: {' '.join(y)}")
            shown += 1
