#!/usr/bin/env python3
"""Expand the Russian Hunspell dictionary (LibreOffice ru_RU) into every word
form, one per line — a second morphological source next to OpenCorpora.

Usage: tools/hunspell_forms.py DIR_WITH_ru_RU.dic [UNMUNCH] > data/hunspell_forms.txt
UNMUNCH defaults to `unmunch` on PATH (hunspell tools).
"""
import re
import subprocess
import sys

d = sys.argv[1]
unmunch = sys.argv[2] if len(sys.argv) > 2 else "unmunch"
out = subprocess.run([unmunch, f"{d}/ru_RU.dic", f"{d}/ru_RU.aff"], capture_output=True, check=True).stdout
word = re.compile(r"[а-яё]+(-[а-яё]+)*")
forms = set()
for raw in out.splitlines():
    line = raw.decode("koi8-r", "replace").strip()
    if line and not line.startswith("parsing"):
        w = line.split("/")[0].lower()
        if word.fullmatch(w):
            forms.add(w)
sys.stdout.write("".join(f"{w}\n" for w in sorted(forms)))
print(f"{len(forms)} forms", file=sys.stderr)
