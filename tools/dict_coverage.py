#!/usr/bin/env python3
"""Measure a local reference word list against data/lexicon.tsv.

Usage: tools/dict_coverage.py data/private/reference.txt [--out data/private/missing.txt]

The reference may be a plain word list, a TSV whose first column is the word, or
Kaikki-style JSONL(.gz). It is read locally only. Keep proprietary references in
data/private (ignored by git), and only where their terms permit this analysis.
No reference data is included in the lexicon, the APK, or git by this tool.
"""
import argparse
import gzip
import json


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("reference")
    ap.add_argument("--lexicon", default="data/lexicon.tsv")
    ap.add_argument("--out", help="optional local list of missing words")
    args = ap.parse_args()

    with open(args.lexicon, encoding="utf-8") as f:
        known = {line.split("\t", 1)[0] for line in f}

    opener = gzip.open if args.reference.endswith(".gz") else open
    reference = set()
    with opener(args.reference, "rt", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("{"):
                word = json.loads(line).get("word", "")
            else:
                word = line.split("\t", 1)[0].split(" ", 1)[0]
            word = word.lower()
            if word:
                reference.add(word)
    missing = sorted(reference - known)
    covered = len(reference) - len(missing)
    share = 100.0 * covered / len(reference) if reference else 100.0
    print(f"reference: {len(reference)} unique words; covered: {covered} ({share:.2f}%); missing: {len(missing)}")
    if args.out:
        with open(args.out, "w", encoding="utf-8") as out:
            out.write("\n".join(missing) + ("\n" if missing else ""))
        print(f"missing words written locally to {args.out}")


if __name__ == "__main__":
    main()
