#!/usr/bin/env python3
"""Swipes drawn by people, in the form `kbdemo --swipes` reads them.

Usage: tools/real_swipes.py DIR [--split valid] [--n N] [--seed S] > swipes.txt
       target/release/kbdemo --swipes swipes.txt   (DICT_FST=…, KB_PROFILE=phone)

DIR: data in the Yandex Cup 2023 NeuroSwipe layout — `valid.jsonl` (or
another split: one swipe a line, `{"word": …, "curve": {"x": […], "y": […],
"t": […], "grid_name": …}}`, or the grid itself under `grid`; the words may
instead be in `valid.ref`, a line each) and `gridname_to_grid.json` (each
layout's keys: label and hitbox) unless the grids come with the swipes. That data
comes with no licence to pass it on: it is kept outside this repository and
read only to measure how our decoder does on real swipes — nothing from it
is committed, tuned on or shipped.

stdout: `#key GRID CHAR X Y` (each letter key's center), `#width GRID W` (a
letter key's width, the median), then `WORD GRID x,y,t x,y,t …`.
"""
import argparse
import json
import os
import random
import statistics
import sys

ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("--split", default="valid")
ap.add_argument("--n", type=int, default=0, help="at most this many swipes (0: all)")
ap.add_argument("--seed", type=int, default=1)
args = ap.parse_args()


def letter_keys(grid):
    """(char, center x, center y) of the grid's single-letter keys, and the
    median width of a letter key."""
    keys, widths = [], []
    for k in grid.get("keys", []):
        label = k.get("label", "")
        box = k.get("hitbox") or {}
        if len(label) != 1 or not label.isalpha() or not box:
            continue
        x, y, w, h = box["x"], box["y"], box["w"], box["h"]
        keys.append((label.lower(), x + w / 2, y + h / 2))
        widths.append(w)
    return keys, (statistics.median(widths) if widths else 0)


grids = {}
path = os.path.join(args.dir, "gridname_to_grid.json")
if os.path.exists(path):
    with open(path, encoding="utf-8") as f:
        grids = json.load(f)

refs = None
ref_path = os.path.join(args.dir, f"{args.split}.ref")
if os.path.exists(ref_path):
    with open(ref_path, encoding="utf-8") as f:
        refs = [w.strip() for w in f]
lines = []
with open(os.path.join(args.dir, f"{args.split}.jsonl"), encoding="utf-8") as f:
    for i, line in enumerate(f):
        r = json.loads(line)
        word = r.get("word") or (refs[i] if refs and i < len(refs) else None)
        curve = r.get("curve", {})
        if not word or not curve.get("x"):
            continue
        name = curve.get("grid_name")
        if name is None and "grid" in curve:
            name = curve["grid"].get("grid_name", "inline")
            grids.setdefault(name, curve["grid"])
        lines.append((word, name, curve["x"], curve["y"], curve["t"]))
if args.n and len(lines) > args.n:
    random.Random(args.seed).shuffle(lines)
    lines = lines[: args.n]

used = {name for _, name, *_ in lines}
for name in sorted(used):
    if name not in grids:
        print(f"no layout `{name}`", file=sys.stderr)
        continue
    keys, width = letter_keys(grids[name])
    for c, x, y in keys:
        print(f"#key {name} {c} {x:.1f} {y:.1f}")
    print(f"#width {name} {width:.1f}")
for word, name, xs, ys, ts in lines:
    pts = " ".join(f"{x},{y},{t}" for x, y, t in zip(xs, ys, ts))
    print(f"{word} {name} {pts}")
print(f"{len(lines)} swipes, layouts {sorted(used)}", file=sys.stderr)
