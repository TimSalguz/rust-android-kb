#!/usr/bin/env python3
"""The emoji panel's list: Unicode's emoji-test.txt → crates/android/src/emoji.txt.

Usage: tools/emoji.py emoji-test.txt > crates/android/src/emoji.txt

Fully-qualified emoji in Unicode's order, without the skin-tone variants and
the components; a line `@group` starts each group, then `version<TAB>emoji`
(the Emoji version ×10, so the keyboard leaves out what the phone's font
can't draw). The file ships inside the library: nothing is read until the
panel opens, and only the page shown is drawn.
"""
import re
import sys

GROUPS = {
    "Smileys & Emotion": "smileys",
    "People & Body": "people",
    "Animals & Nature": "nature",
    "Food & Drink": "food",
    "Travel & Places": "travel",
    "Activities": "activities",
    "Objects": "objects",
    "Symbols": "symbols",
    "Flags": "flags",
}
LINE = re.compile(r"^[0-9A-F ]+;\s*fully-qualified\s*#\s*(\S+)\s+E(\d+)\.(\d+)\s+(.*)$")

group = None
print("# Unicode emoji-test.txt (Unicode License v3, https://www.unicode.org/terms_of_use.html): tools/emoji.py")
with open(sys.argv[1], encoding="utf-8") as f:
    for line in f:
        if line.startswith("# group:"):
            group = GROUPS.get(line.split(":", 1)[1].strip())
            if group:
                print(f"@{group}")
            continue
        m = LINE.match(line)
        if not m or not group or "skin tone" in m.group(4):
            continue
        print(f"{int(m.group(2)) * 10 + int(m.group(3))}\t{m.group(1)}")
