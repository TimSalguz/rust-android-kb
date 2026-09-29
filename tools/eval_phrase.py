#!/usr/bin/env python3
"""Phrase-grammar benchmark: type held-out everyday sentences (Tatoeba ids
divisible by 50) with a preposition or a personal pronoun in them through the
real IME (`examples/typetext.rs`) with noisy taps, and count how many words
come out right — all of them, and the ones the grammar constrains (up to three
words after a preposition, the word after a pronoun) — for each KB_SET value;
and, typed carefully, how many right words it changes. Then next-word
prediction (`kbdemo --predict` after the words before, up to five): how often
the word that comes is the first guess or among the three shown.

With `--swipe σ` the words are drawn instead (`typetext` SWIPE: corners
missed by σ key widths), where the grammar matters most: «дома» and «доме»
differ in the last key only.

Usage: tools/eval_phrase.py [--n 600] [--jitter 0.4] [--swipe 0.3] [--set w_phrase=0 w_phrase=1]
Needs `cargo build --release -p kbime --example typetext -p cli` and the assets of
`android/build.sh` in target/apk/assets.
"""
import argparse
import bz2
import os
import re
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--n", type=int, default=600)
ap.add_argument("--jitter", type=float, default=0.4)
ap.add_argument("--seeds", type=int, default=3, help="noisy runs per setting")
ap.add_argument("--set", nargs="+", default=["w_phrase=0", "w_phrase=1"])
ap.add_argument("--sentences", default=f"{ROOT}/data/tatoeba/rus_sentences.tsv.bz2")
ap.add_argument("--show", type=int, default=8, help="print this many sentences the settings disagree on")
ap.add_argument("--predict-only", action="store_true", help="only the next-word prediction")
ap.add_argument("--swipe", type=float, help="draw the words, corners missed by this many key widths")
args = ap.parse_args()

PREP = set("""в во на о об обо при по с со к ко за под подо над перед передо между через
сквозь про у из изо от ото до без безо для около возле вокруг после кроме среди вместо
мимо против ради""".split())
PRON = {"я", "ты", "он", "она", "оно", "мы", "вы", "они"}
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}


def constrained(words):
    """The positions the grammar speaks about."""
    spots = set()
    for i, w in enumerate(words):
        if w in PREP:
            spots.update(j for j in range(i + 1, min(i + 4, len(words))) if len(words[j]) > 2)
        elif w in PRON and i + 1 < len(words):
            spots.add(i + 1)
    return spots


lines, spots = [], []
with bz2.open(args.sentences, "rt", encoding="utf-8") as f:
    for line in f:
        parts = line.rstrip("\n").split("\t")
        if len(parts) < 3 or not parts[0].isdigit() or int(parts[0]) % 50 or STOCK.search(parts[2]):
            continue
        # Only a phrase's words: no commas inside it (the IME's phrase ends at one).
        text = parts[2].lower().replace("ё", "е")
        if re.search(r"[^а-я .!?-]", text):
            continue
        words = re.findall(r"[а-я]+", text)
        s = constrained(words)
        if 3 <= len(words) <= 10 and s and all(w in lexicon for w in words):
            lines.append(" ".join(words))
            spots.append(s)
        if len(lines) >= args.n:
            break

assets = f"{ROOT}/target/apk/assets"
cmd = [f"{ROOT}/target/release/examples/typetext", f"{assets}/dict.fst", f"{assets}/bigrams.fst"]
if os.path.exists(f"{assets}/casing.fst"):
    cmd.append(f"{assets}/casing.fst")


def run(kb_set, jitter, seed=0, inside=False):
    env = dict(os.environ, JITTER=str(jitter), KB_SET=kb_set, SEED=str(seed), INSIDE="1" if inside else "0")
    if args.swipe is not None:
        env["SWIPE"] = str(args.swipe)
    out = subprocess.run(cmd, input="\n".join(lines) + "\n", capture_output=True, text=True, env=env)
    return [o.strip().lower().replace("ё", "е").split() for o in out.stdout.splitlines()]


total = sum(len(l.split()) for l in lines)
marked = sum(len(s) for s in spots)
print(f"{len(lines)} sentences, {total} words, {marked} the grammar speaks about")
noisy = {}
for kb_set in [] if args.predict_only else args.set:
    right = hit = 0
    runs = []
    for seed in range(args.seeds):
        out = run(kb_set, args.jitter, seed)
        runs.append(out)
        for l, o, s in zip(lines, out, spots):
            w = l.split()
            if len(o) != len(w):
                continue
            right += sum(a == b for a, b in zip(w, o))
            hit += sum(w[i] == o[i] for i in s)
    noisy[kb_set] = runs[0]
    n = args.seeds
    if args.swipe is not None:
        print(f"{kb_set:>14}: swiped, σ {args.swipe} — {100 * right / total / n:.2f}% words right, "
              f"{100 * hit / marked / n:.2f}% of the constrained")
        continue
    careful = run(kb_set, args.jitter, inside=True)
    changed = sum(
        sum(a != b for a, b in zip(l.split(), o)) + abs(len(l.split()) - len(o))
        for l, o in zip(lines, careful)
    )
    print(f"{kb_set:>14}: jitter {args.jitter} — {100 * right / total / n:.2f}% words right, "
          f"{100 * hit / marked / n:.2f}% of the constrained; careful taps changed {changed} "
          f"({100 * changed / total:.2f}%)")
    for l, o in zip(lines, careful):
        if l.split() != o:
            print(f"{'':>16}careful: {l} → {' '.join(o)}")

# Next-word prediction after each word (the grammar only where it speaks);
# not with --swipe: it doesn't change with how the words are typed.
kbdemo = f"{ROOT}/target/release/kbdemo"
queries = []  # (phrase, next word, constrained)
guessed = {}
for l, s in zip(lines, spots):
    w = l.split()
    for i in range(1, len(w)):
        queries.append((" ".join(w[max(0, i - 5):i]), w[i], i in s))
for kb_set in [] if args.swipe is not None else args.set:
    env = dict(os.environ, DICT_FST=f"{assets}/dict.fst", BIGRAMS_FST=f"{assets}/bigrams.fst",
               KB_PROFILE="phone", KB_SET=kb_set)
    out = subprocess.run([kbdemo, "--predict"] + [q for q, _, _ in queries],
                         capture_output=True, text=True, env=env).stdout.splitlines()
    stats = {True: [0, 0, 0], False: [0, 0, 0]}
    guessed[kb_set] = []
    for (_, want, c), line in zip(queries, out):
        guesses = [x.strip().split(" ")[0] for x in line.split("→", 1)[1].split(",") if x.strip()]
        guessed[kb_set].append(guesses[:3])
        st = stats[c]
        st[0] += 1
        st[1] += guesses[:1] == [want]
        st[2] += want in guesses[:3]
    a, b = stats[True], stats[False]
    print(f"{kb_set:>14}: next word — where the grammar speaks top-1 {100 * a[1] / a[0]:.1f}% "
          f"top-3 {100 * a[2] / a[0]:.1f}% ({a[0]}); elsewhere top-1 {100 * b[1] / b[0]:.1f}% "
          f"top-3 {100 * b[2] / b[0]:.1f}% ({b[0]})")

if len(args.set) >= 2 and args.show and guessed:
    # The next words the second setting lost from the three shown.
    a, b = (guessed[s] for s in args.set[:2])
    lost = [(q, w, x, y) for (q, w, _), x, y in zip(queries, a, b) if w in x and w not in y]
    won = sum(w in y and w not in x for (_, w, _), x, y in zip(queries, a, b))
    print(f"    next word: {len(lost)} lost from the three shown, {won} gained")
    for q, w, x, y in lost[: args.show]:
        print(f"      {q} → {w}: {' '.join(x)} | {' '.join(y)}")

if len(args.set) >= 2 and args.show and noisy:
    a, b = (noisy[s] for s in args.set[:2])
    shown = 0
    for l, x, y in zip(lines, a, b):
        if x != y and shown < args.show:
            print(f"    {l}\n      {args.set[0]}: {' '.join(x)}\n      {args.set[1]}: {' '.join(y)}")
            shown += 1
