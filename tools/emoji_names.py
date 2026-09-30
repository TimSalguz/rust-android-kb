#!/usr/bin/env python3
"""Emoji names and keywords for the search, from the Unicode CLDR annotations
(common/annotations/<lang>.xml and annotationsDerived — flags, sequences;
Unicode License v3): for each emoji of the
panel's list (crates/android/src/emoji.txt), its name ("tts") and keywords
in each of the keyboard's languages, lowercase, ё as е.

Usage: tools/emoji_names.py [--cldr DIR]
  --cldr: a folder with the XML files already there (else they are fetched).
Out: crates/android/src/emoji_names/<lang>.txt — `emoji<TAB>name|keyword|…`.
"""
import argparse
import html
import os
import re
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LANGS = ["ru", "en", "de", "fr", "es", "pt"]
URL = "https://raw.githubusercontent.com/unicode-org/cldr/main/common/{}/{}.xml"
ap = argparse.ArgumentParser()
ap.add_argument("--cldr")
opts = ap.parse_args()

listed = []
for line in open(os.path.join(ROOT, "crates/android/src/emoji.txt"), encoding="utf-8"):
    if "\t" in line and not line.startswith("#"):
        listed.append(line.rstrip("\n").split("\t", 1)[1])
FE0F = "️"
ANN = re.compile(r'<annotation cp="([^"]+)"( type="tts")?>([^<]*)</annotation>')


def norm(s):
    return html.unescape(s).strip().lower().replace("ё", "е")


out_dir = os.path.join(ROOT, "crates/android/src/emoji_names")
os.makedirs(out_dir, exist_ok=True)
for lang in LANGS:
    xml = ""
    for part in ("annotations", "annotationsDerived"):
        if opts.cldr:
            xml += open(os.path.join(opts.cldr, part, f"{lang}.xml"), encoding="utf-8").read()
        else:
            req = urllib.request.Request(URL.format(part, lang),
                                         headers={"User-Agent": "rust-android-kb tools/emoji_names.py"})
            xml += urllib.request.urlopen(req, timeout=60).read().decode("utf-8")
    names, words = {}, {}
    for cp, tts, text in ANN.findall(xml):
        key = cp.replace(FE0F, "")
        if tts:
            names[key] = norm(text)
        else:
            words[key] = [norm(w) for w in text.split("|") if norm(w)]
    n = 0
    with open(os.path.join(out_dir, f"{lang}.txt"), "w", encoding="utf-8") as f:
        for e in listed:
            key = e.replace(FE0F, "")
            name = names.get(key)
            if not name:
                continue
            kws = [w for w in words.get(key, []) if w != name]
            f.write(e + "\t" + "|".join([name] + kws) + "\n")
            n += 1
    print(f"{lang}: {n} of {len(listed)} emoji named")
