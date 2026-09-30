#!/usr/bin/env python3
"""Rules learned from facts, not written by hand: Horn rules of up to three
atoms — r(X,Y) ← b(X,Y), or r(X,Y) ← b1(·,Z) ∧ b2(Z,·) with the arguments in
either order — mined from a graph of (subject, relation, object) facts, as
AMIE does (Galárraga et al., WWW 2013): support, head coverage, the standard
confidence and the one under partial completeness (a pair the graph says
nothing about is not a miss: only an X whose r is known at all counts).

A rule is a default: its probability for a new case is the rule of
succession over its record, (k+1)/(n+2) — the same uniform prior on its
reliability as the gate below, so the gate and the probability are one
model. Its exceptions are mined too: a condition on X, Y or Z (having some
relation at all) under which it fails is kept when it makes the misses
predictable enough to shorten the description; the default's probability is
then over the misses left unexplained («the parents of a child are spouses —
unless the child has no … »).

Then a gate by description length: a rule is kept only when the facts it
predicts cost fewer bits with it than without — the rule's own bits (which
of the rules of its shape it is) plus which of its n predictions for known
subjects hit (log2((n+1)·C(n,k)) — a uniform prior on its reliability)
against naming each of the k facts it gets right outright (a pair of
entities: 2·log2(entities) bits).

Usage: tools/graph_rules.py FACTS.tsv [--min-support 5] [--min-pca 0.3] [--apply OTHER.tsv]
FACTS: subject<TAB>relation<TAB>object (a header line is skipped when it
starts with «subject»). --apply: the kept rules closed over OTHER's facts —
each derived fact with the rule and the facts it rests on.
"""
import argparse
import collections
import itertools
import math
import sys

ap = argparse.ArgumentParser()
ap.add_argument("facts")
ap.add_argument("--min-support", type=int, default=5)
ap.add_argument("--min-pca", type=float, default=0.3)
ap.add_argument("--max-pairs", type=int, default=3_000_000, help="skip a body with more pairs than this")
ap.add_argument("--apply")
ap.add_argument("--relations", default="", help="only these relations (comma-separated)")
ap.add_argument("--closed", default="", help="relations the graph knows in full (comma-separated): "
                "a pair without them is a miss, not an unknown")
opts = ap.parse_args()


def load(path, only=None):
    facts = set()
    for line in open(path, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        if len(f) < 3 or f[0] == "subject":
            continue
        if only and f[1] not in only:
            continue
        if f[0] != f[2]:
            facts.add((f[0], f[1], f[2]))
    return facts


class Graph:
    def __init__(self, facts):
        self.facts = facts
        self.out = collections.defaultdict(lambda: collections.defaultdict(set))  # r -> s -> {o}
        self.inn = collections.defaultdict(lambda: collections.defaultdict(set))  # r -> o -> {s}
        for s, r, o in facts:
            self.out[r][s].add(o)
            self.inn[r][o].add(s)
        self.relations = sorted(self.out)
        self.entities = {x for s, _, o in facts for x in (s, o)}

    def pairs(self, r, flip=False):
        """(x, y) with r(x, y) — or r(y, x) when flipped."""
        for s, os in self.out[r].items():
            for o in os:
                yield (o, s) if flip else (s, o)

    def step(self, r, flip):
        """x -> {y}: r(x, y), or r(y, x) when flipped."""
        return self.inn[r] if flip else self.out[r]


def body_pairs(g, body):
    """The (X, Y) pairs the body holds for (X ≠ Y), each with the Z that
    join them (none for a one-atom body)."""
    if len(body) == 1:
        (r, flip), = body
        return {(x, y): () for x, y in g.pairs(r, flip) if x != y}
    (r1, f1), (r2, f2) = body
    first, second = g.step(r1, f1), g.step(r2, f2)
    out = {}
    for x, zs in first.items():
        for z in zs:
            for y in second.get(z, ()):
                if x != y:
                    out.setdefault((x, y), set()).add(z)
        if len(out) > opts.max_pairs:
            return None
    return out


def show(head, body):
    def atom(r, flip, a, b):
        return f"{r}({b},{a})" if flip else f"{r}({a},{b})"
    if len(body) == 1:
        (r, f), = body
        return f"{head}(X,Y) ← {atom(r, f, 'X', 'Y')}"
    (r1, f1), (r2, f2) = body
    return f"{head}(X,Y) ← {atom(r1, f1, 'X', 'Z')} ∧ {atom(r2, f2, 'Z', 'Y')}"


def code_bits(n, k):
    """Bits to say which k of n predictions hit (a uniform prior on the rate)."""
    return math.log2(n + 1) + (math.lgamma(n + 1) - math.lgamma(k + 1) - math.lgamma(n - k + 1)) / math.log(2)


def succession(n, k):
    return (k + 1) / (n + 2)


def conditions(g, x):
    """What can be said of an entity: it has relation r (as subject or object)."""
    out = set()
    for r in g.relations:
        if x in g.out[r]:
            out.add(("has", r))
        if x in g.inn[r]:
            out.add(("is", r))
    return out


def exception(g, head, bp, known_x, head_pairs, n_rel):
    """The condition (on X, Y, or every Z joining them) under which the rule
    fails, if one makes its misses predictable enough to pay for itself."""
    judged = [p for p in bp if p[0] in known_x]
    n, k = len(judged), sum(1 for p in judged if p in head_pairs)
    base = code_bits(n, k)
    best = None
    tally = collections.defaultdict(lambda: [0, 0])  # (side, cond) -> [n, k]
    for x, y in judged:
        hit = (x, y) in head_pairs
        zs = bp[(x, y)]
        sides = [("X", conditions(g, x)), ("Y", conditions(g, y))]
        if zs:
            sides.append(("Z", set.intersection(*(conditions(g, z) for z in zs))))
        for side, conds in sides:
            for c in conds:
                t = tally[(side, c)]
                t[0] += 1
                t[1] += hit
    cond_bits = math.log2(3 * 2 * max(n_rel, 1))
    for key, (nc, kc) in tally.items():
        if nc == n or kc / nc > (k - kc) / max(n - nc, 1):
            continue  # an exception is where it fails more than elsewhere
        bits = cond_bits + code_bits(nc, kc) + code_bits(n - nc, k - kc)
        if bits < base and (best is None or bits < best[0]):
            best = (bits, key, nc, kc, n - nc, k - kc)
    return best


def mine(g):
    rels = g.relations
    n_ent = max(len(g.entities), 2)
    fact_bits = 2 * math.log2(n_ent)
    shapes = [((r, f),) for r in rels for f in (False, True)]
    shapes += [((r1, f1), (r2, f2)) for r1, r2 in itertools.product(rels, rels)
               for f1 in (False, True) for f2 in (False, True)]
    # The rule's own bits: which of the rules of its shape and head it is.
    model_bits = {1: math.log2(len(rels) * 2 * len(rels)), 2: math.log2(len(rels) * (2 * len(rels)) ** 2)}
    cache = {}
    kept = []
    for head in rels:
        head_pairs = set(g.pairs(head))
        # Partial completeness: only an X whose head relation is known at all
        # is judged — unless the graph knows that relation in full.
        known_x = g.entities if head in closed else set(g.out[head])
        for body in shapes:
            if len(body) == 1 and body[0] == (head, False):
                continue
            if body not in cache:
                cache[body] = body_pairs(g, body)
            bp = cache[body]
            if not bp:
                continue
            support = sum(1 for p in bp if p in head_pairs)
            if support < opts.min_support:
                continue
            pca_n = sum(1 for x, _ in bp if x in known_x)
            pca = support / pca_n if pca_n else 0.0
            if pca < opts.min_pca:
                continue
            std = support / len(bp)
            hc = support / len(head_pairs)
            n, k = pca_n, support
            with_rule = model_bits[len(body)] + math.log2(n + 1) + (
                math.lgamma(n + 1) - math.lgamma(k + 1) - math.lgamma(n - k + 1)) / math.log(2)
            without = k * fact_bits
            gain = without - with_rule
            if gain > 0:
                exc = exception(g, head, bp, known_x, head_pairs, len(rels))
                if exc:
                    _, (side, (how, r)), nc, kc, rest_n, rest_k = exc
                    prob = succession(rest_n, rest_k)
                    note = f"unless {side} {'has' if how == 'has' else 'is the object of'} {r} ({kc}/{nc} there)"
                    unless = (side, how, r)
                else:
                    prob, note, unless = succession(n, k), "", None
                kept.append((head, body, support, hc, std, pca, gain, len(bp), prob, note, unless))
    kept.sort(key=lambda x: -x[6])
    return kept


def apply(rules, g, sure=None):
    """Close g under the rules: each new fact with its rule, its premises and
    its credence — the rule's probability times the premises' (the best
    rule's, where several conclude it: their evidence overlaps, a noisy-OR
    would count it twice)."""
    sure = sure if sure is not None else {}
    derived = {}
    changed = True
    while changed:
        changed = False
        for head, body, *rest in rules:
            prob, unless = rest[6], rest[8]

            def excepted(x, y, zs):
                """The rule's exception holds here: it doesn't apply."""
                if not unless:
                    return False
                side, how, r = unless
                ents = [x] if side == "X" else [y] if side == "Y" else list(zs)
                table = g.out[r] if how == "has" else g.inn[r]
                return bool(ents) and all(e in table for e in ents)
            if len(body) == 1:
                (r, f), = body
                cands = [((x, y), [((y, r, x) if f else (x, r, y))]) for x, y in g.pairs(r, f)
                         if x != y and not excepted(x, y, ())]
            else:
                (r1, f1), (r2, f2) = body
                first, second = g.step(r1, f1), g.step(r2, f2)
                cands = []
                for x, zs in list(first.items()):
                    for z in zs:
                        for y in second.get(z, ()):
                            if x != y and not excepted(x, y, (z,)):
                                cands.append(((x, y), [(z, r1, x) if f1 else (x, r1, z),
                                                       (y, r2, z) if f2 else (z, r2, y)]))
            for (x, y), prem in cands:
                fact = (x, head, y)
                credence = prob
                for p in prem:
                    credence *= sure.get(p, 1.0)
                if fact in g.facts and fact not in derived:
                    continue
                if fact in derived and derived[fact][2] >= credence:
                    continue
                if fact not in derived:
                    g.facts.add(fact)
                    g.out[head][x].add(y)
                    g.inn[head][y].add(x)
                    changed = True
                derived[fact] = (show(head, body), prem, credence)
                sure[fact] = credence
    return derived


only = set(filter(None, opts.relations.split(",")))
closed = set(filter(None, opts.closed.split(",")))
g = Graph(load(opts.facts, only))
print(f"{len(g.facts)} facts, {len(g.relations)} relations, {len(g.entities)} entities", file=sys.stderr)
rules = mine(g)
print("rule\tsupport\thead coverage\tconfidence\tPCA confidence\tbits saved\tprobability\texception")
for head, body, support, hc, std, pca, gain, nb, prob, note, unless in rules:
    print(f"{show(head, body)}\t{support}\t{hc:.3f}\t{std:.3f}\t{pca:.3f}\t{gain:.0f}\t{prob:.3f}\t{note}")
if opts.apply:
    other = Graph(load(opts.apply))
    derived = apply(rules, other)
    print(f"\n{len(derived)} facts derived over {opts.apply}", file=sys.stderr)
    for (x, r, y), (rule, prem, credence) in derived.items():
        print(f"derived\t{x}\t{r}\t{y}\t{credence:.3f}\t{rule}\t{' ; '.join(' '.join(p) for p in prem)}")
