#!/usr/bin/env python3
"""Where commas go, learned from running text: how often a comma stands
between two words — by the word after it («что», «который», «но»), the word
before it («конечно», «например»), and the pair where it is common enough to
say more than the two apart.

Usage: tools/build_commas.py SOURCE... [--min 50] > data/commas.tsv
       tools/build_commas.py --eval HELDOUT.tsv.bz2 data/commas.tsv

SOURCE: Tatoeba `*_sentences.tsv.bz2` (held-out ids skipped) or Leipzig
`*.tar.gz`. stdout: `>word<TAB>logit` (before the word), `<word<TAB>logit`
(after it), `word word<TAB>logit` (the pair), `@<TAB>logit` (anywhere): log
odds of a comma, the keyboard adds up the evidence of the two words over the
base odds, the pair when it has its own.

`--eval`: on the held-out Tatoeba sentences, how many of the commas it
would put and how many of those are right, at a few confidence levels.
"""
import argparse
import bz2
import math
import re
import sys
import tarfile
from collections import Counter

HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
TOKEN = re.compile(r"[а-яё]+(?:-[а-яё]+)*|[,.;:!?—–()«»\"]")
WORD = re.compile(r"[а-яё]+(?:-[а-яё]+)*")

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="*")
ap.add_argument("--min", type=int, default=50)
ap.add_argument("--eval", nargs=2, metavar=("HELDOUT", "COMMAS"))
ap.add_argument("--word-readings", default="data/word_readings.tsv")
ap.add_argument("--readings", default="data/readings.tsv")
args = ap.parse_args()

# The kind of a word, by its readings (all of them): what a comma before or
# after it depends on beyond the word itself — a gerund or participle opens
# a phrase, a finite verb closes one, an imperative after a noun at the start
# of a sentence makes that noun an address, two noun phrases in one case side
# by side are homogeneous.
KIND_OF = {"GRND": "GRND", "PRTF": "PRTF", "PRTS": "PRED", "ADJS": "PRED", "INFN": "INFN",
           "ADVB": "ADV", "PRCL": "ADV", "PRED": "ADV", "COMP": "ADV", "PREP": "PREP", "CONJ": "CONJ",
           "INTJ": "INTJ", "NUMR": "NUM"}


def load_kinds():
    sets = {}
    with open(args.readings, encoding="utf-8") as f:
        for line in f:
            i, tags = line.rstrip("\n").split("\t")
            sets[i] = [t.split(",") for t in tags.split("|")]
    kinds = {}
    with open(args.word_readings, encoding="utf-8") as f:
        for line in f:
            w, i = line.rstrip("\n").split("\t")
            ks = set()
            cases = set()
            for t in sets[i]:
                pos = t[0]
                if pos == "VERB":
                    ks.add("VERB" if any(g in t for g in ("past", "pres", "futr")) else "IMPR")
                elif pos in ("NOUN", "NPRO", "ADJF"):
                    c = [g for g in t if g in ("nomn", "gent", "datv", "accs", "ablt", "loct")]
                    cases.update(c)
                    ks.add(("NOM" if c == ["nomn"] else "OBL") + ("A" if pos == "ADJF" else ""))
                else:
                    ks.add(KIND_OF.get(pos, "X"))
            kinds[w] = (ks.pop() if len(ks) == 1 else "MIX", frozenset(cases))
    return kinds


KINDS = load_kinds()


def kind(w):
    return KINDS.get(w, ("UNK", frozenset()))


def pair_kind(a, b):
    """The two words' kinds as a pair: noun phrases side by side sharing a
    case marked so (homogeneous: «яблоки, груши»)."""
    (ka, ca), (kb, cb) = kind(a), kind(b)
    shared = "=" if ka[:3] == kb[:3] and ka[:3] in ("NOM", "OBL") and ca & cb else " "
    return f"{ka}{shared}{kb}"


def sentences(path, heldout=False):
    if path.endswith(".bz2"):
        with bz2.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) != 3 or not p[0].isdigit() or STOCK.search(p[2]):
                    continue
                if (int(p[0]) % HOLDOUT == 0) == heldout:
                    yield p[2]
        return
    with tarfile.open(path) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(member):
            yield raw.decode("utf-8", "replace").split("\t", 1)[-1]


def gaps(text):
    """(word, next word, comma between, state) for the word pairs of a
    sentence with nothing or only a comma between them. State: `^` — the
    first word of the sentence before; `open` — a gerund or participle phrase
    opened by a comma earlier, no comma since; `и` — an «и» among the three
    words before, no comma since («и ветер, и град»)."""
    toks = TOKEN.findall(text.lower())
    start = True
    opened = False
    subject_seen = False  # a noun phrase in the nominative before, this sentence
    opened_after_subject = False
    since_comma = []
    for i in range(len(toks) - 1):
        a = toks[i]
        if not WORD.fullmatch(a):
            if a in ".!?":
                start, opened, since_comma = True, False, []
                subject_seen = opened_after_subject = False
            continue
        b = toks[i + 1]
        state = set()
        if start:
            state.add("^")
        if opened:
            state.add("openS" if opened_after_subject else "open")
        if "и" in since_comma[-3:]:
            state.add("и")
        start = False
        since_comma.append(a)
        if kind(a)[0] in ("NOM", "NOMA"):
            subject_seen = True
        if WORD.fullmatch(b):
            yield a, b, False, state
        elif b == "," and i + 2 < len(toks) and WORD.fullmatch(toks[i + 2]):
            c = toks[i + 2]
            yield a, c, True, state
            # A comma: a phrase opens if a gerund or participle follows,
            # else one open closes.
            opened = kind(c)[0] in ("GRND", "PRTF")
            opened_after_subject = opened and subject_seen
            since_comma = []


def logit(k, n, prior, weight=5.0):
    p = (k + weight * prior) / (n + weight)
    return math.log(p / (1 - p))


def odds(table, base, a, b, state):
    """The log odds of a comma between `a` and `b` — the keyboard's sum
    (kbcore Engine::comma_odds_in): the pair's own if it has them; else what
    each word says, or its kind where the word says nothing, and the kinds as
    a pair where they say more than apart; a gerund or participle phrase open
    — what the next word's kind says then; «и» after an «и»; the first word
    of the sentence — the pair's kinds there (an address)."""
    if f"{a} {b}" in table:
        return table[f"{a} {b}"]
    (ka, _), (kb, _) = kind(a), kind(b)
    ev_b = table.get(f">{b}")
    if ev_b is None:
        ev_b = table.get(f"#>{kb}", base)
    for o in ("open", "openS"):
        if o in state and f"#{o}>{kb}" in table:
            ev_b = table[f"#{o}>{kb}"]
    # A phrase opening after a word of a kind: the pair of kinds.
    if kb in ("GRND", "PRTF") and f"#{ka}>{kb}" in table:
        ev_b = table[f"#{ka}>{kb}"]
    if b == "и" and "и" in state and "#и>и" in table:
        ev_b = table["#и>и"]
    ev_a = table.get(f"<{a}")
    if ev_a is None:
        ev_a = table.get(f"#<{ka}", base)
    guess = base + (ev_b - base) + (ev_a - base)
    kguess = base + (table.get(f"#>{kb}", base) - base) + (table.get(f"#<{ka}", base) - base)
    pk = pair_kind(a, b)
    if "^" in state and f"#^{pk}" in table:
        guess += table[f"#^{pk}"] - kguess
    elif f"#{pk}" in table:
        guess += table[f"#{pk}"] - kguess
    return guess


if args.eval:
    table = {}
    with open(args.eval[1], encoding="utf-8") as f:
        for line in f:
            k, v = line.rstrip("\n").split("\t")
            table[k] = float(v)
    base = table.get("@", -2.0)
    rows = [(odds(table, base, a, b, st), c)
            for text in sentences(args.eval[0], heldout=True) for a, b, c, st in gaps(text)]
    commas = sum(c for _, c in rows)
    print(f"{len(rows)} gaps, {commas} commas")
    for p in (0.5, 0.7, 0.85, 0.9, 0.92, 0.95):
        t = math.log(p / (1 - p))
        put = [c for o, c in rows if o >= t]
        right = sum(put)
        print(f"  sure ≥ {p:.2f}: puts {len(put)}, right {right} ({100 * right / max(len(put), 1):.1f}%), "
              f"finds {100 * right / max(commas, 1):.1f}% of the commas")
    sys.exit()

before, after, pair = Counter(), Counter(), Counter()
n_before, n_after, n_pair = Counter(), Counter(), Counter()
# By kinds (`#`): before and after a kind, a pair of kinds, a pair at the
# sentence's start, the kind after an open phrase, «и» after an «и».
kinds, n_kinds = Counter(), Counter()
total = with_comma = 0
for path in args.sources:
    for text in sentences(path):
        for a, b, c, st in gaps(text):
            total += 1
            with_comma += c
            n_before[b] += 1
            n_after[a] += 1
            n_pair[(a, b)] += 1
            if c:
                before[b] += 1
                after[a] += 1
                pair[(a, b)] += 1
            (ka, _), (kb, _) = kind(a), kind(b)
            pk = pair_kind(a, b)
            feats = [f"#>{kb}", f"#<{ka}", f"#{pk}"]
            if "^" in st:
                feats.append(f"#^{pk}")
            for o in ("open", "openS"):
                if o in st:
                    feats.append(f"#{o}>{kb}")
            if kb in ("GRND", "PRTF"):
                feats.append(f"#{ka}>{kb}")
            if b == "и" and "и" in st:
                feats.append("#и>и")
            for k in feats:
                n_kinds[k] += 1
                kinds[k] += c
    print(f"{path}: {total} gaps, {with_comma} commas", file=sys.stderr)
base_p = with_comma / total
base = math.log(base_p / (1 - base_p))
print(f"@\t{base:.3f}")
kept = 0
for side, counts, ns in ((">", before, n_before), ("<", after, n_after)):
    for w, n in ns.items():
        if n < args.min:
            continue
        v = logit(counts[w], n, base_p)
        if abs(v - base) >= 0.5:
            print(f"{side}{w}\t{v:.3f}")
            kept += 1
# A pair of its own when it says more than its two words apart.
for (a, b), n in n_pair.items():
    if n < args.min:
        continue
    v = logit(pair[(a, b)], n, base_p)
    guess = base
    if n_before[b] >= args.min:
        guess += logit(before[b], n_before[b], base_p) - base
    if n_after[a] >= args.min:
        guess += logit(after[a], n_after[a], base_p) - base
    if abs(v - guess) >= 1.0:
        print(f"{a} {b}\t{v:.3f}")
        kept += 1
# The kinds: before and after one, open phrases, «и» after «и» when they say
# something (0.5 off the base); pairs when they say more than apart.
table = {"@": base}
for k, n in sorted(n_kinds.items()):
    if n < args.min or k[1:].startswith(("MIX", "UNK")) and len(k) < 6:
        continue
    v = logit(kinds[k], n, base_p)
    single = k.startswith(("#>", "#<", "#open>", "#openS>", "#и>")) or k.endswith((">GRND", ">PRTF"))
    if single and abs(v - base) >= 0.5:
        table[k] = v
for k, n in sorted(n_kinds.items()):
    if n < args.min or k.startswith(("#>", "#<", "#open>", "#openS>", "#и>")) or ">" in k:
        continue
    v = logit(kinds[k], n, base_p)
    a_k, _, b_k = k.lstrip("#^").replace("=", " ").partition(" ")
    guess = base + (table.get(f"#>{b_k}", base) - base) + (table.get(f"#<{a_k}", base) - base)
    if abs(v - guess) >= 1.0:
        table[k] = v
# Kept: the kinds the comma rules speak of — a phrase opening before a
# gerund or participle (and after what), one closing (`open`, after a
# subject `openS`), «и» after «и», noun phrases side by side in one case, the
# first word before an imperative or so (an address). Kinds before or after
# anything else, and pairs of other kinds, put wrong commas in more than
# right ones (held-out: «успокойся, ты», «думаешь, или»).
RULES = ("#>GRND", "#>PRTF", "#и>и")
for k, v in table.items():
    if not k.startswith("#"):
        continue
    if (k in RULES or k.startswith(("#open>", "#openS>", "#^NOM")) or "=" in k
            or k.endswith((">GRND", ">PRTF"))):
        print(f"{k}\t{v:.3f}")
        kept += 1
print(f"base rate {100 * base_p:.1f}%; {kept} entries", file=sys.stderr)
