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
ap.add_argument("--grammar", action="store_true",
                help="mask a bare noun's heads by case (off: without the link's type it forbids true links — "
                     "a nominal predicate's subject, conjuncts with their own prepositions; 92.99 → 92.28% "
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
    for m in WORD.finditer(sent):
        between = "".join(MARKS.findall(sent[last:m.start()])).replace("–", "—").replace("-", "—") \
            .replace("„", "«").replace("“", "»").replace("”", "»")
        words.append(m.group(0).lower().replace("ё", "е"))
        before.append(between or "_")
        last = m.end()
    tail = "".join(MARKS.findall(sent[last:])) or "_"
    return words, before, tail


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
for ln, par in enumerate(open(opts.text, encoding="utf-8"), 1):
    for k, sent in enumerate(sentences(par.rstrip("\n"))):
        words, before, tail = tokens(sent)
        if not words:
            continue
        for a, b in pieces(words, before):
            rows.append((ln, k, words[a:b], before[a:b], tail if b == len(words) else "_"))
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
        hp = p.max(-1).values
        hidx = pick.clamp(max=L - 1)
        rl = model.rel(torch.cat([h, torch.gather(h, 1, hidx.unsqueeze(-1).expand(-1, -1, h.shape[-1]))], -1))
        rp, rk = torch.softmax(rl, -1).max(-1)
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
