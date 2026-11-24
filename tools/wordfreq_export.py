#!/usr/bin/env python3
"""Export an open wordfreq list as word<TAB>estimated-count for make_lexicon.py.

Usage: target/dict-venv/bin/python tools/wordfreq_export.py ru \
           --limit 500000 -o data/wordfreq_ru.tsv

Requires the build-only `wordfreq` package (not an application dependency).
Counts are occurrences per billion tokens in the source corpora. The lexicon
uses them only for words its existing sources do not already contain.
"""
import argparse
import re
import unicodedata

from wordfreq import top_n_list, zipf_frequency

WORD = re.compile(r"[a-zß-öø-ÿœа-яё'-]*[a-zß-öø-ÿœа-яё][a-zß-öø-ÿœа-яё'-]*")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lang", choices=["ru", "en"])
    ap.add_argument("--limit", type=int, default=500_000)
    ap.add_argument("-o", "--out", required=True)
    args = ap.parse_args()

    rows = {}
    for raw in top_n_list(args.lang, args.limit, wordlist="large"):
        word = unicodedata.normalize("NFC", raw.lower())
        if not WORD.fullmatch(word):
            continue
        if args.lang == "ru" and not re.search("[а-яё]", word):
            continue
        if args.lang == "en" and re.search("[^a-z'-]", word):
            continue
        count = max(1, round(10 ** zipf_frequency(word, args.lang, wordlist="large")))
        rows[word] = max(rows.get(word, 0), count)
    with open(args.out, "w", encoding="utf-8") as out:
        for word in sorted(rows):
            out.write(f"{word}\t{rows[word]}\n")
    print(f"{args.out}: {len(rows)} words")


if __name__ == "__main__":
    main()
