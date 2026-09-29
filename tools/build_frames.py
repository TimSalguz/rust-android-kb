#!/usr/bin/env python3
"""Government frames and agreement weights for the phrase grammar
(kbcore::gram), learned from running text.

Usage: tools/build_frames.py SOURCE... [--word-readings data/word_readings.tsv]
           [--readings data/readings.tsv] [--min 100] > data/frames.tsv

Every word of a phrase (punctuation ends one) is read against the words
before it the way the keyboard reads them: back over the adjectives before
it (and «и» between them) to the word that governs them — a preposition, a
verb, a noun — past adverbs and particles. For each governing form seen at
least `--min` times: how much likelier than anywhere the next word is a noun
phrase in each case, or something else (a preposition, a conjunction, an
adverb) — `ln(P(outcome | governor) / P(outcome))`; and, after adjectives,
how much likelier the next word agrees with them than not. Kept when it
says something (a log ratio of at least 0.3 somewhere): the frames are the
rules, the rest is the default.

SOURCE: Tatoeba `*_sentences.tsv.bz2` (held-out ids skipped, as in
build_bigrams.py) or Leipzig `*.tar.gz`.

stdout: `governor<TAB>other,nomn,gent,datv,accs,ablt,loct,infn` (log ratios,
nats), and `@attr<TAB>other,agree,disagree,ln P(other),ln P(infn),fits,misfits`
for the words after adjectives (then how often something else and an
infinitive come anywhere, to tell a frame's cases apart among noun phrases
alone), and after a subject — a personal pronoun or a noun only in the
nominative, maybe adverbs between — how much likelier than by chance a
predicate (a verb, a short adjective or participle) fits it and doesn't.
And `%verb<TAB>…` (the same eight): what comes after a noun phrase in the
verb's clause («дал книгу [другу]», «помог ему [встать]») — the verb two or
more words back, where the word pairs don't see it.
"""
import argparse
import bz2
import math
import re
import sys
import tarfile
from collections import Counter, defaultdict

HOLDOUT = 50
STOCK = re.compile(r"\b(?:Том|Тома|Тому|Томом|Томе|Мэри|Бостон\w*)\b")
TOKEN = re.compile(r"[а-яё-]*[а-яё][а-яё-]*|\s+|.")
WORD = re.compile(r"[а-яё-]*[а-яё][а-яё-]*")
CASES = ["nomn", "gent", "datv", "accs", "ablt", "loct"]
FOLD = {"gen2": "gent", "acc2": "accs", "loc2": "loct", "voct": None}
ATTRIBUTE = {"ADJF", "PRTF"}
NOMINAL = {"NOUN", "ADJF", "PRTF", "NPRO", "NUMR"}
GENDERS = {"masc", "femn", "neut", "ms-f"}
ALPHA = 20.0  # smoothing toward the default, in cases
MIN_SAYS = 0.3

ap = argparse.ArgumentParser()
ap.add_argument("sources", nargs="+")
ap.add_argument("--word-readings", default="data/word_readings.tsv")
ap.add_argument("--readings", default="data/readings.tsv")
ap.add_argument("--min", type=int, default=100)
args = ap.parse_args()


PREDICATE = {"VERB", "ADJS", "PRTS"}
PRONOUNS = {
    "я": ("1per", "sing", None),
    "ты": ("2per", "sing", None),
    "он": ("3per", "sing", "masc"),
    "она": ("3per", "sing", "femn"),
    "оно": ("3per", "sing", "neut"),
    "мы": ("1per", "plur", None),
    "вы": ("2per", "plur", None),
    "они": ("3per", "plur", None),
}


class Reading:
    __slots__ = ("pos", "case", "number", "gender", "animacy", "person", "tense")

    def __init__(self, tag):
        parts = tag.split(",")
        self.pos = parts[0]
        self.case = self.number = self.gender = self.animacy = None
        self.person = self.tense = None
        for g in parts[1:]:
            if g in ("1per", "2per", "3per"):
                self.person = g
            if g in ("past", "pres", "futr"):
                self.tense = g
            if g in ("anim", "inan"):
                self.animacy = g
            if g in CASES:
                self.case = g
            elif g in FOLD:
                self.case = FOLD[g]
            elif g in ("sing", "plur"):
                self.number = g
            elif g in GENDERS:
                self.gender = g


sets = {}
with open(args.readings, encoding="utf-8") as f:
    for line in f:
        i, tags = line.rstrip("\n").split("\t")
        sets[i] = tuple(Reading(t) for t in tags.split("|"))
readings = {}
with open(args.word_readings, encoding="utf-8") as f:
    for line in f:
        w, i = line.rstrip("\n").split("\t")
        readings[w] = sets[i]


def attributive(rs):
    return any(r.pos in ATTRIBUTE and r.case for r in rs)


def strict(rs):
    return bool(rs) and all(r.pos in ATTRIBUTE for r in rs)


def nominal(rs):
    return bool(rs) and all(r.pos in NOMINAL and r.case for r in rs)


def agree(a, b):
    if not a.case or a.case != b.case or not a.number or a.number != b.number:
        return False
    if a.case == "accs" and a.animacy and b.animacy and a.animacy != b.animacy:
        return False
    if a.number == "plur" or not a.gender or not b.gender:
        return True
    return a.gender == b.gender or "ms-f" in (a.gender, b.gender)


def subject_noun(rs):
    """A noun only in the nominative as a subject: (person, number, gender)."""
    if not rs or not all(r.pos == "NOUN" and r.case == "nomn" for r in rs):
        return None
    numbers = {r.number for r in rs if r.number}
    genders = {r.gender for r in rs if r.gender}
    if len(numbers) != 1:
        return None
    return ("3per", numbers.pop(), genders.pop() if len(genders) == 1 else None)


def fits_subject(p, s):
    """A predicate reading fits its subject (as kbcore::gram reads it)."""
    person, number, gender = s
    if not p.number or p.number != number:
        return False
    gender_fits = p.number != "sing" or not gender or gender == "ms-f" or p.gender == gender
    if p.pos in ("ADJS", "PRTS"):
        return gender_fits
    if not p.tense:
        return person == "2per"
    if p.tense == "past":
        return gender_fits
    return not p.person or p.person == person


def adverbial(rs):
    return (bool(rs) and any(x.pos in ("ADVB", "PRCL") for x in rs)
            and all(x.pos in ("ADVB", "PRCL", "ADJS", "COMP", "PRED") for x in rs))


def subject_at(words, rs, k):
    """The subject at `k`, and with another joined to it by «и»/«или»
    («Эстелла и я») the two together: plural, the lesser person; None when
    that other one can't be told."""
    s = PRONOUNS.get(words[k]) or subject_noun(rs[k])
    if not s or k < 1 or words[k - 1] not in ("и", "или"):
        return s
    if k < 2:
        return None
    other = PRONOUNS.get(words[k - 2])
    if not other and any(r.pos in ("NOUN", "NPRO") and r.case == "nomn" for r in rs[k - 2]):
        other = ("3per", None, None)
    if not other:
        return None
    return (min(s[0], other[0]), "plur", None)


def subject(words, rs):
    """The subject right before the end, maybe adverbs between: (person,
    number, gender) or None."""
    for k in range(len(words) - 1, max(-1, len(words) - 6), -1):
        if PRONOUNS.get(words[k]) or subject_noun(rs[k]):
            return subject_at(words, rs, k)
        if not adverbial(rs[k]):
            return None
    return None


def walk(words, rs):
    """Back from the end: (governor index or None, the strict attributes)."""
    attrs = []
    collected = 0
    for k in range(len(words) - 1, max(-1, len(words) - 6), -1):
        w, r = words[k], rs[k]
        if w.startswith("котор"):
            return None, attrs
        if collected == len(words) - 1 - k and r and attributive(r):
            nxt = attrs[-1] if attrs else None
            as_attr = [x for x in r if x.pos in ATTRIBUTE]
            if nxt and not any(agree(x, y) for x in as_attr for y in nxt):
                return None, attrs
            if strict(r):
                attrs.append(r)
            collected += 1
            continue
        if collected and w in ("и", "или"):
            collected += 1
            continue
        if not collected and r and all(x.pos in ("ADVB", "PRCL") for x in r):
            collected += 1
            continue
        return k, attrs
    return None, attrs


VERBAL = {"VERB", "INFN", "GRND"}


def clause(words, rs, g):
    """The verb whose clause the noun phrase ending at `g` (the walk's
    governor, a noun or pronoun) stands in: back over noun phrases and
    adverbs, not past a preposition; its index or None."""
    r = rs[g]
    if not r or not all(x.pos in ("NOUN", "NPRO") and x.case for x in r):
        return None
    for k in range(g - 1, max(-1, g - 5), -1):
        rk = rs[k]
        if not rk:
            return None
        if all(x.pos in VERBAL for x in rk):
            return k
        if nominal(rk) or adverbial(rk):
            continue
        return None
    return None


def sentences(path):
    if path.endswith(".bz2"):
        with bz2.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) == 3 and p[0].isdigit() and int(p[0]) % HOLDOUT and not STOCK.search(p[2]):
                    yield p[2]
        return
    with tarfile.open(path) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(member):
            yield raw.decode("utf-8", "replace").split("\t", 1)[-1]


# Outcomes: 0 something else, 1… a noun phrase in CASES[i - 1] (fractional
# over the cases the word may be in), 7 an infinitive («могут выдержать»).
OUTCOMES = 8
base = [0.0] * OUTCOMES
gov = defaultdict(lambda: [0.0] * OUTCOMES)
attr = Counter()  # other / agree / disagree after adjectives, and in general
predicates = Counter()  # predicates anywhere, by reading set
# A verb's clause after a noun phrase (see `clause`): what comes next, by the
# verb. (By the case the phrase filled it says what the word pairs' classes
# already know: the next word's case after that one's.)
clause_verb = defaultdict(lambda: [0.0] * OUTCOMES)
clause_base = [0.0] * OUTCOMES
after_subject = Counter()  # (subject, fits) for a predicate after a subject
seen = 0
for path in args.sources:
    for text in sentences(path):
        phrase_w, phrase_r = [], []
        for tok in TOKEN.findall(text.lower()):
            if not WORD.fullmatch(tok):
                if tok.strip():
                    phrase_w, phrase_r = [], []
                continue
            rs = readings.get(tok)
            if phrase_w and rs:
                g, attrs = walk(phrase_w, phrase_r)
                if all(r.pos in PREDICATE for r in rs):
                    predicates[rs] += 1
                    subj = subject(phrase_w, phrase_r)
                    if subj:
                        after_subject[subj, any(fits_subject(r, subj) for r in rs)] += 1
                outcome = [0.0] * OUTCOMES
                if all(r.pos == "INFN" for r in rs):
                    outcome[7] = 1.0
                    if attrs:
                        attr["other"] += 1
                elif nominal(rs):
                    fits = [r for r in rs if all(any(agree(a, r) for a in at) for at in attrs)]
                    cases = {r.case for r in (fits or rs)}
                    for c in cases:
                        outcome[1 + CASES.index(c)] = 1 / len(cases)
                    if attrs:
                        attr["agree" if fits else "disagree"] += 1
                else:
                    outcome[0] = 1.0
                    if attrs:
                        attr["other"] += 1
                if attrs:
                    attr["n"] += 1
                for i in range(OUTCOMES):
                    base[i] += outcome[i]
                if g is not None:
                    row = gov[phrase_w[g]]
                    for i in range(OUTCOMES):
                        row[i] += outcome[i]
                    v = clause(phrase_w, phrase_r, g)
                    if v is not None:
                        for i in range(OUTCOMES):
                            clause_base[i] += outcome[i]
                            clause_verb[phrase_w[v]][i] += outcome[i]
                seen += 1
            phrase_w.append(tok)
            phrase_r.append(rs or ())
    print(f"{path}: {seen} words read", file=sys.stderr)

total = sum(base)
p_base = [b / total for b in base]
kept = 0
for g, row in sorted(gov.items()):
    n = sum(row)
    if n < args.min:
        continue
    lr = [math.log((row[i] + ALPHA * p_base[i]) / (n + ALPHA) / p_base[i]) for i in range(OUTCOMES)]
    if max(abs(x) for x in lr) < MIN_SAYS:
        continue
    print(g + "\t" + ",".join(f"{x:.3f}" for x in lr))
    kept += 1
# After a subject: predicates that fit it and don't, against a predicate
# drawn from anywhere (how often that one would fit the same subject).
n_pred = sum(predicates.values()) or 1
chance = {}
fit = misfit = expected = 0.0
for (subj, ok), k in after_subject.items():
    if subj not in chance:
        chance[subj] = sum(c for rs, c in predicates.items()
                           if any(fits_subject(r, subj) for r in rs)) / n_pred
    expected += k * chance[subj]
    fit += k * ok
    misfit += k * (not ok)
n_subj = fit + misfit
# After adjectives: other words, agreeing ones and not, against their share
# anywhere (agreeing: any noun phrase; not: all but never, so a floor).
n = attr["n"] or 1
p_other = p_base[0] + p_base[7]
p_nominal = 1 - p_other
print("@attr\t" + ",".join(f"{x:.3f}" for x in (
    math.log((attr["other"] + 1) / n / p_other),
    math.log((attr["agree"] + 1) / n / p_nominal),
    math.log((attr["disagree"] + 1) / n / p_nominal),
    math.log(p_base[0]),
    math.log(p_base[7]),
    math.log((fit + 1) / (expected + 1)),
    math.log((misfit + 1) / (n_subj - expected + 1)),
)))
# A verb's clause after a noun phrase (`%verb`), against any clause after one.
n_cl = sum(clause_base) or 1
p_cl = [(b + 1) / (n_cl + OUTCOMES) for b in clause_base]
kept_cl = 0
for v, row in sorted(clause_verb.items()):
    n = sum(row)
    if n < args.min:
        continue
    lr = [math.log((row[i] + ALPHA * p_cl[i]) / (n + ALPHA) / p_cl[i]) for i in range(OUTCOMES)]
    if max(abs(x) for x in lr) < MIN_SAYS:
        continue
    print("%" + v + "\t" + ",".join(f"{x:.3f}" for x in lr))
    kept_cl += 1
print(f"clauses after a noun phrase {n_cl:.0f}: {kept_cl} verbs of {len(clause_verb)}", file=sys.stderr)
print(f"after a subject {n_subj:.0f} predicates: {fit:.0f} fit, {misfit:.0f} don't "
      f"({expected:.0f} would by chance)", file=sys.stderr)
print(f"{kept} governors of {len(gov)}; after adjectives {attr['n']} words: "
      f"{attr['other']} other, {attr['agree']} agree, {attr['disagree']} don't", file=sys.stderr)
