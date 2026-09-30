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
            links.append({"head": j, "p": float(p[i + 1, j]), "rels": [[r, round(q, 4)] for q, r in rel_p]})
        links.sort(key=lambda x: -x["p"])
        out.append({"word": w, "readings": readings(w), "links": links})
    return out


PRON = {"он": ("masc", "sing"), "его": ("masc", "sing"), "ему": ("masc", "sing"), "им": ("masc", "sing"),
        "нем": ("masc", "sing"), "него": ("masc", "sing"), "она": ("femn", "sing"), "ее": ("femn", "sing"),
        "её": ("femn", "sing"), "ей": ("femn", "sing"), "ней": ("femn", "sing"), "нее": ("femn", "sing"),
        "они": (None, "plur"), "их": (None, "plur"), "ими": (None, "plur")}


def antecedents(text_words, at, gender, number):
    """Earlier nouns (animate) the pronoun can stand for, equal chances; with
    the reading that matched."""
    found = []
    for k in range(at - 1, -1, -1):
        s_i, w_i, w = text_words[k]
        for t in readings(w):
            parts = t.split(",")
            if parts[0] == "NOUN" and "anim" in parts and number in parts and (gender is None or gender in parts):
                found.append((s_i, w_i, w, t))
                break
    return found


CASE_Q = {"nomn": "кто? что?", "gent": "кого? чего?", "datv": "кому? чему?", "accs": "кого? что?",
          "ablt": "кем? чем?", "loct": "о ком? о чём?", "gen2": "кого? чего?", "loc2": "где?"}
# A preposition's cases, and the question each asks.
PREP_Q = {"в": {"loct": "где?", "loc2": "где?", "accs": "куда? во что?"}, "на": {"loct": "где? на чём?", "loc2": "где?", "accs": "куда? на что?"},
          "из": {"gent": "откуда? из чего?"}, "от": {"gent": "от кого? от чего?"}, "с": {"ablt": "с кем? с чем?", "gent": "откуда?"},
          "к": {"datv": "к кому? к чему?"}, "о": {"loct": "о ком? о чём?"}, "об": {"loct": "о ком? о чём?"},
          "за": {"ablt": "за кем? за чем?", "accs": "за что?"}, "по": {"datv": "по чему? где?"}, "у": {"gent": "у кого? где?"},
          "для": {"gent": "для кого? для чего?"}, "без": {"gent": "без кого? без чего?"}, "до": {"gent": "до чего? докуда?"},
          "под": {"ablt": "под чем? где?", "accs": "под что? куда?"}, "над": {"ablt": "над чем?"}, "перед": {"ablt": "перед чем?"},
          "через": {"accs": "через что?"}, "про": {"accs": "про кого? про что?"}, "при": {"loct": "при ком? при чём?"}}
REL_Q = {"nsubj": "кто? (подлежащее)", "obj": "кого? что?", "iobj": "кому? чему?", "amod": "какой?", "det": "чей? какой?",
         "advmod": "как?", "nummod": "сколько?", "conj": "и ещё", "appos": "то есть",
         "ccomp": "что?", "acl": "какой? (оборот)", "flat": "(имя)", "vocative": "(обращение)"}
CAUSE = {"потому": "почему? (причина)", "так": "почему? (причина)", "поскольку": "почему? (причина)",
         "когда": "когда?", "чтобы": "зачем?", "если": "при каком условии?", "хотя": "вопреки чему?"}
# Words that mark a clause, not roles of their own.
MARKERS = {"потому", "что", "так", "как", "поскольку", "чтобы", "если", "хотя", "когда"}
PRED = ("VERB", "INFN", "ADJS", "PRTS", "PRED", "GRND")
# A word is an event where the graph makes it a clause's head.
EVENT_RELS = {"root", "conj", "advcl", "ccomp", "parataxis", "acl", "csubj"}


def best(x):
    return (x["links"][0]["head"], x["links"][0]["rels"][0][0]) if x["links"] and x["links"][0]["rels"] else (None, None)


def dependents(sent, i):
    """The words whose likeliest link goes to word i: (index, relation)."""
    return [(d, best(y)[1]) for d, y in enumerate(sent) if best(y)[0] == i + 1]


def question(sent, dep, rel):
    """The school question from the head to the dependent `dep`."""
    y = sent[dep]
    if rel == "xcomp":
        # A verb's infinitive, or a nominal part: «стали друзьями» — кем?
        if any(t.startswith("INFN") for t in y["readings"]):
            return "что делать?"
        rel = "obl"
    if rel in REL_Q:
        return REL_Q[rel]
    cases = [c for t in y["readings"] for c in t.split(",") if c in CASE_Q]
    preps = [sent[k]["word"] for k, r in dependents(sent, dep) if r == "case"]
    if preps:
        by_case = PREP_Q.get(preps[0], {})
        for c in cases:
            if c in by_case:
                return by_case[c]
        return preps[0] + " " + CASE_Q.get(cases[0], "что?") if cases else preps[0] + " что?"
    for c in ("ablt", "datv", "gent", "accs", "loct", "nomn"):
        if c in cases:
            return CASE_Q[c]
    return "что?"


def clause_question(sent, head):
    """How a clause hangs on its head: its marker («потому что» — why)."""
    for k, r in dependents(sent, head):
        w = sent[k]["word"]
        if w in CAUSE and r in ("mark", "advmod", "fixed"):
            return CAUSE[w]
    return None


def events(sent):
    """The sentence's events: each word the graph makes a clause's head,
    with the chance of that, its roles — every dependent the graph links to
    it, by the chance of that link, with the school question — the clauses
    hanging on it (why, when), and the facts its grammemes and helpers imply
    (tense, from the word or its copula; negation; supposition)."""
    out = []
    TENSE = {"past": "прошлое", "pres": "настоящее", "futr": "будущее"}
    for i, x in enumerate(sent):
        p_event = sum(l["p"] for l in x["links"] if l["rels"] and l["rels"][0][0] in EVENT_RELS)
        if p_event < 0.05 or not any(t.split(",")[0] in PRED for t in x["readings"]):
            continue
        roles, facts = [], []
        for d, y in enumerate(sent):
            for l in y["links"]:
                if l["head"] != i + 1 or l["p"] < 0.005 or not l["rels"]:
                    continue
                rel = l["rels"][0][0]
                if rel in ("case", "mark", "fixed", "punct", "aux") or y["word"] in MARKERS:
                    continue
                if rel == "cop":
                    tense = [g for t in y["readings"] for g in t.split(",") if g in TENSE]
                    if tense:
                        facts.append({"fact": "время", "value": TENSE[tense[0]], "from": y["word"], "p": round(l["p"], 4)})
                    continue
                if y["word"] == "не":
                    facts.append({"fact": "отрицание", "value": "да", "from": "не", "p": round(l["p"], 4)})
                    continue
                if y["word"] == "бы":
                    facts.append({"fact": "предположение", "value": "да", "from": "бы", "p": round(l["p"], 4)})
                    continue
                q = clause_question(sent, d) if rel in ("conj", "advcl", "parataxis") else None
                roles.append({"question": q or question(sent, d, rel), "word": y["word"], "at": d, "rel": rel,
                              "clause": bool(q), "p": round(l["p"], 4)})
        if not any(f["fact"] == "время" for f in facts):
            tense = sorted({g for t in x["readings"] for g in t.split(",") if g in TENSE})
            if tense:
                facts.append({"fact": "время", "value": " или ".join(TENSE[t] for t in tense), "from": x["word"], "p": 1.0})
            elif any(t.startswith(("ADJS", "PRTS", "PRED")) for t in x["readings"]):
                facts.append({"fact": "время", "value": "настоящее (нет связки)", "from": x["word"], "p": 1.0})
        out.append({"at": i, "predicate": x["word"], "p_event": round(p_event, 4),
                    "roles": sorted(roles, key=lambda r: -r["p"]), "facts": facts})
    return out


text = [re.findall(r"[а-яё]+(?:-[а-яё]+)*", line.lower().replace("ё", "е")) for line in sys.stdin]
text = [s for s in text if s]
graph = {"sentences": [], "coref": []}
flat = []
GENDERS = {"masc": "мужской род", "femn": "женский род", "neut": "средний род"}
NUMBERS = {"sing": "единственное число", "plur": "множественное число"}


def agreement(sent, pred, subj):
    """What the predicate and its subject share (gender, number) — the
    feature node through which they agree: «отец → мужской род ← бил»."""
    def feats(w):
        return {g for t in sent[w]["readings"] for g in t.split(",") if g in GENDERS or g in NUMBERS}
    return sorted((GENDERS | NUMBERS)[g] for g in feats(pred) & feats(subj))


graph["events"] = []
for si, words in enumerate(text):
    graph["sentences"].append(sentence_graph(words))
    evs = events(graph["sentences"][-1])
    for e in evs:
        for r in e["roles"]:
            if r["rel"] == "nsubj":
                shared = agreement(graph["sentences"][-1], e["at"], r["at"])
                if shared:
                    e["facts"].append({"fact": "согласовано с подлежащим", "value": ", ".join(shared),
                                       "from": r["word"], "p": r["p"]})
    graph["events"].append(evs)
    for wi, w in enumerate(words):
        if w in PRON:
            gen, num = PRON[w]
            cands = antecedents(flat, len(flat), gen, num)
            if cands:
                graph["coref"].append({"sentence": si, "word": wi, "pronoun": w,
                                       "grammemes": [x for x in (gen, num, "anim") if x],
                                       "candidates": [{"sentence": a, "word": b, "form": c, "reading": t,
                                                       "p": round(1 / len(cands), 3)} for a, b, c, t in cands]})
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
# A pronoun's role holds every antecedent it may stand for (the worlds of
# the text): «кто? → он» is «отец? 0.5 | сын? 0.5».
for c in graph["coref"]:
    for e in graph["events"][c["sentence"]]:
        for r in e["roles"]:
            if r["at"] == c["word"]:
                r["stands_for"] = [{"form": k["form"], "sentence": k["sentence"], "p": k["p"]} for k in c["candidates"]]
for si, evs in enumerate(graph["events"]):
    for e in evs:
        roles = "; ".join(f"{r['question']} → {r['word']} {r['p']:.2f}"
                          + (" [" + " | ".join(f"{k['form']}? {k['p']:.2f}" for k in r.get("stands_for", [])) + "]"
                             if r.get("stands_for") else "")
                          for r in e["roles"] if r["p"] >= 0.05)
        facts = "; ".join(f"{f['fact']}: {f['value']}" for f in e["facts"])
        print(f"  [{si + 1}] «{e['predicate']}» (событие {e['p_event']:.2f}): {roles}" + (f"  ({facts})" if facts else ""))
for c in graph["coref"]:
    cands = ", ".join(f"{x['form']} ({x['p']:.2f})" for x in c["candidates"])
    print(f"  «{c['pronoun']}» (предл. {c['sentence'] + 1}) → {cands}")
if opts.json:
    json.dump(graph, open(opts.json, "w", encoding="utf-8"), ensure_ascii=False)
