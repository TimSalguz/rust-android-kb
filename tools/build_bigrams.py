#!/usr/bin/env python3
"""Build the context model (bigrams) from open corpora.

Usage: tools/build_bigrams.py SOURCE[:WEIGHT] [...] [--lexicon data/lexicon.tsv]
                              [--min-count 3] [--min-gain 1.0] [--per-word 64]
                              [--per-right 8] [--endings data/endings.tsv]
                              > data/bigrams.tsv

Sources:
- Leipzig Corpora Collection archives (`*.tar.gz`, https://wortschatz.uni-leipzig.de,
  CC BY): their words (`-words.txt`: id, word, count) and significant
  neighbour pairs (`-co_n.txt`: left id, right id, count, significance).
- Tatoeba sentence exports (`*_sentences.tsv.bz2`, https://tatoeba.org,
  CC BY 2.0 FR): everyday sentences; pairs are counted here (two words with
  only a space between them). Sentences whose id is a multiple of 50 are held
  out for tools/eval_context.py; ones about Tatoeba's stock characters (Tom,
  Mary, Boston) are skipped.

Each source is normalised by its token count and mixed with its weight
(default 1), so a small conversational source can outweigh a big web one.
`P(right | left) = count(pair) / count(left)` over the mixture. A pair is kept
only when context really changes the odds — at least e^min_gain times the
word's own frequency (a symmetric test) — and it occurs min_count times; then
each left word keeps its per_word likeliest continuations, and each right word
its per_right likeliest predecessors (в принципе: «принципе» almost always
follows «в», though «в» is followed by thousands of words). Both words must be
in the lexicon. Output: `left<TAB>right<TAB>−ln P×1000` for
`index-builder --bigrams`.

With --endings, also a model of which ending follows which, from the Tatoeba
sentences (Russian only): the left side is a short word itself (в, на, для —
they govern the case) or the last two letters of a longer one (-ых), the right
side the last two letters (or a word of ≤ 2 letters). Stored as
`left<TAB>right<TAB>(PMI + 8)×1000`, PMI shrunk toward 0 for rare pairs:
«неведомых дорожкдж» → дорожках, not дорожка, even though that pair of words
was never seen. Classes are written `=word` or `*ending`
(see `Engine::ending_pmi`).
"""
import argparse
import bz2
import math
import re
import sys
import tarfile
from collections import defaultdict

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--lexicon", default="data/lexicon.tsv")
ap.add_argument("--min-count", type=int, default=3)
ap.add_argument("--min-gain", type=float, default=1.0)
ap.add_argument("--per-word", type=int, default=64)
ap.add_argument("--per-right", type=int, default=8)
ap.add_argument("--endings", default=None)
ap.add_argument("--endings-any-script", action="store_true",
                help="endings of every word, not only Cyrillic ones (inflected Latin-script languages)")
args = ap.parse_args()

# Letters of kbcore::alphabet::CHARSET.
WORD = re.compile(r"[a-zß-öø-ÿœа-яё'-]*[a-zß-öø-ÿœа-яё][a-zß-öø-ÿœа-яё'-]*")
TOKEN = re.compile(r"[a-zß-öø-ÿœа-яё'-]*[a-zß-öø-ÿœа-яё][a-zß-öø-ÿœа-яё'-]*|\s+|.")
HOLDOUT = 50
# Tatoeba's stock characters (thousands of sentences about Tom, Mary and
# Boston) would make «в Бостоне» a top guess: such sentences are skipped.
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*|Tom|Tom's|Tomás|Mary|Mary's|Maria|Marias|María|Marie|Boston)\b")

with open(args.lexicon, encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}


def script(w):
    return "cyr" if re.search("[а-яё]", w) else "lat"


def leipzig(path):
    """(unigram counts, pair counts) of one Leipzig archive."""
    uni, pairs = defaultdict(int), defaultdict(int)
    with tarfile.open(path) as tar:
        members = {m.name.rsplit("-", 1)[-1]: m for m in tar.getmembers() if m.isfile()}
        words = {}
        for raw in tar.extractfile(members["words.txt"]):
            parts = raw.decode("utf-8", "replace").rstrip("\n").split("\t")
            if len(parts) < 3:
                continue
            w = parts[1].lower()
            if WORD.fullmatch(w) and w in lexicon:
                words[parts[0]] = w
                uni[w] += int(parts[2])
        for raw in tar.extractfile(members["co_n.txt"]):
            a, b, count = raw.decode("ascii", "replace").split("\t")[:3]
            if a in words and b in words:
                pairs[(words[a], words[b])] += int(count)
    return uni, pairs


# Which words have their endings counted.
CYR = WORD if args.endings_any_script else re.compile(r"[а-яё-]+")
end_uni = defaultdict(int)     # right class → count
end_left = defaultdict(int)    # left class → count
end_pairs = defaultdict(int)   # (left class, right class) → count


def left_class(w):
    return "=" + w if len(w) <= 3 else "*" + w[-2:]


def right_class(w):
    return "=" + w if len(w) <= 2 else "*" + w[-2:]


def tatoeba(path):
    """(unigram counts, pair counts) of a Tatoeba export, held-out ids skipped."""
    uni, pairs = defaultdict(int), defaultdict(int)
    with bz2.open(path, "rt", encoding="utf-8") as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) < 3 or not parts[0].isdigit() or int(parts[0]) % HOLDOUT == 0:
                continue
            if STOCK.search(parts[2]):
                continue
            prev = None
            for tok in TOKEN.findall(parts[2].lower()):
                if WORD.fullmatch(tok):
                    w = tok if tok in lexicon else None
                    if w:
                        uni[w] += 1
                        if prev:
                            pairs[(prev, w)] += 1
                            if CYR.fullmatch(prev) and CYR.fullmatch(w):
                                end_pairs[(left_class(prev), right_class(w))] += 1
                                end_left[left_class(prev)] += 1
                        if CYR.fullmatch(w):
                            end_uni[right_class(w)] += 1
                    prev = w
                elif tok != " ":
                    prev = None  # punctuation or a wider gap breaks the pair
    return uni, pairs


unigram = defaultdict(float)   # word → weighted share
mass = defaultdict(float)      # script → weighted share of all tokens
pairs = defaultdict(float)     # (left, right) → weighted share
raw = defaultdict(int)         # (left, right) → plain count, for min_count
for spec in args.sources:
    path, _, weight = spec.partition(":")
    weight = float(weight or 1)
    uni, prs = (tatoeba if path.endswith(".bz2") else leipzig)(path)
    tokens = sum(uni.values()) or 1
    for w, c in uni.items():
        unigram[w] += weight * c / tokens
        mass[script(w)] += weight * c / tokens
    for p, c in prs.items():
        pairs[p] += weight * c / tokens
        raw[p] += c
    print(f"{path}: {tokens} tokens, {len(prs)} pairs (weight {weight})", file=sys.stderr)

kept = []
by_left, by_right = defaultdict(list), defaultdict(list)
for (a, b), c in pairs.items():
    if raw[(a, b)] < args.min_count:
        continue
    p = c / unigram[a]
    gain = math.log(p) - math.log(unigram[b] / mass[script(b)])
    if gain >= args.min_gain:
        i = len(kept)
        kept.append((a, b, p))
        by_left[a].append((p, i))
        by_right[b].append((c / unigram[b], i))
chosen = set()
for group, k in ((by_left, args.per_word), (by_right, args.per_right)):
    for cands in group.values():
        chosen.update(i for _, i in sorted(cands, reverse=True)[:k])

# People mostly type е for ё (ее, еще): a pair spelled with ё is also kept
# spelled with е, unless that spelling is a pair of its own (все ≠ всё).
rows = {(a, b): p for a, b, p in (kept[i] for i in chosen)}
for (a, b), p in list(rows.items()):
    plain = (a.replace("ё", "е"), b.replace("ё", "е"))
    if plain != (a, b) and plain not in rows and plain[0] in lexicon and plain[1] in lexicon:
        rows[plain] = p
out = sys.stdout
for (a, b), p in sorted(rows.items()):
    out.write(f"{a}\t{b}\t{round(-math.log(min(p, 1.0)) * 1000)}\n")
print(f"kept {len(rows)} pairs ({len(rows) - len(chosen)} of them ё spelled е) "
      f"for {len(by_left)} left words", file=sys.stderr)

if args.endings:
    total = sum(end_uni.values()) or 1
    n = 0
    with open(args.endings, "w", encoding="utf-8") as f:
        for (a, b), c in sorted(end_pairs.items()):
            if c < args.min_count:
                continue
            pmi = math.log(c / end_left[a]) - math.log(end_uni[b] / total)
            pmi *= c / (c + 5.0)  # rare pairs say less
            f.write(f"{a}\t{b}\t{round((max(-8.0, min(8.0, pmi)) + 8.0) * 1000)}\n")
            n += 1
    print(f"endings: {n} pairs of {len(end_left)} → {len(end_uni)} classes", file=sys.stderr)
