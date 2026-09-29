#!/usr/bin/env python3
"""Rule by rule: held-out sentences with one kind of mistake put in, typed
through the real keyboard (`examples/typetext.rs`, taps at the key centers),
and how many of the mistakes come out fixed — and, the same sentences typed
right, how many right words the keyboard changes.

Usage: tools/eval_rules.py [--n 300] [--only tsya,hyphen,…] [--set w_phrase=0.5]

Kinds of mistakes (each only where the sentence has what it needs):
  tsya      -тся ↔ -ться (он учится → он учиться)
  hyphen    a hyphenated word typed as two (кто-то → кто то, по-русски)
  yo        ё typed as е, where е and ё are one word (ещё → еще)
  prep      к/ко, с/со, в/во, о/об/обо swapped (ко мне → к мне)
  nn        н / нн swapped where both are words (раненый ↔ раненный)
  agree     a past verb's gender after a subject noun (девочка побежала →
            побежал)
  commas    the commas before a gerund or participle taken out (the keyboard
            puts commas in by itself: COMMAS=1)

Held-out: Tatoeba sentences with ids divisible by 50 (as the models leave
them out). Needs `cargo build --release -p kbime --example typetext` and the
assets of `android/build.sh` in target/apk/assets.
"""
import argparse
import bz2
import os
import re
import subprocess
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--n", type=int, default=300, help="sentences per kind of mistake")
ap.add_argument("--only", default="", help="kinds, comma-separated")
ap.add_argument("--set", default="", help="KB_SET for the engine")
ap.add_argument("--show", type=int, default=3, help="print this many misses per kind")
args = ap.parse_args()

STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
WORD = re.compile(r"[а-яё]+(?:-[а-яё]+)*")
lexicon = {}
with open(f"{ROOT}/data/lexicon.tsv", encoding="utf-8") as f:
    for line in f:
        w, _, c = line.rstrip("\n").partition("\t")
        lexicon[w] = int(c or 0)
readings = {}
with open(f"{ROOT}/data/word_readings.tsv", encoding="utf-8") as f:
    for line in f:
        w, i = line.rstrip("\n").split("\t")
        readings[w] = i
sets = {}
with open(f"{ROOT}/data/readings.tsv", encoding="utf-8") as f:
    for line in f:
        i, tags = line.rstrip("\n").split("\t")
        sets[i] = tags.split("|")


def tags(w):
    return sets.get(readings.get(w, ""), [])


sentences = []
with bz2.open(f"{ROOT}/data/tatoeba/rus_sentences.tsv.bz2", "rt", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) != 3 or not p[0].isdigit() or int(p[0]) % 50 or STOCK.search(p[2]):
            continue
        t = p[2].strip().lower()
        # Words, spaces, hyphens inside words, and , . ! ? only.
        if re.search(r"[^а-яё ,.!?-]", t) or re.search(r"(^|\s)-|-(\s|$)", t):
            continue
        sentences.append(t)

PREPS = [("к", "ко"), ("с", "со"), ("в", "во"), ("о", "об"), ("об", "обо"), ("о", "обо")]


def mistakes(kind, t):
    """(typed with the mistake, the words meant, positions of the mistake)
    or None."""
    words = t.split()
    bare = [w.strip(",.!?") for w in words]
    if kind == "tsya":
        for i, w in enumerate(bare):
            for a, b in (("тся", "ться"), ("ться", "тся")):
                if w.endswith(a) and w[: -len(a)] + b in lexicon:
                    typed = words[:]
                    typed[i] = words[i].replace(w, w[: -len(a)] + b)
                    return " ".join(typed), t, {i}
    if kind == "hyphen":
        for i, w in enumerate(bare):
            if "-" in w and w in lexicon:
                return t.replace(w, w.replace("-", " "), 1), t, {i}
    if kind == "yo":
        pos = {i for i, w in enumerate(bare)
               if "ё" in w and w.replace("ё", "е") in lexicon
               and readings.get(w) == readings.get(w.replace("ё", "е"))}
        if pos:
            return t.replace("ё", "е"), t, pos
    if kind == "prep":
        for i, w in enumerate(bare[:-1]):
            for a, b in PREPS:
                if w == b and words[i] == b:
                    typed = words[:]
                    typed[i] = a
                    return " ".join(typed), t, {i}
    if kind == "nn":
        for i, w in enumerate(bare):
            m = re.search(r"(нн|н)(ый|ая|ое|ые|ого|ой|ую|ым|ых|ом)$", w)
            if not m:
                continue
            other = w[: m.start(1)] + ("н" if m.group(1) == "нн" else "нн") + m.group(2)
            if other in lexicon and lexicon[other] > 10:
                return t.replace(w, other, 1), t, {i}
    if kind == "agree":
        for i, w in enumerate(bare[1:], 1):
            if not re.fullmatch(r"\w+ла", w) or w[:-1] not in lexicon:
                continue
            if any(g.startswith("VERB") and "femn" in g for g in tags(w)) and \
                    any(g.startswith("NOUN") and "femn" in g and "nomn" in g for g in tags(bare[i - 1])):
                typed = words[:]
                typed[i] = words[i].replace(w, w[:-1])
                return " ".join(typed), t, {i}
    if kind == "commas":
        pos = set()
        for i, w in enumerate(words[:-1]):
            nxt = bare[i + 1]
            if w.endswith(",") and any(g.startswith(("GRND", "PRTF")) for g in tags(nxt)):
                pos.add(i)
        if pos:
            return t.replace(",", ""), t, pos
    return None


cmd = [f"{ROOT}/target/release/examples/typetext", f"{ROOT}/target/apk/assets/dict.fst",
       f"{ROOT}/target/apk/assets/bigrams.fst", f"{ROOT}/target/apk/assets/casing.fst"]


def run(lines, commas):
    env = dict(os.environ, JITTER="0", KB_SET=args.set, COMMAS="1" if commas else "0")
    out = subprocess.run(cmd, input="\n".join(lines) + "\n", capture_output=True, text=True, env=env)
    return [o.strip().lower() for o in out.stdout.splitlines()]


def tokens(t, keep_commas, yo=True):
    """The words (with a comma after, when commas count); е and ё alike
    unless ё is what is measured (the sentences are not all spelled with
    their ё, the keyboard puts it in)."""
    t = t if keep_commas else t.replace(",", "")
    if not yo:
        t = t.replace("ё", "е")
    return re.findall(r"[а-яё]+(?:-[а-яё]+)*,?", t)


kinds = [k for k in ["tsya", "hyphen", "yo", "prep", "nn", "agree", "commas"]
         if not args.only or k in args.only.split(",")]
print(f"{'':>8} {'sentences':>9} {'fixed':>12} {'right words changed':>22}")
for kind in kinds:
    cases = []
    for t in sentences:
        m = mistakes(kind, t)
        if m:
            cases.append(m)
        if len(cases) >= args.n:
            break
    commas = kind == "commas"
    # What the keyboard should give: the sentence as meant, without its
    # final punctuation (typetext types it as is, commas only where meant).
    typed = [re.sub(r"[.!?]+$", "", c[0]) for c in cases]
    meant = [re.sub(r"[.!?]+$", "", c[1]) for c in cases]
    got = run(typed, commas)
    # Typed right: the sentence as meant (for commas: typed without them all,
    # what counts is a comma where none is meant).
    got_clean = run(meant if not commas else [m.replace(",", "") for m in meant], commas)
    fixed = total = 0
    changed = words = 0
    misses = []
    false = []
    for (_, _, pos), m, g, gc in zip(cases, meant, got, got_clean):
        yo = kind == "yo"
        want = tokens(m, commas, yo)
        have = tokens(g, commas, yo)
        clean = tokens(gc, commas, yo)
        if kind == "hyphen":  # two words typed, one meant: compare the text
            ok = have == want
            fixed += ok
            total += 1
        else:
            for i in pos:
                total += 1
                ok = i < len(have) and len(have) == len(want) and have[i] == want[i]
                fixed += ok
        if not ok and len(misses) < args.show:
            misses.append(f"    {m}  →  {g}")
        # Typed right: words the keyboard changed (commas: a comma put where
        # none is meant).
        n = min(len(clean), len(want))
        words += len(want)
        if commas:
            changed += sum(a.endswith(",") and not b.endswith(",") for a, b in zip(clean[:n], want[:n]))
        else:
            wrong = [(b, a) for a, b in zip(clean[:n], want[:n]) if a != b]
            changed += len(wrong) + abs(len(clean) - len(want))
            if wrong and len(false) < args.show:
                false.append("    typed right: " + ", ".join(f"{b} → {a}" for b, a in wrong))
    print(f"{kind:>8} {len(cases):>9} {fixed:>5}/{total:<5} ({100 * fixed / max(total, 1):5.1f}%)"
          f" {changed:>8}/{words:<6} ({100 * changed / max(words, 1):.2f}%)")
    for m in misses + false:
        print(m)
