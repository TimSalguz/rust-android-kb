#!/usr/bin/env python3
"""Punctuation from the sentence's graph, by the rules of Russian: each
link says where its marks go — a subordinate clause, a «который» clause, a
participle or gerund phrase, an address, a parenthetical word: commas at
their bounds; «а», «но» and joined clauses: a comma before the joining word;
things listed without «и»: a comma between. The marks are the graph's, not
a guess of their own.

Usage: tools/graph_marks.py --eval PARSES.tsv [--show N] [--rule NAME]

PARSES.tsv: data/parse's format plus the marks before each word and after
the last (data/wp/graph/wp_parse.tsv: Stanza's parses of «Война и мир»).
Measured: the commas the rules put against the text's own, overall and by
rule — the rules on a right graph; the parser's graph is measured apart.
"""
import argparse
import collections
import re

PARENTHETICAL = {
    "конечно", "наверное", "например", "кстати", "впрочем", "во-первых", "во-вторых",
    "в-третьих", "кажется", "пожалуй", "видимо", "разумеется", "по-моему", "по-видимому",
    "вероятно", "очевидно", "безусловно", "бесспорно", "несомненно", "право", "правда",
    "итак", "следовательно", "напротив", "наоборот", "словом", "значит", "стало", "может",
    "казалось", "говорят", "признаться", "по-твоему", "по-вашему", "по-нашему",
}
# Words that stand apart in a sentence of their own (UD parataxis / discourse
# heads): a comma sets them off. Any other parataxis frames a quote or an
# aside — a dash, a colon, brackets: not a comma (the writer's choice).
APART = PARENTHETICAL | {"ну", "да", "нет", "ах", "ох", "эх", "ой", "вот", "наконец",
                         "главное", "действительно", "бывает", "ну-ка", "ведь", "впрочем"}
INTERJECTIONS = {"ах", "ох", "эх", "ой", "ай", "ух", "увы"}
# Verbs of saying and thinking: after a quote, their frame is set off («…,
# сообщил он»).
SAYING = re.compile(r"^(сказа|говор|сообщ|заяв|отмет|подчеркн|добав|пояснил|уточн|рассказ|"
                    r"признал|написал|спрос|ответ|отвеча|продолж|прибав|повтор|крикн|закрич|"
                    r"прошепт|шепн|проговор|восклик|думал|подумал|решил|заметил|возраз|объясн|"
                    r"указ|призна|констатир|передает|пишет|считает|полагает|уверен)")
# Set expressions standing apart as parentheses: compound nodes set off whole
# («по его словам», «в частности»). A word sequence; `*` a word of any kind
# («по мнению *», «по данным *»: to the end of the phrase they head).
PAREN_PHRASES = [p.split() for p in (
    "в частности", "кроме того", "к тому же", "таким образом", "с одной стороны",
    "с другой стороны", "в первую очередь", "в свою очередь", "по крайней мере", "на самом деле", "по сути", "как правило", "как известно",
    "как оказалось", "как выяснилось", "как сообщается", "как говорится", "к сожалению",
    "к счастью", "к слову", "по-видимому", "без сомнения", "конечно же",
    "по его словам", "по ее словам", "по их словам", "по моему мнению", "по словам",
    "по данным", "по мнению", "по информации", "по сообщению", "по оценкам", "по прогнозам",
    "по подсчетам", "по сведениям", "по версии", "по мнению")]
# «несмотря на»: set off always.
SET_OFF = {"несмотря"}
ADVERSATIVE = {"а", "но", "однако", "зато", "да", "же"}
JOINING = {"и", "или", "либо", "ни", "да"}
RELATIVE = {"который", "которая", "которое", "которые", "которого", "которой", "которому",
            "которую", "которым", "котором", "которых", "которыми"}


def feats_of(s):
    return dict(kv.split("=", 1) for kv in s.split(";") if "=" in kv)


class Sentence:
    def __init__(self, words, upos, heads, rels, feats, sure=None):
        self.w, self.u, self.h, self.r = words, upos, heads, rels
        # Each word's chance of its head and of its relation (a parser's
        # graph); 1 for a given parse.
        self.sure = sure or [(1.0, 1.0)] * len(words)
        self.f = [feats_of(x) for x in feats]
        self.n = len(words)
        self.kids = collections.defaultdict(list)
        for i, h in enumerate(heads, 1):
            self.kids[h].append(i)

    def rel(self, i):
        return self.r[i - 1]

    def base(self, i):
        return self.r[i - 1].split(":")[0]

    def span(self, i):
        """The words of i's subtree: first and last (1-based)."""
        lo = hi = i
        stack, seen = [i], {i}
        while stack:
            j = stack.pop()
            lo, hi = min(lo, j), max(hi, j)
            for k in self.kids[j]:
                if k not in seen:  # a parser's graph may have a cycle
                    seen.add(k)
                    stack.append(k)
        return lo, hi

    def finite(self, i):
        f = self.f[i - 1]
        return self.u[i - 1] in ("VERB", "AUX") and f.get("VerbForm") == "Fin"

    def clause(self, i):
        """i heads a clause: a finite verb, or a predicate with a subject or
        a copula, or introduced by a subordinating word."""
        if self.finite(i):
            return True
        kids = self.kids[i]
        if any(self.base(k) in ("nsubj", "cop", "mark", "aux", "expl") for k in kids):
            return True
        return self.u[i - 1] in ("ADJ", "VERB") and self.f[i - 1].get("Variant") == "Short"

    def chance(self, i):
        """How sure the graph is of the link that put a mark: word i's head
        and relation."""
        h, r = self.sure[i - 1]
        return h * r

    def marks(self):
        """Commas before each word (index 1..n) the rules put: the rule and
        the word whose link put it."""
        put = {}

        def around(lo, hi, why):
            if lo > 1:
                put.setdefault(lo, (why, i))
            if hi < self.n:
                put.setdefault(hi + 1, (why, i))

        for i in range(1, self.n + 1):
            b, rel, f, word = self.base(i), self.rel(i), self.f[i - 1], self.w[i - 1]
            lo, hi = self.span(i)
            head = self.h[i - 1]
            if b == "advcl" and f.get("VerbForm") == "Conv":
                around(lo, hi, "gerund")
            elif b == "acl" and f.get("VerbForm") == "Part":
                if lo > head:
                    around(lo, hi, "participle after")
            elif rel == "acl:relcl" or (b == "acl" and self.clause(i)):
                around(lo, hi, "relative")
            elif b in ("advcl", "ccomp") and self.clause(i):
                around(lo, hi, b)
            elif b == "vocative":
                around(lo, hi, "address")
            # Set off by its link, not by the word: «очевидно» as a parenthesis
            # (parataxis) has its commas, as an adverb (advmod) none.
            elif (b == "discourse" and word in INTERJECTIONS) or (b == "parataxis" and word in APART):
                around(lo, hi, "parenthetical")
            elif b == "parataxis" and lo > head and SAYING.match(word):
                around(lo, hi, "said")  # «…, сообщил он»
            elif b == "parataxis":
                pass  # an aside: dash, colon, brackets
            elif b in ("obl", "advcl") and any(self.w[k - 1] in SET_OFF for k in self.kids[i]):
                around(lo, hi, "несмотря на")
            elif b == "conj":
                ccs = [k for k in self.kids[i] if self.base(k) == "cc" and k < i]
                # A joining word right before the conjunct, whatever it hangs
                # on (a parse may hang «и» on the first conjunct).
                if not ccs and lo > 1 and self.u[lo - 2] == "CCONJ":
                    ccs = [lo - 1]
                cc = ccs[0] if ccs else None
                start = cc if cc else lo
                if cc and self.w[cc - 1] in ADVERSATIVE and self.w[cc - 1] not in JOINING:
                    put.setdefault(start, ("а/но", i))
                elif cc is None:
                    put.setdefault(start, ("listed", i))
                elif self.clause(i) and self.clause(head) and \
                        any(self.base(k) == "nsubj" for k in self.kids[i]) and \
                        any(self.base(k) == "nsubj" for k in self.kids[head]):
                    put.setdefault(start, ("clauses и", i))
                elif any(self.base(k) == "cc" and self.w[k - 1] == self.w[cc - 1] and k < head
                         for k in self.kids[head]):
                    put.setdefault(start, ("и…, и…", i))
            elif b == "appos":
                pass  # set off by a dash or brackets as often as by commas
        # Set expressions standing apart: the phrase, to the end of the
        # phrase its last word heads («по данным обсерватории»).
        for start in range(1, self.n + 1):
            for phrase in PAREN_PHRASES:
                k = len(phrase)
                if self.w[start - 1:start - 1 + k] != phrase:
                    continue
                last = start + k - 1
                hi = max(self.span(last)[1], last) if phrase[0] == "по" and k == 2 else last
                if start > 1:
                    put.setdefault(start, ("set phrase", last))
                if hi < self.n:
                    put.setdefault(hi + 1, ("set phrase", last))
                break
        return put


QUESTION = {"ли", "разве", "неужели", "неужто", "что-ли"}
# Question words (as the root clause's own words, not a subordinate's
# «как только», «когда он пришел»).
ASKING = {"кто", "что", "где", "куда", "откуда", "когда", "почему", "зачем", "отчего", "как",
          "сколько", "насколько", "какой", "какая", "какое", "какие", "каков", "какова", "чей",
          "чья", "чье", "чьи", "каким", "какую", "какого", "какому", "каком", "каких", "кого",
          "кому", "кем", "ком", "чего", "чему", "чем", "чего-нибудь"}


def dashes(self):
    """Dashes the links put: between a subject and a predicate noun with no
    verb («Москва — столица», «Москва — это столица»), between an
    infinitive subject and an infinitive predicate («жить — родине
    служить»). A quote's frame takes a dash too, but whether a text writes
    its dialogue with dashes is the writer's, not the graph's."""
    put = {}
    for i in range(1, self.n + 1):
        kids = self.kids[i]
        if any(self.base(k) in ("cop", "aux") for k in kids):
            continue
        subj = [k for k in kids if self.base(k) == "nsubj" and k < i]
        if not subj:
            continue
        k = subj[-1]
        f_i, f_k = self.f[i - 1], self.f[k - 1]
        nominal = self.u[i - 1] in ("NOUN", "PROPN", "NUM") and f_i.get("Case") == "Nom" and \
            self.u[k - 1] in ("NOUN", "PROPN") and f_k.get("Case") == "Nom"
        infinitives = f_i.get("VerbForm") == "Inf" and f_k.get("VerbForm") == "Inf"
        if nominal or infinitives:
            lo = self.span(i)[0]
            # «это» before the predicate: the dash before «это».
            eto = [j for j in kids if self.w[j - 1] == "это" and j < i]
            at = min([lo] + eto)
            if at > self.span(k)[1]:
                put.setdefault(at, ("zero copula" if nominal else "infinitives", i))
    return put


def end_mark(self):
    """How the sentence ends, by its root clause: a question word or «ли»
    in it — «?»; else «.». (An exclamation is the writer's voice.)"""
    root = next((i for i, h in enumerate(self.h, 1) if h == 0), None)
    if root is None:
        return "."
    def asking(j):
        return self.w[j - 1] in ASKING and self.base(j) not in ("mark", "cc", "fixed") and \
            "Int" in self.f[j - 1].get("PronType", "Int")

    clause = [root] + self.kids[root]
    for j in clause:
        if self.w[j - 1] in QUESTION or (j != root and asking(j)):
            return "?"
        # A question word one step down («в каком году…», «сколько лет…»).
        if self.base(j) in ("obl", "obj", "nsubj", "advmod", "nmod", "iobj") and \
                any(asking(k) for k in self.kids[j]):
            return "?"
    # A question word as the predicate itself («кто он», «как дела»).
    return "?" if asking(root) and root == 1 else "."


Sentence.dashes = dashes
Sentence.end_mark = end_mark


def read(path):
    for line in open(path, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        if len(f) < 8:
            continue
        words = f[1].split()
        if not len(words) == len(f[3].split()) == len(f[4].split()) == len(f[6].split()):
            continue
        sure = [tuple(map(float, x.split(","))) for x in f[8].split()] if len(f) > 8 else None
        yield f[0], Sentence(words, f[2].split(), [int(x) for x in f[3].split()], f[4].split(),
                             f[5].split("|"), sure), f[6].split()


def read_ends(path):
    ends = {}
    for line in open(path, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        if len(f) >= 8:
            ends[f[0]] = f[7]
    for sid, s, before in read(path):
        yield sid, s, before, ends.get(sid, ".")


ap = argparse.ArgumentParser()
ap.add_argument("--eval")
ap.add_argument("--show", type=int, default=0)
ap.add_argument("--rule", default="")
args = ap.parse_args()
if args.eval:
    LATIN = re.compile(r"[a-z]")
    tp = fp = fn = other = 0
    by_rule = collections.defaultdict(lambda: [0, 0, 0])  # fired, right, at another mark
    missed = collections.Counter()
    shown = collections.defaultdict(list)
    levels = [0.0, 0.5, 0.7, 0.8, 0.9, 0.95, 0.98]
    at_level = {t: [0, 0] for t in levels}  # put, right
    gold_total = 0
    for sid, s, before in read(args.eval):
        if any(LATIN.search(w) for w in s.w):
            continue
        put = s.marks()
        for i in range(2, s.n + 1):
            gold = before[i - 1]
            comma = "," in gold
            gold_total += comma
            why = put.get(i)
            if why:
                why, trigger = why
                p = s.chance(trigger)
                for t in levels:
                    if p >= t:
                        at_level[t][0] += 1
                        at_level[t][1] += comma
                st = by_rule[why]
                st[0] += 1
                if comma:
                    st[1] += 1
                    tp += 1
                elif gold != "_":
                    st[2] += 1
                    other += 1
                else:
                    fp += 1
                    if (not args.rule or args.rule == why) and len(shown["+" + why]) < args.show:
                        shown["+" + why].append(f"{' '.join(s.w[:i - 1])} ‸ {' '.join(s.w[i - 1:])}  [{s.rel(i)}]")
            elif comma:
                fn += 1
                missed[s.rel(i)] += 1
                if len(shown["-missed"]) < args.show and not args.rule:
                    shown["-missed"].append(f"{' '.join(s.w[:i - 1])} ‸ {' '.join(s.w[i - 1:])}  [{s.rel(i)}]")
    put_all = tp + fp + other
    put_all = max(put_all, 1)
    print(f"commas: put {put_all}, right {tp} ({tp / put_all:.1%}), at another mark {other}, "
          f"wrong {fp}; the text's commas found {tp / max(tp + fn, 1):.1%} of {tp + fn}")
    print("by how sure the graph is of the link: put / right / the text's commas found")
    for t in levels:
        n, ok = at_level[t]
        if n:
            print(f"  ≥ {t:.2f}  {n:>6}  {ok / n:6.1%}  {ok / max(gold_total, 1):6.1%}")
    for why, (n, ok, oth) in sorted(by_rule.items(), key=lambda x: -x[1][0]):
        print(f"  {why:<18} {n:>6}  right {ok / n:6.1%}  at another mark {oth / n:5.1%}")
    print("missed, by the relation of the word after the comma:",
          ", ".join(f"{r} {c}" for r, c in missed.most_common(12)))
    # Dashes and sentence ends, the same way.
    d_put = d_right = d_other = d_gold = 0
    ends = collections.Counter()
    for sid, s, before, end in read_ends(args.eval):
        if any(LATIN.search(w) for w in s.w):
            continue
        put = s.dashes()
        for i in range(2, s.n + 1):
            gold = before[i - 1]
            d_gold += "—" in gold
            if i in put:
                d_put += 1
                d_right += "—" in gold
                d_other += gold != "_" and "—" not in gold
        want = next((c for c in end if c in ".?!"), ".")
        ends[(want, s.end_mark())] += 1
    if d_put:
        print(f"\ndashes: put {d_put}, right {d_right} ({d_right / d_put:.1%}), at another mark {d_other}; "
              f"the text's dashes found {d_right / max(d_gold, 1):.1%} of {d_gold}")
    total = sum(ends.values())
    right = sum(n for (w, g), n in ends.items() if w == g)
    print(f"sentence ends right: {right / total:.1%} of {total}")
    for mark in ".?!":
        want = sum(n for (w, g), n in ends.items() if w == mark)
        got = sum(n for (w, g), n in ends.items() if g == mark)
        ok = ends[(mark, mark)]
        if want or got:
            print(f"  {mark}  the text's {want}, put {got}, right {ok} "
                  f"({ok / max(got, 1):.1%} of those put, {ok / max(want, 1):.1%} of the text's)")
    for k, v in shown.items():
        print(f"\n== {k}")
        for e in v:
            print("  " + e)
