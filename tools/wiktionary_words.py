#!/usr/bin/env python3
"""Export single words and forms from a Kaikki/wiktextract JSONL.gz dictionary.

Usage: tools/wiktionary_words.py data/wiktionary/kaikki-Russian.jsonl.gz \
           --lang ru -o data/wiktionary_ru.txt

The output is a plain word list for tools/make_lexicon.py --wordlist. Only open
Wiktionary data is accepted here; proprietary dictionary dumps are not a build
input. Parsing is streaming, so the source never has to be inflated in memory.
"""
import argparse
import gzip
import json
import re
import sys
import unicodedata

WORD = re.compile(r"[a-zß-öø-ÿœа-яё'-]*[a-zß-öø-ÿœа-яё][a-zß-öø-ÿœа-яё'-]*")
SKIP_POS = {"prefix", "suffix", "interfix", "infix", "circumfix", "character", "symbol",
            "punct", "phrase", "proverb", "prep_phrase"}
BAD_TAGS = {"misspelling", "obsolete", "archaic", "romanization", "error-unrecognized-form"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("source")
    ap.add_argument("--lang", choices=["ru", "en"], required=True)
    ap.add_argument("-o", "--out", required=True)
    args = ap.parse_args()

    def clean(value):
        word = unicodedata.normalize("NFC", value.strip().lower())
        if not WORD.fullmatch(word):
            return None
        if args.lang == "ru" and not re.search("[а-яё]", word):
            return None
        if args.lang == "en" and re.search("[^a-z'-]", word):
            return None
        return word

    words = set()
    entries = forms = skipped = 0
    with gzip.open(args.source, "rt", encoding="utf-8") as src:
        for line in src:
            entry = json.loads(line)
            if entry.get("pos", "") in SKIP_POS:
                continue
            senses = entry.get("senses") or []
            bad = any("misspelling" in (s.get("tags") or []) for s in senses)
            word = clean(entry.get("word", ""))
            if word and not bad:
                words.add(word)
                entries += 1
            else:
                skipped += 1
            for form in entry.get("forms") or []:
                tags = set(form.get("tags") or [])
                if tags & BAD_TAGS:
                    continue
                word = clean(form.get("form", ""))
                if word and not bad:
                    words.add(word)
                    forms += 1
    with open(args.out, "w", encoding="utf-8") as out:
        for word in sorted(words):
            out.write(word + "\n")
    print(f"{args.out}: {len(words)} words ({entries} entries, {forms} forms; {skipped} skipped)",
          file=sys.stderr)


if __name__ == "__main__":
    main()
