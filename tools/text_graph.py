#!/usr/bin/env python3
"""The graph of a text: every reading of every word, every link the grammar
allows between them with its chance, every antecedent a pronoun may have —
all readings of the text in one graph, none thrown away, the likely first.

Usage: tools/text_graph.py [--parser data/graph/parser.pt] [--links data/graph/links.tsv]
                           [--json out.json] < text.txt

Words: the keyboard's readings (lemma + grammemes; «стали» a noun in five
cases and a verb). Links: the grammar's (tools/graph_grammar.py — the true
parse is in the graph for 99.8% of sentences), weighed by the student parser
(tools/graph_parser.py): its chance that a word hangs on each other word,
spread over the relations the grammar allows there. Pronouns: every earlier
noun or pronoun of the text that can be their antecedent by gender, number
and animacy, with equal chances (the chooser to weigh them comes next).
"""
import argparse
import json
import math
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("--parser", default=f"{ROOT}/data/graph/parser.pt")
ap.add_argument("--links", default=f"{ROOT}/data/graph/links.tsv")
ap.add_argument("--json")
ap.add_argument("--top", type=int, default=3, help="links shown per word")
opts = ap.parse_args()

# The parser's definitions, without its training (tools/graph_parser.py).
src = open(f"{ROOT}/tools/graph_parser.py", encoding="utf-8").read().split("\ntrain = read(")[0]
sys.argv = ["graph_parser.py"]
g = {"__file__": f"{ROOT}/tools/graph_parser.py", "__name__": "graph_parser"}
exec(compile(src, "graph_parser.py", "exec"), g)
torch = g["torch"]
saved = torch.load(opts.parser, map_location="cpu")
g["RELS"].update(saved["rels"])
rel_name = {i: r for r, i in saved["rels"].items()}
# Unseen words' grammeme sets are numbered as they come: size the table first.
model = g["Parser"](g["args"].d, g["args"].layers)
state = saved["model"]
model.bag_rows = state["bag_rows"]
model.load_state_dict(state)
model.eval()
# The bag table the model was saved with numbers the training words' sets;
# rebuild the word → set number map in the same order.
bag_rows = state["bag_rows"].numpy()
key_of_row = {tuple(sorted(int(i) for i in (row > 0).nonzero()[0])): n for n, row in enumerate(bag_rows)}

sets, word_set, G = g["sets"], g["word_set"], g["G"]


def bag_id(word):
    key = tuple(sorted({G[x] for t in sets.get(word_set.get(word, -1), []) for x in t.split(",")}))
    return key_of_row.get(key, 0)


CASES = {"nomn", "gent", "datv", "accs", "ablt", "loct", "voct", "gen2", "acc2", "loc2"}


def sig(tag):
    parts = tag.split(",")
    return (parts[0], next((p for p in parts if p in CASES), "-"))


allowed = {}
with open(opts.links, encoding="utf-8") as f:
    for line in f:
        a, b, r, d, k, c = line.rstrip("\n").split("\t")
        key = (tuple(a.split(",")), "ROOT" if b == "ROOT" else tuple(b.split(",")), int(d), int(k))
        allowed.setdefault(key, set()).add(r)


def dist(d):
    d = abs(d)
    return 1 if d == 1 else 2 if d == 2 else 3 if d <= 5 else 4


def readings(word):
    return sets.get(word_set.get(word, -1), [])


def sentence_graph(words):
    L = g["L"]
    words = words[: L - 1]
    n = len(words)
    lem = torch.full((1, L), g["UNK"], dtype=torch.long)
    cls = torch.zeros((1, L), dtype=torch.long)
    bags = torch.zeros((1, L), dtype=torch.long)
    mask = torch.ones((1, L), dtype=torch.bool)
    mask[0, 0] = False
    for j, w in enumerate(words):
        lem[0, j + 1] = g["lemma_of"].get(w, g["UNK"])
        cls[0, j + 1] = g["class_of"].get(w, 0)
        bags[0, j + 1] = bag_id(w)
        mask[0, j + 1] = False
    with torch.no_grad():
        s, h = model(lem, cls, bags, mask)
        p = torch.softmax(s[0], dim=-1)
    out = []
    sigs = [sorted({sig(t) for t in readings(w)}) or [("UNK", "-")] for w in words]
    for i, w in enumerate(words):
        links = []
        for j in range(n + 1):  # 0: the root
            if j == i + 1:
                continue
            if j == 0:
                rs = set().union(*[allowed.get((a, "ROOT", 0, 0), set()) for a in sigs[i]])
            else:
                d = 1 if j - 1 > i else -1
                rs = set().union(*[allowed.get((a, b, d, dist(j - 1 - i)), set())
                                   for a in sigs[i] for b in sigs[j - 1]])
            if not rs:
                continue
            with torch.no_grad():
                hj = h[0, j].unsqueeze(0)
                rl = torch.softmax(model.rel(torch.cat([h[0, i + 1].unsqueeze(0), hj], -1))[0], -1)
            rel_p = sorted(((float(rl[saved["rels"][r]]), r) for r in rs if r in saved["rels"]), reverse=True)
            links.append({"head": j, "p": float(p[i + 1, j]), "rels": [[r, round(q, 3)] for q, r in rel_p[:3]]})
        links.sort(key=lambda x: -x["p"])
        out.append({"word": w, "readings": readings(w), "links": links})
    return out


PRON = {"он": ("masc", "sing"), "его": ("masc", "sing"), "ему": ("masc", "sing"), "им": ("masc", "sing"),
        "нем": ("masc", "sing"), "него": ("masc", "sing"), "она": ("femn", "sing"), "ее": ("femn", "sing"),
        "её": ("femn", "sing"), "ей": ("femn", "sing"), "ней": ("femn", "sing"), "нее": ("femn", "sing"),
        "они": (None, "plur"), "их": (None, "plur"), "ими": (None, "plur")}


def antecedents(text_words, at, gender, number):
    """Earlier nouns (animate) the pronoun can stand for: equal chances."""
    found = []
    for k in range(at - 1, -1, -1):
        s_i, w_i, w = text_words[k]
        for t in readings(w):
            parts = t.split(",")
            if parts[0] == "NOUN" and "anim" in parts and number in parts and (gender is None or gender in parts):
                found.append((s_i, w_i, w))
                break
    return found


text = [re.findall(r"[а-яё]+(?:-[а-яё]+)*", line.lower().replace("ё", "е")) for line in sys.stdin]
text = [s for s in text if s]
graph = {"sentences": [], "coref": []}
flat = []
for si, words in enumerate(text):
    graph["sentences"].append(sentence_graph(words))
    for wi, w in enumerate(words):
        if w in PRON:
            gen, num = PRON[w]
            cands = antecedents(flat, len(flat), gen, num)
            if cands:
                graph["coref"].append({"sentence": si, "word": wi, "pronoun": w,
                                       "candidates": [{"sentence": a, "word": b, "form": c,
                                                       "p": round(1 / len(cands), 3)} for a, b, c in cands]})
        flat.append((si, wi, w))

for si, sent in enumerate(graph["sentences"]):
    words = [x["word"] for x in sent]
    print(" ".join(words))
    for i, x in enumerate(sent):
        alts = []
        for l in x["links"][: opts.top]:
            head = "ROOT" if l["head"] == 0 else words[l["head"] - 1]
            rel = l["rels"][0][0] if l["rels"] else "?"
            alts.append(f"{head}:{rel} {l['p']:.2f}")
        more = len(x["links"]) - opts.top
        print(f"  {x['word']:<12} → " + " | ".join(alts) + (f"  (+{more} more)" if more > 0 else ""))
for c in graph["coref"]:
    cands = ", ".join(f"{x['form']} ({x['p']:.2f})" for x in c["candidates"])
    print(f"  «{c['pronoun']}» (предл. {c['sentence'] + 1}) → {cands}")
if opts.json:
    json.dump(graph, open(opts.json, "w", encoding="utf-8"), ensure_ascii=False)
