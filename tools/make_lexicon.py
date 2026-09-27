#!/usr/bin/env python3
"""Build data/lexicon.tsv (`word<TAB>count`) from frequency lists, optionally
filtered by a word list.

Usage: tools/make_lexicon.py FREQ.txt [FREQ.txt ...] [--words WORDS.txt] [--min-count 3]
                              [--keep-frequent 300] [--extra data/slang.tsv]
                              [--forms data/opencorpora_forms.tsv] [--wordlist LIST:FLOOR ...]
                              [--cyrillic-words-only] [--latin-lists-only] [--yo-variants]
                              [-o data/lexicon.tsv]

FREQ files are `word count` per line (hermitdave/FrequencyWords format). A word
seen in several lists keeps its largest count. With --words, only words present
in that list are kept — this drops the typos and junk that subtitle corpora are
full of, and measurably improves correction (docs/DESIGN.md). Words at least
--keep-frequent times in the corpus are kept even when the word list lacks them
(brands, hyphenated forms like вообще-то, slang) — minus subtitle artifacts such
as stutters (i-i) and clipped tokens ('cause). --extra adds hand-curated
`word<TAB>count` lists (chat slang, abbreviations) as they are. --forms adds
every word form of a morphological dictionary (`form<TAB>lemma`, from
tools/opencorpora_forms.py): a form seen in the corpus keeps its count; an
unseen one gets --form-share of its lemma's most frequent form (at least 1), so
forms of common words are plausible and forms of rare ones stay rare.
--wordlist LIST:FLOOR adds a plain word list (e.g. an SCOWL level): a word seen
in the corpus keeps its count, an unseen one gets FLOOR. With
--cyrillic-words-only, --words only vouches for Cyrillic words; Latin corpus
words then need a --wordlist (or --keep-frequent). With --latin-lists-only, Latin words come only
from --wordlist (no --keep-frequent: subtitle "words" like `dont` stay out).
--words may be a TSV; its first column is used. --yo-variants adds the е
spelling of every word with ё (people type еще, актерская): typed that way it
is a word, not a typo to "fix"; the ё form still shows up as a suggestion.
"""
import argparse
import re

ap = argparse.ArgumentParser()
ap.add_argument("freq", nargs="+")
ap.add_argument("--words")
ap.add_argument("--min-count", type=int, default=3)
ap.add_argument("--keep-frequent", type=int, default=300)
ap.add_argument("--extra", action="append", default=[], help="word<TAB>count list added as is")
ap.add_argument("--forms", help="form<TAB>lemma list; every form joins the lexicon")
ap.add_argument("--form-share", type=float, default=0.02)
ap.add_argument("--wordlist", action="append", default=[], help="LIST:FLOOR plain word list")
ap.add_argument("--cyrillic-words-only", action="store_true")
ap.add_argument("--latin-lists-only", action="store_true")
ap.add_argument("--yo-variants", action="store_true")
ap.add_argument("-o", "--out", default="data/lexicon.tsv")
args = ap.parse_args()

# Must match kbcore::alphabet::CHARSET (a-z, Latin-1 letters, œ, а-я, ё).
WORD = re.compile(r"[a-zß-öø-ÿœа-яё'-]*[a-zß-öø-ÿœа-яё][a-zß-öø-ÿœа-яё'-]*")
# Subtitle artifacts: clipped tokens ('cause, goin') and stutters (i-i, w-what).
ARTIFACT = re.compile(r"^['-]|['-]$|^(\w{1,2})-\1")

counts = {}
raw = {}  # every corpus count, for --forms
for path in args.freq:
    with open(path, encoding="utf-8") as f:
        for line in f:
            parts = line.split()
            if len(parts) < 2:
                continue
            w = parts[0].lower()
            try:
                c = int(parts[1])
            except ValueError:
                continue
            if (args.forms or args.wordlist) and c > raw.get(w, 0):
                raw[w] = c
            if c >= args.min_count and WORD.fullmatch(w) and c > counts.get(w, 0):
                counts[w] = c

def read_list(path):
    """A plain word list, UTF-8 or Latin-1 (SCOWL), lowercased."""
    with open(path, "rb") as f:
        data = f.read()
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        text = data.decode("latin-1")
    return [w.strip().lower() for w in text.splitlines() if w.strip()]


lists = []  # (words, floor)
for spec in args.wordlist:
    path, floor = spec.rsplit(":", 1)
    lists.append(([w for w in read_list(path) if WORD.fullmatch(w)], int(floor)))

# Words run together by mistake that the subtitles have often enough to pass
# for slang: не with a verb (незнаю — не с глаголами пишется раздельно), a
# phrase glued up (вобщем, всмысле), -то/-нибудь/-либо without the hyphen.
GLUED = {"вобщем", "вообщем", "впринципе", "всмысле", "впорядке", "вообщето", "тоесть",
         "чтоли", "какбы", "ачем", "тото", "втом"}
HYPHENLESS = re.compile(
    "(кто|что|где|когда|как|куда|откуда|почему|зачем|чей|чья|чье|чьё|чьи|какой|какая|какое|"
    "какие|какого|какому|каким|каком|какую|кого|кому|кем|ком|чего|чему|чем|чём)(то|нибудь|либо)")
verb_forms = set()
if args.forms:
    with open(args.forms, encoding="utf-8") as f:
        for line in f:
            form, lemma = line.rstrip("\n").split("\t")
            if lemma.endswith(("ть", "ти", "чь", "ться", "тись", "чься")):
                verb_forms.add(form)


def glued(w):
    return (w in GLUED or HYPHENLESS.fullmatch(w) is not None
            or (w.startswith("не") and w[2:] in verb_forms))


if args.words:
    with open(args.words, encoding="utf-8") as f:
        allowed = {line.split("\t", 1)[0].strip() for line in f}
    if args.cyrillic_words_only:
        allowed = {w for w in allowed if re.search("[а-яё]", w)}
    for words, _ in lists:
        allowed.update(words)
    counts = {
        w: c for w, c in counts.items()
        if w in allowed
        or (c >= args.keep_frequent and not ARTIFACT.search(w) and not glued(w)
            and not (args.latin_lists_only and not re.search("[а-яё]", w)))
    }

for path in args.extra:
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or "\t" not in line:
                continue
            w, c = line.rstrip("\n").split("\t")[:2]
            if WORD.fullmatch(w):
                counts[w] = max(counts.get(w, 0), int(c))

for words, floor in lists:
    for w in words:
        c = raw.get(w) or floor
        if c > counts.get(w, 0):
            counts[w] = c

if args.forms:
    lemma_top = {}
    with open(args.forms, encoding="utf-8") as f:
        for line in f:
            form, lemma = line.rstrip("\n").split("\t")
            lemma_top[lemma] = max(lemma_top.get(lemma, 0), raw.get(form, 0))
    added = 0
    with open(args.forms, encoding="utf-8") as f:
        for line in f:
            form, lemma = line.rstrip("\n").split("\t")
            c = raw.get(form) or max(1, round(args.form_share * lemma_top[lemma]))
            if c > counts.get(form, 0):
                added += form not in counts
                counts[form] = c
    print(f"--forms: {added} new forms")

if args.yo_variants:
    for w, c in list(counts.items()):
        if "ё" in w:
            e = w.replace("ё", "е")
            counts[e] = max(counts.get(e, 0), c)

with open(args.out, "w", encoding="utf-8") as f:
    for w in sorted(counts):
        f.write(f"{w}\t{counts[w]}\n")
print(f"wrote {args.out}: {len(counts)} words")
