#!/usr/bin/env python3
"""Sentences of two clauses, punctuated as they should be, from the parser's
everyday sentences (a selection of Tatoeba, kept there without commas): a
clause a conjunction opens first or last («Хотя было поздно, мы …», «…,
если …»), a cause («…, потому что …»), two clauses joined by «но», «а» or
«и», what someone says or thinks («Я думаю, что …»); and short clauses
with no subject of their own («было поздно», «стало темно») among them. The
teacher (Stanza) parses them (~/.cache/claude-parse/parse.py), the marks
are read off (tools/parse_marks.py), and the parser — its graph and its
comma head — learns where one clause ends and the next begins in text
typed without commas.

Usage: tools/clause_augment.py SEL_TRAIN.tsv OUT_PREFIX [N]
Out: OUT_PREFIX_train.tsv and OUT_PREFIX_valid.tsv (one in twenty), in the
selection's form (id<TAB>as written<TAB>words); ids `clause:<n>`.
"""
import random
import re
import sys

src, prefix = sys.argv[1], sys.argv[2]
n_out = int(sys.argv[3]) if len(sys.argv) > 3 else 40000
rng = random.Random(11)
ROOT = __file__.rsplit("/tools/", 1)[0]
proper = {line.split("\t", 1)[0] for line in open(f"{ROOT}/data/proper.tsv", encoding="utf-8")}

IMPERSONAL = ["было поздно", "было холодно", "стало темно", "было жарко", "было скучно",
              "шёл дождь", "светило солнце", "дул ветер", "никого не было дома", "было уже темно",
              "стало холодно", "было весело", "было тихо", "пошёл снег", "наступила ночь",
              "было очень рано", "стало совсем светло", "было тепло", "прошёл час", "кончился урок"]
SAYING = ["Я думаю", "Он сказал", "Она сказала", "Мне кажется", "Я знаю", "Все знают", "Я уверен",
          "Я надеюсь", "Мы поняли", "Он ответил", "Я слышал", "Говорят", "Оказалось", "Я боюсь",
          "Мама сказала", "Он думает", "Я не знал", "Она поняла"]
FIRST = ["Хотя", "Когда", "Если", "Пока", "Как только", "После того как", "Раз", "Поскольку"]
LAST = ["хотя", "когда", "если", "пока", "потому что", "так как", "как только", "чтобы"]
JOIN = ["но", "а", "и", "однако"]

base = []
for line in open(src, encoding="utf-8"):
    f = line.rstrip("\n").split("\t")
    # One statement each, no question: a clause to put in another.
    if len(f) == 3 and 2 <= len(f[2].split()) <= 7 and f[1][-1:] == "." and not re.search(r"[.!?…] ", f[1]):
        # Not one that opens with a joining word itself («Но никто …»).
        if f[2].split()[0] not in {"но", "а", "и", "да", "или", "как", "что", "ведь", "зато", "однако",
                                   "хотя", "когда", "если", "потому", "так", "чтобы", "пока"}:
            base.append(f[1][:-1])


def lower_first(s):
    first = s.split(" ", 1)[0].strip(".,!?")
    if first.lower().replace("ё", "е") in proper:
        return s
    return s[:1].lower() + s[1:]


def upper_first(s):
    return s[:1].upper() + s[1:]


def clause():
    return rng.choice(IMPERSONAL) if rng.random() < 0.15 else lower_first(rng.choice(base))


def make():
    k = rng.random()
    a, b = clause(), clause()
    if k < 0.3:
        return f"{rng.choice(FIRST)} {a}, {b}."
    if k < 0.55:
        return f"{upper_first(a)}, {rng.choice(LAST)} {b}."
    if k < 0.8:
        return f"{upper_first(a)}, {rng.choice(JOIN)} {b}."
    return f"{rng.choice(SAYING)}, что {b}."


seen = set()
rows = []
while len(rows) < n_out:
    text = make()
    if text in seen:
        continue
    seen.add(text)
    words = re.findall(r"[а-я]+(?:-[а-я]+)*", text.lower().replace("ё", "е"))
    if len(words) <= 20:
        rows.append((text, words))
with open(f"{prefix}_train.tsv", "w", encoding="utf-8") as tr, \
        open(f"{prefix}_valid.tsv", "w", encoding="utf-8") as va:
    for k, (text, words) in enumerate(rows):
        (va if k % 20 == 0 else tr).write(f"clause:{k}\t{text}\t{' '.join(words)}\n")
print(f"{len(rows)} sentences from {len(base)}", file=sys.stderr)
