#!/usr/bin/env python3
"""The student parser (tools/graph_parser.py) for the phone: kbcore::parser.

Usage: tools/graph_parser_export.py data/graph/parser_causal.pt --causal --out parser.bin
       [--int8] [--check sentences.txt --dump check.tsv]

The blob (little-endian): `b"KBGP"`, u32 version (1: f32; 2: `--int8`, each
weight matrix and embedding table as a f32 scale per row, then its rows in
i8, padded to 4 bytes — a quarter of the size), then u32 d, layers,
heads, ff, lemma dims, classes, places (the root and the words), grammemes,
relations, causal (1: a word sees only the words before it; 2: and the waiting
word's relation, below); the grammemes'
names and the relations' names (u16 length + UTF-8 each); then f32 weights
in this order: lemma projection, class embedding, grammeme projection,
places, the root, each layer (norm, attention in and out, norm, feed-forward
in and out), the final norm, the dependent and head projections (two layers
each), the "later" vector (causal), the relation classifier (two layers),
and — causal field 2 — the waiting word's relation classifier (two layers:
how a word waiting hangs on the head still to come).

The grammemes are those of the keyboard's readings (tools/build_classes.py):
the phone takes a word's from its reading set (`Engine::readings`), all
readings together.

`--check`: for each line of the file, every word's chance of each head as
PyTorch computes it, for the `parser_check` example to compare.
"""
import argparse
import os
import struct
import sys

import numpy as np
import torch

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("model")
ap.add_argument("--causal", action="store_true")
ap.add_argument("--out", required=True)
ap.add_argument("--int8", action="store_true", help="matrices in i8, a scale per row")
ap.add_argument("--places", type=int, default=12, help="words it reads at most (as trained)")
ap.add_argument("--d", type=int, default=128)
ap.add_argument("--layers", type=int, default=4)
ap.add_argument("--check")
ap.add_argument("--dump")
opts = ap.parse_args()

src = open(f"{ROOT}/tools/graph_parser.py", encoding="utf-8").read().split("\ntrain = read(")[0]
sys.argv = ["graph_parser.py", "--places", str(opts.places), "--d", str(opts.d), "--layers", str(opts.layers)] \
    + (["--causal"] if opts.causal else [])
g = {"__file__": f"{ROOT}/tools/graph_parser.py", "__name__": "graph_parser"}
exec(compile(src, "graph_parser.py", "exec"), g)
saved = torch.load(opts.model, map_location="cpu")
g["RELS"].update(saved["rels"])
model = g["Parser"](g["args"].d, g["args"].layers)
model.bag_rows = saved["model"]["bag_rows"]
model.load_state_dict(saved["model"], strict=False)  # older models: no wait_rel
model.eval()
sd = saved["model"]
grammemes = saved["grammemes"]
rels = [r for r, _ in sorted(saved["rels"].items(), key=lambda x: x[1])]
d, layers, L = g["args"].d, g["args"].layers, g["L"]

names = ["lemma.weight", "lemma.bias", "cls.weight", "gram.weight", "gram.bias", "pos.weight", "root"]
for i in range(layers):
    e = f"enc.layers.{i}."
    names += [e + n for n in ("norm1.weight", "norm1.bias", "self_attn.in_proj_weight", "self_attn.in_proj_bias",
                              "self_attn.out_proj.weight", "self_attn.out_proj.bias", "norm2.weight", "norm2.bias",
                              "linear1.weight", "linear1.bias", "linear2.weight", "linear2.bias")]
names += ["norm.weight", "norm.bias", "dep.0.weight", "dep.0.bias", "dep.2.weight", "dep.2.bias",
          "hd.0.weight", "hd.0.bias", "hd.2.weight", "hd.2.bias", "later",
          "rel.0.weight", "rel.0.bias", "rel.2.weight", "rel.2.bias"]
reads_marks = "marks.weight" in sd
if reads_marks:
    names.insert(names.index("gram.bias") + 1, "marks.weight")
waiting = opts.causal and "wait_rel.0.weight" in sd
if waiting:
    names += ["wait_rel.0.weight", "wait_rel.0.bias", "wait_rel.2.weight", "wait_rel.2.bias"]


def text(s):
    b = s.encode("utf-8")
    return struct.pack("<H", len(b)) + b


with open(opts.out + ".part", "wb") as f:
    f.write(b"KBGP" + struct.pack("<11I", 2 if opts.int8 else 1, d, layers, 4, 2 * d, g["D"], g["N_CLASS"], L,
                                  len(grammemes), len(rels),
                                  (2 if waiting else int(opts.causal)) + (4 if reads_marks else 0)))
    for s in grammemes + rels:
        f.write(text(s))
    # Floats start 4-aligned.
    pad = (-f.tell()) % 4
    f.write(b"\0" * pad)
    n = 0
    for name in names:
        t = sd[name].detach().float().numpy()
        if opts.int8 and t.ndim == 2:
            # A scale per row: its largest weight is ±127.
            scale = np.maximum(np.abs(t).max(axis=1), 1e-12) / 127.0
            q = np.clip(np.rint(t / scale[:, None]), -127, 127).astype("i1")
            f.write(scale.astype("<f4").tobytes())
            f.write(q.tobytes())
            f.write(b"\0" * ((-q.size) % 4))
        else:
            f.write(t.astype("<f4").ravel().tobytes())
        n += t.size
os.replace(opts.out + ".part", opts.out)
print(f"wrote {opts.out}: {n} numbers, {os.path.getsize(opts.out) / 1e6:.2f} MB; "
      f"{len(grammemes)} grammemes, {len(rels)} relations")

if opts.check:
    with open(opts.check, encoding="utf-8") as f, open(opts.dump, "w", encoding="utf-8") as out:
        for line in f:
            words = line.split()[: L - 1]
            if not words:
                continue
            # A word may bring the marks before it: «,что» (a comma, then «что»).
            marks = [w[: len(w) - len(w.lstrip(",—:;()«»\"!?.…"))] or "_" for w in words]
            words = [w.lstrip(",—:;()«»\"!?.…") for w in words]
            n = len(words)
            t = [x.long() if x.dtype == torch.int32 else x for x in g["tensors"]([(words, [0] * n, [0] * n, marks)])]
            # The grammeme sets as numbered here (the saved table numbers the
            # training's).
            model.bag_rows = torch.tensor(g["np"].stack(g["bag_rows"]))
            with torch.no_grad():
                s, h = model(*t[:4], t[6])
                p = torch.softmax(s[0], -1)
                pick = p.argmax(-1).clamp(max=L - 1)
                rel = model.rel(torch.cat([h[0], h[0][pick]], -1)).argmax(-1)
            for i, w in enumerate(words):
                row = " ".join(f"{float(x):.5f}" for x in p[i + 1][: n + 1])
                later = f" {float(p[i + 1][-1]):.5f}" if opts.causal else ""
                out.write(f"{' '.join(m.replace('_', '') + x for m, x in zip(marks, words))}\t{i}\t{row}{later}"
                          f"\t{int(rel[i + 1])}\n")
    print(f"wrote {opts.dump}")
