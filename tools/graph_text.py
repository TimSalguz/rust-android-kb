#!/usr/bin/env python3
"""A text into sentences and their graphs, with our own tools only: the
sentences split at their ends, the words with the marks before them, the
parse of each sentence by the student parser (tools/graph_parser.py; the
marks read), each word's relation to its head, how sure, and its part of
speech from the dictionary — the reading its link calls for
(tools/graph_tags.py). What the rest of the text graph is built on (events,
who is who, time) — no tagger or parser from outside.

Usage: tools/graph_text.py MODEL.pt --d 256 --layers 6 [--places 40] TEXT OUT.tsv

TEXT: a paragraph a line. OUT: a sentence a row — line<TAB>sentence (within
the line)<TAB>words<TAB>parts of speech<TAB>heads<TAB>relations<TAB>features
<TAB>marks before each word<TAB>marks after the last<TAB>the graph's chance
of each head and relation («h,r»)<TAB>each word's second likeliest head and its
chance («head:chance») — the graph keeps the other reading. A sentence
longer than the parser reads
is cut at its «;» or «:» (then at the limit): each piece parsed apart.
"""
import argparse
import math
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ap = argparse.ArgumentParser()
ap.add_argument("model")
ap.add_argument("text")
ap.add_argument("out")
ap.add_argument("--d", type=int, default=128)
ap.add_argument("--layers", type=int, default=4)
ap.add_argument("--places", type=int, default=40)
ap.add_argument("--no-names", action="store_true", help="the parser's heads as they are, no name grammar")
ap.add_argument("--tree", action="store_true",
                help="the best tree whose links don't cross (Eisner), on the joint scores")
ap.add_argument("--joint", action="store_true",
                help="choose each word's head and relation together, weighed by the grammar of the pair")
ap.add_argument("--grammar", action="store_true",
                help="mask a bare noun's heads by case (off: without the link's type it forbids true links — "
                     "a nominal predicate's subject, conjuncts with their own prepositions; 92.99 → 92.28%% "
                     "on «Война и мир»; the case must go with the relation, head and relation chosen together)")
opts = ap.parse_args()

src = open(f"{ROOT}/tools/graph_parser.py", encoding="utf-8").read().split("\ntrain = read(")[0]
sys.argv = ["graph_parser.py", "--places", str(opts.places), "--d", str(opts.d), "--layers", str(opts.layers)]
g = {"__file__": f"{ROOT}/tools/graph_parser.py", "__name__": "graph_parser"}
exec(compile(src, "graph_parser.py", "exec"), g)
sys.path.insert(0, f"{ROOT}/tools")
sys.argv = ["graph_tags.py"]
import graph_tags  # noqa: E402

torch, np, L = g["torch"], g["np"], g["L"]
saved = torch.load(opts.model, map_location="cpu")
g["RELS"].update(saved["rels"])
rel_name = {i: r for r, i in saved["rels"].items()}
model = g["Parser"](opts.d, opts.layers)
model.bag_rows = saved["model"]["bag_rows"]
model.load_state_dict(saved["model"], strict=False)
model.to(g["dev"]).eval()

WORD = re.compile(r"[^\W\d_]+(?:[-’'][^\W\d_]+)*")
LATIN = re.compile(r"[a-zà-öø-ÿœ]")
MARKS = re.compile(r"[,—–:;()«»\"„“”.!?…-]")
# A sentence ends at . ! ? … (with closing quotes or brackets) before a space
# and a capital, a dash or an opening quote — not after an initial or a
# short form («Л. Н.», «т. е.», «г.»).
END = re.compile(r"([.!?…]+[»\")]*)\s+(?=[—–«\"(]?\s*[A-ZА-ЯЁ])")
SHORT = re.compile(r"(?:^|\s)(?:[А-ЯЁA-Z]|т|г|гг|см|ср|др|пр|им|св|ст|стр|ул|тыс|млн|руб|коп)\.$")


def sentences(par):
    out, start = [], 0
    for m in END.finditer(par):
        if SHORT.search(par[:m.start() + 1]):
            continue
        out.append(par[start:m.end(1)])
        start = m.end()
    out.append(par[start:])
    return [s.strip() for s in out if s.strip()]


def tokens(sent):
    """Words (lowercase, ё → е) with the marks before each, and the marks
    after the last."""
    sent = re.sub(r"\d+(?:[.,:]\d+)*", lambda m: " " * len(m.group(0)), sent)
    words, before, last = [], [], 0
    caps = []
    for m in WORD.finditer(sent):
        between = "".join(MARKS.findall(sent[last:m.start()])).replace("–", "—").replace("-", "—") \
            .replace("„", "«").replace("“", "»").replace("”", "»")
        words.append(m.group(0).lower().replace("ё", "е"))
        caps.append(m.group(0)[:1].isupper())
        before.append(between or "_")
        last = m.end()
    tail = "".join(MARKS.findall(sent[last:])) or "_"
    tokens.caps = caps
    return words, before, tail


# ---------------------------------------------------------------- names
# A person's name: [title] [given name] [patronymic] [surname] — its words in
# one case («княгиня Анна Михайловна Друбецкая»). Where the case changes a
# new name begins: «у Анны Павловны | княгине Друбецкой» are two people.
TITLES = {"князь", "княгиня", "княжна", "граф", "графиня", "барон", "баронесса", "маркиз", "виконт",
          "царь", "царица", "император", "императрица", "государь", "государыня", "генерал", "полковник",
          "капитан", "ротмистр", "майор", "поручик", "есаул", "профессор", "доктор", "господин", "госпожа"}
ALL_CASES = {"nomn", "gent", "datv", "accs", "ablt", "loct"}
TITLE_STEMS = sorted({t[:-1] if t[-1] in "аяьй" else t for t in TITLES}, key=len, reverse=True)


def is_title(word):
    """A title in any case: «княгине», «князю», «графа»."""
    return any(word.startswith(st) and len(word) <= len(st) + 3 for st in TITLE_STEMS)


def name_cases(word):
    rs = graph_tags.sets.get(graph_tags.word_set.get(word, ""), [])
    cs = {("gent" if g == "gen2" else "accs" if g == "acc2" else "loct" if g == "loc2" else g)
          for r in rs if r[0] == "NOUN" for g in r[1:] if g in CASES}
    return cs or ALL_CASES  # a name the dictionary doesn't know: any case


def name_spans(words, caps):
    """The names in a sentence: runs of capitalized words (and titles before
    them) that share a case; each run as (start, end, cases)."""
    n = len(words)
    namish = [caps[i] and i > 0 for i in range(n)]
    for i in range(n - 1):
        if is_title(words[i]) and namish[i + 1]:
            namish[i] = True
    spans, i = [], 0
    while i < n:
        if not namish[i]:
            i += 1
            continue
        a, cs = i, name_cases(words[i])
        i += 1
        while i < n and namish[i] and not (is_title(words[i]) and i > a) and cs & name_cases(words[i]):
            cs = cs & name_cases(words[i])
            i += 1
        spans.append((a, i, cs))
    return spans


def mend_names(words, caps, pick, p):
    """A word of one name hanging on another name whose case it can't share
    goes to its likeliest head outside that name («княгине» → «данное», not
    «Анны»). `pick` [n+1] heads (1-based words, 0 root), `p` [n+1, ≥n+1]."""
    spans = name_spans(words, caps)
    if len(spans) < 2:
        return 0
    span_of = {}
    for k, (a, b, cs) in enumerate(spans):
        for i in range(a, b):
            span_of[i] = k
    mended = 0
    for i in range(len(words)):
        h = int(pick[i + 1]) - 1
        # A participle after a comma goes with the nearest name before it that
        # agrees with it («у Анны Павловны княгине Друбецкой, просившей»).
        rs_i = graph_tags.sets.get(graph_tags.word_set.get(words[i], ""), [])
        if h >= 0 and h in span_of and rs_i and all(r[0] in ("PRTF", "ADJF") for r in rs_i) and h < i:
            nearer = [k for k, (a, b, cs) in enumerate(spans) if spans[span_of[h]][1] <= a and b <= i]
            for k in reversed(nearer):
                a, b, cs = spans[k]
                head_rs = [r for w in words[a:b] for r in graph_tags.sets.get(graph_tags.word_set.get(w, ""), [])]
                if not head_rs or agree(rs_i, head_rs):
                    pick[i + 1] = a + 1
                    mended += 1
                    break
            continue
        if i not in span_of or h < 0 or h not in span_of or span_of[h] == span_of[i]:
            continue
        mine, theirs = spans[span_of[i]], spans[span_of[h]]
        if mine[2] & theirs[2]:
            continue
        banned = set(range(theirs[0], theirs[1])) | {i}
        order = torch.argsort(p[i + 1, : len(words) + 1], descending=True).tolist()
        for c in order:
            if c == 0 or (c - 1) not in banned:
                pick[i + 1] = c
                mended += 1
                break
    return mended


NOUNISH = ("NOUN", "NPRO")
CASES = {"nomn", "gent", "gen2", "datv", "accs", "acc2", "ablt", "loct", "loc2", "voct"}


def noun_cases(word):
    """The cases of a word that can only be a noun (None: it may be else)."""
    rs = graph_tags.sets.get(graph_tags.word_set.get(word, ""), [])
    if not rs or any(r[0] not in NOUNISH for r in rs):
        return None
    return {g for r in rs for g in r[1:] if g in CASES}


def only(word, pos):
    rs = graph_tags.sets.get(graph_tags.word_set.get(word, ""), [])
    return bool(rs) and all(r[0] in pos for r in rs)


def admissible(words):
    """The heads the grammar allows each word (a mask over root + words):
    a noun with no preposition hangs on a noun only in the genitive, or
    sharing its case (one name of several words, an apposition) — «у Анны
    Павловны княгине»: the dative «княгине» can't hang on «Анны». (With a
    preposition — «письмо к отцу» — any case.)"""
    n = len(words)
    cs = [noun_cases(w) for w in words]
    mask = []
    for i in range(n):
        row = [True] * (n + 1)
        # A preposition before it, over its adjectives: no constraint.
        j = i - 1
        while j >= 0 and only(words[j], ("ADJF", "PRTF", "NUMR", "NPRO")) and not noun_cases(words[j]):
            j -= 1
        rs = graph_tags.sets.get(graph_tags.word_set.get(words[j], ""), []) if j >= 0 else []
        has_prep = any(r[0] == "PREP" for r in rs)
        if cs[i] and not has_prep:
            for j in range(n):
                if j != i and cs[j] and not (cs[i] & cs[j]) and not (cs[i] & {"gent", "gen2"}):
                    row[j + 1] = False
        mask.append(row)
    return mask


# ---------------------------------------------------------------- the joint choice
GOVERNS = {"у": {"gent", "gen2"}, "от": {"gent", "gen2"}, "из": {"gent", "gen2"}, "до": {"gent", "gen2"},
           "для": {"gent"}, "без": {"gent", "gen2"}, "после": {"gent"}, "около": {"gent"}, "возле": {"gent"},
           "кроме": {"gent"}, "среди": {"gent"}, "из-за": {"gent"}, "из-под": {"gent"}, "против": {"gent"},
           "вокруг": {"gent"}, "мимо": {"gent"}, "к": {"datv"}, "ко": {"datv"}, "по": {"datv", "loct", "accs"},
           "с": {"ablt", "gent", "gen2"}, "со": {"ablt", "gent"}, "в": {"loct", "loc2", "accs", "acc2"},
           "во": {"loct", "accs"}, "на": {"loct", "loc2", "accs"}, "о": {"loct", "accs"}, "об": {"loct", "accs"},
           "обо": {"loct"}, "при": {"loct"}, "за": {"ablt", "accs"}, "под": {"ablt", "accs"}, "над": {"ablt"},
           "перед": {"ablt"}, "между": {"ablt", "gent"}, "через": {"accs"}, "про": {"accs"}, "сквозь": {"accs"}}
PENALTY = 8.0  # nats: the grammar against a pair — not a ban (unknown words have no readings)


def readings_of(word):
    return graph_tags.sets.get(graph_tags.word_set.get(word, ""), [])


def cases_of(rs, pos):
    return {g for r in rs if r[0] in pos for g in r[1:] if g in CASES}


def agree(dep, head):
    """A modifier and its noun share a case and a number (a gender too in
    the singular) in some pair of their readings."""
    for a in dep:
        if a[0] not in ("ADJF", "PRTF", "NPRO", "NUMR"):
            continue
        for b in head:
            if b[0] not in ("NOUN", "NPRO", "ADJF"):
                continue
            ca, cb = set(a) & CASES, set(b) & CASES
            if not (ca & cb):
                continue
            na, nb = set(a) & {"sing", "plur"}, set(b) & {"sing", "plur"}
            if na and nb and not (na & nb):
                continue
            if "sing" in (na & nb):
                ga, gb = set(a) & {"masc", "femn", "neut"}, set(b) & {"masc", "femn", "neut"}
                if ga and gb and not (ga & gb) and "ms-f" not in a and "ms-f" not in b:
                    continue
            return True
    return False


CASE_TABLE = None


def case_table():
    """P(case | relation, preposition) learned from the teacher's parses
    (data/graph/case_by_relation.json): «iobj» is the dative 0.74 and the
    instrumental 0.23 («развела руками»), a bare «nmod» the genitive 0.97."""
    global CASE_TABLE
    if CASE_TABLE is None:
        import json
        raw = json.load(open(f"{ROOT}/data/graph/case_by_relation.json", encoding="utf-8"))
        CASE_TABLE = {}
        for key, d in raw.items():
            t = sum(d.values())
            if t >= 50:
                CASE_TABLE[key] = {c: (k + 1) / (t + 10) for c, k in d.items()}
    return CASE_TABLE


HEAD_TABLE = None
UPOS_OF = {"NOUN": ("NOUN", "PROPN"), "NPRO": ("PRON", "DET"), "VERB": ("VERB", "AUX"), "INFN": ("VERB",),
           "GRND": ("VERB",), "PRTF": ("VERB", "ADJ"), "PRTS": ("VERB", "ADJ"), "ADJF": ("ADJ", "DET"),
           "ADJS": ("ADJ",), "COMP": ("ADJ", "ADV"), "ADVB": ("ADV",), "PRED": ("ADV", "VERB"),
           "NUMR": ("NUM",), "PREP": ("ADP",), "CONJ": ("CCONJ", "SCONJ"), "PRCL": ("PART",), "INTJ": ("INTJ",)}


def head_table():
    """P(the head's part of speech | relation), learned from the parses:
    «iobj» hangs on a verb 0.83 (a noun 0.04), «nmod» on a noun 0.95."""
    global HEAD_TABLE
    if HEAD_TABLE is None:
        import json
        raw = json.load(open(f"{ROOT}/data/graph/head_pos_by_relation.json", encoding="utf-8"))
        HEAD_TABLE = {}
        for r, d in raw.items():
            t = sum(d.values())
            if t >= 50:
                HEAD_TABLE[r] = {u: (k + 1) / (t + 20) for u, k in d.items()}
    return HEAD_TABLE


def grammar_costs(words, rel_names):
    """Each (word, head, relation)'s cost by the grammar of the pair, in
    nats: how unlikely the word's possible cases are for the relation (and
    its preposition), as the parses show — 0 for the relation's likeliest
    case; a name's word or an apposition not sharing its head's case, an
    adjective not agreeing with its noun: PENALTY (one name's words share
    their case almost always), PENALTY / 2."""
    table = case_table()
    n = len(words)
    rs = [readings_of(w) for w in words]
    nounish = [bool(r) and all(x[0] in ("NOUN", "NPRO") for x in r) for r in rs]
    modifier = [bool(r) and all(x[0] in ("ADJF", "PRTF", "NUMR", "NPRO") for x in r) and
                any(x[0] in ("ADJF", "PRTF") for x in r) for r in rs]
    prep = []
    for i in range(n):  # the preposition before a noun, over its modifiers
        j = i - 1
        while j >= 0 and modifier[j]:
            j -= 1
        prep.append(words[j] if j >= 0 and any(x[0] == "PREP" for x in rs[j]) else "-")
    R = len(rel_names)
    cost = np.zeros((n, n + 1, R), dtype=np.float32)
    # The head's part of speech for each relation (the root: «ROOT»).
    heads_t = head_table()
    upos_sets = [{"ROOT"}] + [{u for x in r for u in UPOS_OF.get(x[0], ())} for r in rs]
    for k, r in enumerate(rel_names):
        d = heads_t.get(r)
        if not d:
            continue
        best = max(d.values())
        for j, us in enumerate(upos_sets):
            if not us:
                continue
            p = sum(d.get(u, 0.0) for u in us)
            cost[:, j, k] += min(PENALTY, max(0.0, math.log(best) - math.log(max(p, 1e-6))))
    for i in range(n):
        if not nounish[i] and not modifier[i]:
            continue
        own = cases_of(rs[i], ("NOUN", "NPRO"))
        if nounish[i]:
            for k, r in enumerate(rel_names):
                d = table.get(f"{r}|{prep[i]}") or (table.get(f"{r}|-") if prep[i] == "-" else None)
                if not d or not own:
                    continue
                best = max(d.values())
                p = sum(d.get(c, 0.0) for c in own | {"gent" if c == "gen2" else c for c in own})
                cost[i, :, k] = min(PENALTY, max(0.0, math.log(best) - math.log(max(p, 1e-6))))
        for j in range(n):
            if j == i or not rs[j]:
                continue
            if nounish[i] and nounish[j] and not (own & cases_of(rs[j], ("NOUN", "NPRO", "ADJF"))):
                # One name's words, an apposition, and bare conjuncts share the
                # case («Анны и Марии»; «с выражением … и звездах» has its own «в»).
                for r in ("flat", "appos") + (("conj",) if prep[i] == "-" else ()):
                    if r in rel_names:
                        cost[i, j + 1, rel_names.index(r)] += PENALTY
            if modifier[i] and any(x[0] in ("NOUN", "NPRO") for x in rs[j]) and not agree(rs[i], rs[j]):
                for r in ("amod", "det", "acl", "nummod"):
                    if r in rel_names:
                        cost[i, j + 1, rel_names.index(r)] += PENALTY / 2
    return cost


def eisner(scores):
    """The best projective tree for one sentence: scores[d, h] of word d
    (1..n) hanging on h (0 the root), [n+1, n+1] — heads, [n+1] (0 for the
    root). First-order Eisner, one root's child allowed per its rule."""
    n = scores.shape[0] - 1
    NEG = -1e9
    # complete/incomplete spans, right-headed (0) and left-headed (1)
    C = np.full((n + 1, n + 1, 2), NEG)
    I = np.full((n + 1, n + 1, 2), NEG)
    Cb = np.zeros((n + 1, n + 1, 2), dtype=np.int64)
    Ib = np.zeros((n + 1, n + 1, 2), dtype=np.int64)
    for i in range(n + 1):
        C[i, i, 0] = C[i, i, 1] = 0.0
    for w in range(1, n + 1):
        for i in range(0, n + 1 - w):
            j = i + w
            # incomplete: an arc between i and j
            cand = C[i, i:j, 1] + C[i + 1:j + 1, j, 0]
            k = int(np.argmax(cand))
            best = cand[k]
            I[i, j, 0] = best + scores[i, j] if i else NEG  # j -> i (i's head is j); the root has no head
            I[i, j, 1] = best + scores[j, i]                  # i -> j
            Ib[i, j, 0] = Ib[i, j, 1] = i + k
            # complete
            cand = C[i, i:j, 0] + I[i:j, j, 0]
            k = int(np.argmax(cand))
            C[i, j, 0], Cb[i, j, 0] = cand[k], i + k
            cand = I[i, i + 1:j + 1, 1] + C[i + 1:j + 1, j, 1]
            k = int(np.argmax(cand))
            C[i, j, 1], Cb[i, j, 1] = cand[k], i + 1 + k
    heads = np.zeros(n + 1, dtype=np.int64)

    def back(i, j, d, complete):
        if i == j:
            return
        if complete:
            k = Cb[i, j, d]
            if d == 0:
                back(i, k, 0, True)
                back(k, j, 0, False)
            else:
                back(i, k, 1, False)
                back(k, j, 1, True)
        else:
            k = Ib[i, j, d]
            if d == 0:
                heads[i] = j
            else:
                heads[j] = i
            back(i, k, 1, True)
            back(k + 1, j, 0, True)
    back(0, n, 1, True)
    return heads


def pieces(words, before):
    """A sentence the parser can read whole, or cut at «;» «:» (then at the
    limit)."""
    n = L - 1
    if len(words) <= n:
        return [(0, len(words))]
    cuts = [i for i, m in enumerate(before) if i and (";" in m or ":" in m)]
    out, start = [], 0
    while len(words) - start > n:
        fits = [c for c in cuts if start < c <= start + n]
        end = fits[-1] if fits else start + n
        out.append((start, end))
        start = end
    out.append((start, len(words)))
    return out


rows = []  # (line, sentence, words, marks, tail, piece start)
caps_of = []  # each row's words: capitalized as written
for ln, par in enumerate(open(opts.text, encoding="utf-8"), 1):
    for k, sent in enumerate(sentences(par.rstrip("\n"))):
        words, before, tail = tokens(sent)
        caps = tokens.caps
        if not words:
            continue
        for a, b in pieces(words, before):
            rows.append((ln, k, words[a:b], before[a:b], tail if b == len(words) else "_"))
            caps_of.append(caps[a:b])
print(f"{len(rows)} sentences (pieces)", file=sys.stderr, flush=True)

with torch.no_grad(), open(opts.out, "w", encoding="utf-8") as out:
    for i in range(0, len(rows), 512):
        chunk = rows[i:i + 512]
        t = g["tensors"]([(w, [0] * len(w), [0] * len(w), m) for _, _, w, m, _ in chunk])
        model.bag_rows = torch.tensor(np.stack(g["bag_rows"])).to(g["dev"])
        lem, cls, bags, mask, head, rel, mk = g["batch"](t, slice(0, len(chunk)))
        s, h = model(lem, cls, bags, mask, mk)
        if opts.grammar:
            for j, (_, _, w, _, _) in enumerate(chunk):
                for i, row in enumerate(admissible(w)):
                    bad = [c for c, ok in enumerate(row) if not ok]
                    if bad and len(bad) < len(row):
                        s[j, i + 1, bad] = -1e9
        p = torch.softmax(s, -1)
        pick = p.argmax(-1)
        if not opts.no_names:
            for j, (_, _, w, _, _) in enumerate(chunk):
                mend_names(w, caps_of[i + j], pick[j], p[j])
        # A foreign stretch (Latin letters: Tolstoy's French) is one node:
        # its words hang on its first, as UD writes it (flat:foreign).
        foreign = []
        for j, (_, _, w, _, _) in enumerate(chunk):
            run = []
            for t, word in enumerate(w + [""]):
                if word and LATIN.search(word):
                    run.append(t)
                    continue
                if len(run) >= 2:
                    for q in run[1:]:
                        pick[j, q + 1] = run[0] + 1
                        foreign.append((j, q + 1))
                    if run[0] + 1 <= int(pick[j, run[0] + 1]) <= run[-1] + 1:
                        # its first word goes outside the stretch
                        order = torch.argsort(p[j, run[0] + 1, : len(w) + 1], descending=True).tolist()
                        pick[j, run[0] + 1] = next(c for c in order if not run[0] + 1 <= c <= run[-1] + 1)
                run = []
        hp = p.gather(-1, pick.unsqueeze(-1))[..., 0]
        hidx = pick.clamp(max=L - 1)
        rl = model.rel(torch.cat([h, torch.gather(h, 1, hidx.unsqueeze(-1).expand(-1, -1, h.shape[-1]))], -1))
        rp, rk = torch.softmax(rl, -1).max(-1)
        flat_id = next((k for k, r in rel_name.items() if r == "flat"), None)
        for j, q in foreign:
            if flat_id is not None:
                rk[j, q] = flat_id
        if opts.joint:
            # Every (head, relation) pair: the relation layer's first linear
            # map splits into the word's part and the head's part.
            W = model.rel[0].weight
            dpart = h @ W[:, : h.shape[-1]].T + model.rel[0].bias  # [B, L, d]
            hpart = h @ W[:, h.shape[-1]:].T                        # [B, L, d]
            names = [rel_name[k] for k in range(len(rel_name))]
            logp_head = torch.log_softmax(s, -1)[..., :L]            # [B, L, L] (no «later»)
            for j, (_, _, w, _, _) in enumerate(chunk):
                n = len(w)
                pair = torch.relu(dpart[j, 1:n + 1, None, :] + hpart[j, None, : n + 1, :])  # [n, n+1, d]
                logp_rel = torch.log_softmax(model.rel[2](pair), -1)                        # [n, n+1, R]
                cost = torch.tensor(grammar_costs(w, names), device=pair.device)
                total = logp_head[j, 1:n + 1, : n + 1, None] + logp_rel - cost
                total[torch.arange(n), torch.arange(1, n + 1)] = -1e9  # not itself
                flat = total.reshape(n, -1).argmax(-1)
                hj, rj = flat // len(names), flat % len(names)
                if opts.tree:
                    # Each arc's score: its best relation; the tree over them.
                    arc, arc_r = total.max(-1)                       # [n, n+1]
                    sc = np.full((n + 1, n + 1), -1e9)
                    sc[1:, :] = arc.cpu().numpy()
                    th = torch.tensor(eisner(sc)[1:], device=arc.device)
                    hj, rj = th, arc_r[torch.arange(n), th]
                pick[j, 1:n + 1] = hj
                rk[j, 1:n + 1] = rj
                hp[j, 1:n + 1] = logp_head[j, 1:n + 1].exp().gather(1, hj[:, None])[:, 0]
                rp[j, 1:n + 1] = logp_rel.exp()[torch.arange(n), hj, rj]
        top2 = p.topk(2, dim=-1)
        pick, hp, rp, rk = pick.cpu(), hp.cpu(), rp.cpu(), rk.cpu()
        alt_h, alt_p = top2.indices[..., 1].cpu(), top2.values[..., 1].cpu()
        for j, (ln, k, words, before, tail) in enumerate(chunk):
            n = len(words)
            heads = [int(pick[j, t]) if int(pick[j, t]) <= n else 0 for t in range(1, n + 1)]
            rels = [rel_name.get(int(rk[j, t]), "dep") for t in range(1, n + 1)]
            tagged = [graph_tags.tag(w, r) for w, r in zip(words, rels)]
            upos = " ".join(u for u, _ in tagged)
            feats = "|".join(";".join(f"{a}={b}" for a, b in fs.items()) or "_" for _, fs in tagged)
            sure = " ".join(f"{float(hp[j, t]):.3f},{float(rp[j, t]):.3f}" for t in range(1, n + 1))
            alt = " ".join(f"{int(alt_h[j, t]) if int(alt_h[j, t]) <= n else 0}:{float(alt_p[j, t]):.3f}"
                           for t in range(1, n + 1))
            out.write(f"{ln}.{k}\t{' '.join(words)}\t{upos}\t{' '.join(map(str, heads))}\t{' '.join(rels)}"
                      f"\t{feats}\t{' '.join(before)}\t{tail}\t{sure}\t{alt}\n")
        if i % 10240 == 0:
            print(f"{i + len(chunk)}/{len(rows)}", file=sys.stderr, flush=True)
