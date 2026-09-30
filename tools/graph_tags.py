#!/usr/bin/env python3
"""The tags the rules of tools/graph_marks.py read (UD part of speech and
features), from the keyboard's own structures — each word's readings in the
dictionary (data/word_readings.tsv, data/readings.tsv), the one its link in
the graph calls for — not from a tagger's (the phone has none).

Usage: tools/graph_tags.py PARSES.tsv OUT.tsv
Replaces columns 3 (part of speech) and 6 (features) of each row; the rest
(words, heads, relations, marks, the graph's chances) stays.
"""
import sys

JOINING = {"и", "или", "а", "но", "да", "либо", "ни", "однако", "зато", "иль"}
SUBORDINATING = {"что", "чтобы", "если", "когда", "потому", "так", "как", "хотя", "пока", "раз",
                 "будто", "словно", "ибо", "ежели", "дабы", "поскольку", "чем", "лишь", "едва"}
CASES = {"nomn": "Nom", "gent": "Gen", "gen2": "Gen", "datv": "Dat", "accs": "Acc", "acc2": "Acc",
         "ablt": "Ins", "loct": "Loc", "loc2": "Loc", "voct": "Voc"}

ROOT = __import__("os").path.dirname(__import__("os").path.dirname(__import__("os").path.abspath(__file__)))
sets = {}
for line in open(f"{ROOT}/data/readings.tsv", encoding="utf-8"):
    i, tags = line.rstrip("\n").split("\t")
    sets[i] = [t.split(",") for t in tags.split("|")]
word_set = {}
for line in open(f"{ROOT}/data/word_readings.tsv", encoding="utf-8"):
    w, i = line.rstrip("\n").split("\t")
    word_set[w] = i


def tag(word, rel):
    """The reading the link calls for, as UD would tag it."""
    readings = sets.get(word_set.get(word, ""), [])
    pos = {r[0] for r in readings}
    base = rel.split(":")[0]
    feats = {}

    def case_of(r):
        c = next((CASES[g] for g in r[1:] if g in CASES), None)
        if c:
            feats["Case"] = c

    if word in JOINING and base in ("cc", "conj", "advmod", "discourse", "mark") and "CONJ" in pos:
        return "CCONJ", feats
    if word in SUBORDINATING and base == "mark":
        return "SCONJ", feats
    if "GRND" in pos and base in ("advcl", "conj", "root", "parataxis", "acl"):
        return "VERB", {"VerbForm": "Conv"}
    if ("PRTF" in pos or "PRTS" in pos) and base in ("acl", "amod", "conj", "root", "advcl", "xcomp"):
        r = next(r for r in readings if r[0] in ("PRTF", "PRTS"))
        feats = {"VerbForm": "Part"}
        if r[0] == "PRTS":
            feats["Variant"] = "Short"
        case_of(r)
        return "VERB", feats
    if "INFN" in pos and base in ("xcomp", "csubj", "advcl", "acl", "ccomp", "root", "conj", "nsubj", "obj"):
        return "VERB", {"VerbForm": "Inf"}
    if base in ("cop", "aux") and "VERB" in pos:
        return "AUX", {"VerbForm": "Fin"}
    if "VERB" in pos and base in ("root", "conj", "advcl", "ccomp", "acl", "parataxis", "csubj", "xcomp"):
        return "VERB", {"VerbForm": "Fin"}
    # A pronoun's reading first: «нее» is «она», not the rare noun «нея».
    for want, upos in (("NPRO", "PRON"), ("NOUN", "NOUN"), ("ADJF", "ADJ"), ("ADJS", "ADJ"),
                       ("NUMR", "NUM"), ("ADVB", "ADV"), ("PREP", "ADP"), ("PRCL", "PART"),
                       ("INTJ", "INTJ"), ("PRED", "ADV"), ("COMP", "ADV"), ("CONJ", "CCONJ")):
        if want in pos:
            r = next(r for r in readings if r[0] == want)
            if want in ("NOUN", "NPRO", "ADJF", "NUMR"):
                # The nominative when the link wants it (a subject).
                noms = [x for x in readings if x[0] == want and "nomn" in x]
                case_of(noms[0] if base == "nsubj" and noms else r)
            if want == "ADJS":
                feats["Variant"] = "Short"
            return upos, feats
    if "VERB" in pos:
        return "VERB", {"VerbForm": "Fin"}
    if "GRND" in pos:
        return "VERB", {"VerbForm": "Conv"}
    return ("PROPN" if not readings else "X"), feats


def main():
    src, dst = sys.argv[1:3]
    with open(dst, "w", encoding="utf-8") as out:
        retag(src, out)


def retag(src, out):
    for line in open(src, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        words, rels = f[1].split(), f[4].split()
        if len(words) != len(rels):
            continue
        tagged = [tag(w, r) for w, r in zip(words, rels)]
        f[2] = " ".join(u for u, _ in tagged)
        f[5] = "|".join(";".join(f"{k}={v}" for k, v in fs.items()) or "_" for _, fs in tagged)
        out.write("\t".join(f) + "\n")


if __name__ == "__main__":
    main()
