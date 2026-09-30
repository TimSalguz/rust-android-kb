#!/usr/bin/env python3
"""The marks before each word of a parse (data/parse format), from the
sentence as written: adds column 7 — for each word the marks between it and
the word before (`_` none; `,` `—` `:` `;` `(` `)` `«` `»` `"` `.` `!` `?` `…`
as they stand) — and column 8, the marks after the last word. Sentences with
the dumps' debris («( )», «//», `*`, `<`, `~`, `[`, a space before a closing
mark) are left out.

Usage: tools/parse_marks.py PARSE.tsv SELECTION.tsv OUT.tsv
SELECTION: id<TAB>as written<TAB>words (the parse's own selection).
"""
import re
import sys

WORD = re.compile(r"[а-яё]+(?:-[а-яё]+)*", re.I)
MARK = re.compile(r"[,—–:;()«»\"„“”.!?…-]")
DEBRIS = re.compile(r"\(\s*\)|//|[*<>~\[\]{}|=#§&]| [),.;:!?]")

parse_path, sel_path, out_path = sys.argv[1:4]
written = {}
for line in open(sel_path, encoding="utf-8"):
    f = line.rstrip("\n").split("\t")
    if len(f) >= 2:
        written[f[0]] = f[1]


def marks(text, words):
    """Marks before each of `words` in `text`, and after the last; None if
    the words don't line up."""
    # Numbers go first: «1,5» is no comma between words.
    text = re.sub(r"\d+(?:[.,:]\d+)*", lambda m: " " * len(m.group(0)), text)
    found = list(WORD.finditer(text))
    got = [m.group(0).lower().replace("ё", "е") for m in found]
    if got != words:
        return None
    out, last = [], 0
    for m in found:
        between = "".join(MARK.findall(text[last:m.start()])).replace("–", "—").replace("„", "«") \
            .replace("“", "»").replace("”", "»")
        # A hyphen standing alone between words is a dash.
        between = between.replace("-", "—")
        out.append(between or "_")
        last = m.end()
    tail = "".join(MARK.findall(text[last:])).replace("–", "—") or "_"
    return out, tail


kept = debris = unaligned = 0
with open(out_path, "w", encoding="utf-8") as out:
    for line in open(parse_path, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        text = written.get(f[0])
        if text is None or DEBRIS.search(text):
            debris += 1
            continue
        m = marks(text, f[1].split())
        if m is None:
            unaligned += 1
            continue
        out.write("\t".join(f[:6] + [" ".join(m[0]), m[1]]) + "\n")
        kept += 1
print(f"{out_path}: {kept} kept, {debris} with debris, {unaligned} not lined up", file=sys.stderr)
