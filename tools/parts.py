#!/usr/bin/env python3
"""The parts of a sentence — main clause, clauses a conjunction or «который»
opens, parts joined by «и/а/но», participle and gerund phrases, asides and
addresses — each word in one: from the teacher's tree (the reference) and
from the keyboard's own graph of the sentence typed with no mark
(crates/cli/examples/graph_dump.rs). How well our bounds of the parts
match the reference ones, and how many of the text's commas stand on a
bound (what the marks can be read off the parts for).

Usage: tools/parts.py GRAPH.tsv [--show N]
"""
import argparse

ap = argparse.ArgumentParser()
ap.add_argument("graph")
ap.add_argument("--show", type=int, default=0)
opts = ap.parse_args()

# The links that start a part of their own (a clause, a phrase set apart).
PART_RELS = {"root", "advcl", "ccomp", "csubj", "parataxis", "vocative", "discourse", "appos"}
CLAUSE_KIDS = {"nsubj", "cop", "aux", "mark", "expl"}


def parts(heads, rels, tags, repair=False):
    """Each word's part: the nearest word up the tree (itself included)
    that starts one."""
    n = len(heads)
    kids = [[] for _ in range(n + 1)]
    for i, h in enumerate(heads, 1):
        kids[h].append(i)

    def starts(i):
        r = rels[i - 1]
        t = tags[i - 1] if tags else ""
        if r in PART_RELS:
            return True
        if r == "conj":
            # A joined part is its own when it is a clause: a verb, or with
            # a subject, a copula, an auxiliary.
            return "Verb/Fin" in t or any(rels[k - 1] in CLAUSE_KIDS for k in kids[i])
        if r == "acl":
            # «который» clauses and participle phrases with their words.
            return "Verb/Fin" in t or bool(kids[i]) or any(rels[k - 1] in CLAUSE_KIDS for k in kids[i])
        return False

    out = []
    for i in range(1, n + 1):
        j, hops = i, 0
        while j and not starts(j) and hops < n:
            j, hops = heads[j - 1], hops + 1
        out.append(j)
    if repair and REPAIR:
        out = with_predicates(out, heads, rels, tags)
    return out


# The anchors' repairs (on our graph; the teacher's tree needs none).
REPAIR = True
PREDICATE = ("Verb/Fin", "Verb/Conv", "short", "Verb/Inf")
NO_PREDICATE_NEEDED = {"vocative", "discourse", "appos"}


def with_predicates(part, heads, rels, tags):
    """A part is a part with something that says what happens in it (a
    verb, a gerund, a participle with its words, a short adjective): a piece
    with none — the graph read without marks cut it off — joins the part
    before it (after it, at the sentence's start); addresses, asides and
    appositions need none."""
    part = list(part)
    changed = True
    while changed:
        changed = False
        for p in sorted(set(part)):
            members = [i for i, q in enumerate(part) if q == p]
            head_rel = rels[p - 1] if p else "root"
            if p == 0 or head_rel in NO_PREDICATE_NEEDED or head_rel == "root":
                continue
            # Or a subject: a predicate noun or adjective with no verb («что
            # он врач»).
            has_pred = any(any(k in tags[i] for k in PREDICATE) or
                           ("Verb/Part" in tags[i] and len(members) > 1) or
                           (rels[i] in ("nsubj", "cop") and heads[i] - 1 in members) for i in members)
            if has_pred:
                continue
            first = members[0]
            to = part[first - 1] if first > 0 else part[members[-1] + 1] if members[-1] + 1 < len(part) else p
            if to != p:
                part = [to if q == p else q for q in part]
                changed = True
                break
    return part


def bounds(part):
    return {i for i in range(1, len(part)) if part[i] != part[i - 1]}


import collections
lost = collections.Counter()
extra = collections.Counter()
tp = fp = fn = 0
comma_n = comma_on_gold = comma_on_ours = 0
shown = 0
for line in open(opts.graph, encoding="utf-8"):
    f = line.rstrip("\n").split("\t")
    words = f[1].split()
    ours_h = [int(x) for x in f[2].split()]
    ours_r = f[3].split()
    tags = f[4].split("|")
    gold_h = [int(x) for x in f[5].split()]
    gold_r = [r.split(":")[0] for r in f[6].split()]
    marks = f[7].split()
    if not len(words) == len(ours_h) == len(gold_h) == len(marks):
        continue
    ours_p = parts(ours_h, ours_r, tags, repair=True)
    gold_p = parts(gold_h, gold_r, tags)
    ours, gold = bounds(ours_p), bounds(gold_p)
    # The kind of part a bound opens or closes: its head's link (the part
    # on the right, or the left one when that is the inner one).
    def kind(p, rs, k):
        a, b = p[k - 1], p[k]
        inner = b if b and (a == 0 or rs[b - 1] != "root") else a
        return rs[inner - 1] if inner else "root"
    for k in gold - ours:
        lost[kind(gold_p, gold_r, k)] += 1
    for k in ours - gold:
        extra[kind(ours_p, ours_r, k)] += 1
    tp += len(ours & gold)
    fp += len(ours - gold)
    fn += len(gold - ours)
    # A bound k lies between word k and word k+1 (0-based word k-1 | k):
    # the comma before word k+1, marks[k].
    for k in range(1, len(words)):
        if "," in marks[k]:
            comma_n += 1
            comma_on_gold += k in gold
            comma_on_ours += k in ours
    if shown < opts.show and ours != gold:
        shown += 1
        print(" ".join(w + ("|" if i + 1 in gold else "") for i, w in enumerate(words)))
        print(" ".join(w + ("|" if i + 1 in ours else "") for i, w in enumerate(words)), "  ← ours")
        print()
print(f"bounds of the parts, ours against the teacher's: right {tp / max(tp + fp, 1):.1%}, "
      f"found {tp / max(tp + fn, 1):.1%} ({tp + fn} bounds)")
print("lost, by the part's link:", ", ".join(f"{r} {c}" for r, c in lost.most_common(8)))
print("extra, by the part's link:", ", ".join(f"{r} {c}" for r, c in extra.most_common(8)))
print(f"commas on a bound: the teacher's parts {comma_on_gold / max(comma_n, 1):.1%}, "
      f"ours {comma_on_ours / max(comma_n, 1):.1%} (of {comma_n})")
