#!/usr/bin/env python3
"""Everyday sentences as they are said in a chat: the short sentences the
parser learns from (a selection of Tatoeba, written with no commas) with
what speech puts before or after them, punctuated as it should be — an
address («Мама, я уже дома.», «Иди сюда, Саша.»), an interjection («Ой,
…», «Блин, …»), an aside («Слушай, …», «Кстати, …», «Честно говоря, …»),
a yes or no («Да, конечно, …»), and short lines of their own («Привет,
Саша!», «Спасибо, дорогая.», «Угу, да, конечно.»). The teacher (Stanza)
parses them as it parsed the rest (~/.cache/claude-parse/parse.py), the
marks are read off (tools/parse_marks.py), and the parser learns that such
words stand apart — with their commas and without them.

Usage: tools/chat_augment.py SEL_TRAIN.tsv OUT_PREFIX [N]
SEL_TRAIN: id<TAB>as written<TAB>words. Out: OUT_PREFIX_train.tsv and
OUT_PREFIX_valid.tsv (one in twenty), in the same form; ids `chat:<n>`.
"""
import random
import re
import sys

src, prefix = sys.argv[1], sys.argv[2]
n_out = int(sys.argv[3]) if len(sys.argv) > 3 else 40000
rng = random.Random(7)

NAMES = ["Саша", "Маша", "Лена", "Дима", "Коля", "Оля", "Аня", "Серёжа", "Андрей", "Катя", "Миша",
         "Наташа", "Паша", "Юля", "Вова", "Таня", "Игорь", "Света", "Настя", "Женя"]
KIN = ["мама", "папа", "бабушка", "дедушка", "сынок", "доченька", "друг", "дружище", "ребята",
       "коллеги", "дорогая", "дорогой", "милая", "милый", "брат", "сестрёнка", "солнце", "малыш",
       "мам", "пап"]
INTERJ = ["Ой", "Ах", "Эх", "Ну", "Блин", "Ага", "Угу", "Эй", "О", "Ого", "Ура", "Хм", "Ох", "Фу"]
ASIDES = ["Слушай", "Смотри", "Знаешь", "Кстати", "Короче", "Конечно", "Наверное", "Кажется",
          "Честно говоря", "В общем", "Может быть", "К сожалению", "Во-первых", "Кроме того",
          "Главное", "Представляешь", "Понимаешь", "Видишь ли", "Между прочим", "Правда"]
YESNO = ["Да", "Нет", "Ну да", "Да нет", "Конечно", "Ладно", "Хорошо", "Окей"]
LINES = ["Привет, {a}!", "Пока, {a}!", "Спасибо, {a}.", "Спасибо, {a}!", "Доброе утро, {a}!",
         "Спокойной ночи, {a}.", "С днём рождения, {a}!", "Прости, {a}.", "Извини, {a}.",
         "Молодец, {a}!", "Держись, {a}.", "Удачи, {a}!", "Хорошо, {a}.", "Ладно, {a}, пока.",
         "Угу, да, конечно.", "Ага, понял.", "Ну, ладно.", "Да, конечно.", "Нет, спасибо.",
         "Ой, всё.", "Эх, жаль.", "Ну, пока.", "Ага, спасибо.", "Да, да, помню.",
         "Нет, не надо.", "Ну, как хочешь.", "Слушай, а ты где?", "Ой, извини!"]

ROOT = __file__.rsplit("/tools/", 1)[0]
# Words written with a capital (names, places): they keep it mid-sentence.
proper = {line.split("\t", 1)[0] for line in open(f"{ROOT}/data/proper.tsv", encoding="utf-8")}
base = []
for line in open(src, encoding="utf-8"):
    f = line.rstrip("\n").split("\t")
    # One sentence each: no mark ending one inside.
    if len(f) == 3 and 3 <= len(f[2].split()) <= 9 and not re.search(r"[.!?…] ", f[1]):
        base.append(f[1])


def address():
    return rng.choice(NAMES) if rng.random() < 0.6 else rng.choice(KIN)


def lower_first(s):
    # Names and places keep their capital; «Я» mid-sentence is «я».
    first = s.split(" ", 1)[0].strip(".,!?")
    if first in NAMES or first.lower().replace("ё", "е") in proper:
        return s
    return s[:1].lower() + s[1:]


def upper_first(s):
    return s[:1].upper() + s[1:]


def make():
    kind = rng.random()
    if kind < 0.12:
        a = address()
        return rng.choice(LINES).format(a=a if a in NAMES else a)
    s = rng.choice(base)
    body, end = s[:-1], s[-1]
    if end not in ".!?":
        body, end = s, "."
    if kind < 0.35:  # an address first
        return f"{upper_first(address())}, {lower_first(body)}{end}"
    if kind < 0.47:  # an address last
        return f"{body}, {address()}{end}"
    if kind < 0.62:  # an interjection
        return f"{rng.choice(INTERJ)}, {lower_first(body)}{end}"
    if kind < 0.82:  # an aside
        return f"{rng.choice(ASIDES)}, {lower_first(body)}{end}"
    if kind < 0.92:  # yes or no, and more
        more = f" {rng.choice(['конечно', 'наверное', 'кажется'])}," if rng.random() < 0.3 else ""
        return f"{rng.choice(YESNO)},{more} {lower_first(body)}{end}"
    # An aside and an address together.
    return f"{rng.choice(ASIDES + INTERJ)}, {address()}, {lower_first(body)}{end}"


seen = set()
rows = []
while len(rows) < n_out:
    text = make()
    if text in seen:
        continue
    seen.add(text)
    words = re.findall(r"[а-я]+(?:-[а-я]+)*", text.lower().replace("ё", "е"))
    if len(words) >= 2:
        rows.append((text, words))
with open(f"{prefix}_train.tsv", "w", encoding="utf-8") as tr, \
        open(f"{prefix}_valid.tsv", "w", encoding="utf-8") as va:
    for k, (text, words) in enumerate(rows):
        (va if k % 20 == 0 else tr).write(f"chat:{k}\t{text}\t{' '.join(words)}\n")
print(f"{len(rows)} sentences from {len(base)}", file=sys.stderr)
