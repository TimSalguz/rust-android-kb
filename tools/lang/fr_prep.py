#!/usr/bin/env python3
"""French language pack: the French-specific steps of tools/lang/fr.sh.

Usage: tools/lang/fr_prep.py forms    GRAMMALECTE.txt > forms.tsv
       tools/lang/fr_prep.py freq     GRAMMALECTE.txt fr_full.txt LEIPZIG.tar.gz \\
                                      --freq-out freq.txt --allowed-out allowed.txt \\
                                      [--extra-out hyphenated.tsv] [--min-hyphenated 20]
       tools/lang/fr_prep.py tatoeba  fra_sentences.tsv.bz2 OUT.tsv.bz2
       tools/lang/fr_prep.py leipzig  LEIPZIG.tar.gz OUT.tsv.bz2
       tools/lang/fr_prep.py proper   GRAMMALECTE.txt LEIPZIG.tar.gz lexicon.tsv > proper.tsv
       tools/lang/fr_prep.py coverage fra_sentences.tsv.bz2 lexicon.tsv

Elision. The elided clitics c' ç' d' j' l' m' n' s' t' qu' jusqu' lorsqu'
puisqu' quoiqu' are words of their own, written with the apostrophe, and the
word after them is a separate word: «l'homme» = «l'» + «homme», «qu'est-ce»
= «qu'» + «est-ce». Everything else keeps its apostrophe inside
(aujourd'hui, quelqu'un, presqu'île, prud'homme, entr'ouvert, p'tit). No
lexicon word starts with a clitic + apostrophe (Grammalecte's few such words
lose the clitic: c'est-à-dire → est-à-dire, m'as-tu-vu → as-tu-vu), so the
split is one regular expression, the same for the corpora, the bigrams and
the keyboard. ’ and ʼ are written '.

forms: every form of the Grammalecte lexicon (MPL 2.0) as `form<TAB>lemma`,
lowercase NFC, for make_lexicon.py --forms — without the spellings only the
1990 reform uses (connaitre, ile, gout: typed that way they are almost always
a missing accent), the doubtful ones (sub-dictionary X), the «aimè-je» forms,
Grammalecte's error entries, symbols (kVA, the letter names el, em — common
units like km, kg stay), roman numerals, single letters other than a à y ô,
two-letter capitalized codes that are not acronyms, and capitalized words that
are an accentless spelling of a more frequent word (CA/ça, Grace/grâce). A
name or an acronym is its own lemma (the unit VA shares nothing with «va»).

freq: the subtitle counts (hermitdave FrequencyWords), lowercase NFC, and the
list of words the lexicon may take from them (make_lexicon.py --words): every
Grammalecte form, the lexicalized apostrophe words the subtitles split
(quelqu' + un → quelqu'un, shared out by the Leipzig counts), and the
hyphenated verb + pronoun forms Grammalecte leaves to its tokenizer (est-ce,
dis-moi, a-t-il, allez-vous-en, ce jour-là), from 20 occurrences. Other
subtitle words outside Grammalecte are not taken: at ≥ 1000 occurrences they
are still TV-series names, OCR errors (lci, iui) and accentless spellings
(etre, cest) — slang.tsv adds the real ones.

tatoeba, leipzig: the sentences with the clitics split off, in the Tatoeba
export format (`id<TAB>fra<TAB>text`, bz2) for build_bigrams.py and
export_observations.py. Leipzig: sentences only (its own word pairs keep
«l'homme» whole), spam (locksmiths, plumbers, …) and repeated templates
dropped.

proper: `word<TAB>1` (Capitalized) / `word<TAB>2` (ALL CAPS) for the words
Grammalecte writes only with capitals (Paris, Macron, SNCF, Pâques) — unless
the web corpus writes them in lowercase in the middle of a sentence at least a
fifth of the time — plus words Grammalecte has both ways that the corpus
capitalizes ≥ 90 % mid-sentence (Noël, Internet stays).

coverage: share of held-out Tatoeba tokens (ids divisible by 50) in the
lexicon, overall and for tokens with accents.
"""
import argparse
import bz2
import re
import sys
import tarfile
import unicodedata
from collections import Counter, defaultdict

LETTERS = "a-zß-öø-ÿœ"
WORD = re.compile(rf"[{LETTERS}'-]*[{LETTERS}][{LETTERS}'-]*")
CLITIC = re.compile(r"(?i)(jusqu|lorsqu|puisqu|quoiqu|qu|[cçdjlmnst])'(?=[^\W\d_])")
RUN = re.compile(r"[^\W\d_](?:[^\W\d_]|['-](?=[^\W\d_]))*'?")
ACCENTED = re.compile("[à-öø-ÿœ]")


def norm(s):
    s = unicodedata.normalize("NFC", s)
    return s.replace("’", "'").replace("ʼ", "'").replace(" ", " ").replace(" ", " ")


def split_elision(text):
    """«L'homme qu'il voit» → «L' homme qu' il voit»."""
    def one(m):
        run = m.group(0)
        out = []
        while True:
            c = CLITIC.match(run)
            if not c:
                break
            out.append(c.group(0))
            run = run[c.end():]
        out.append(run)
        return " ".join(out)
    return RUN.sub(one, norm(text))


def bare(w):
    return "".join(c for c in unicodedata.normalize("NFD", w) if not unicodedata.combining(c))


def grammalecte(path):
    """Rows (form, lemma, tags, notes, subdict, total count) of the Grammalecte lexicon."""
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) < 16 or not p[0].isdigit():
                continue
            yield (norm(p[2]), norm(p[3]), [t.rstrip("!") for t in p[4].split()], p[7].split(),
                   p[10].split("/")[-1], int(p[15] or 0))


ORDINAL = re.compile(r"^(?:[IVXLC]{2,}(?:e|er|re|ème|es|ers|res)?|[IVX](?:e|er|re|ème|es|ers|res))$")
SINGLE = {"a", "à", "y", "ô"}
# Unit symbols people type; the other symbols (letter names el, em; kVA,
# dacal, zvar …) stay out.
UNITS = {"km", "kg", "cm", "mm", "ml", "cl", "dl", "mg", "ha", "kcal", "ppm", "dpi", "px", "lb", "oz", "atm"}
# Two-letter capitalized words that are not acronyms (sigles) or
# abbreviations are mostly codes: typed in lowercase they are typos.
SHORT = {"ok", "ia", "qg", "dj", "wc", "cc", "mn"}


def keep_row(form, tags, notes, sub):
    if sub in ("R", "X") or "err" in tags or "1isg" in tags or "nbro" in tags or ORDINAL.match(form):
        return False
    low = form.lower()
    if "symb" in notes and low not in UNITS:
        return False
    if len(low) == 1 and low not in SINGLE:
        return False
    if len(low) == 2 and form != low and not ({"sig", "abty", "abr"} & set(notes)) and low not in SHORT:
        return False
    return bool(WORD.fullmatch(low))


def unclitic(w):
    """Grammalecte's c'est-à-dire, m'as-tu-vu → est-à-dire, as-tu-vu."""
    while (m := CLITIC.match(w)) and len(w) > m.end():
        w = w[m.end():]
    return w


PARTICLES = {"de", "du", "des", "la", "le", "les", "en", "sur", "sous", "et", "lès", "aux", "au"}


def casing(orig):
    """lower / cap / upper / compound (Saint-Étienne) / mixed (iPhone, McDonald)."""
    low = orig.lower()
    if orig == low:
        return "lower"
    if orig == orig.upper() and len(orig) > 1:
        return "upper"
    if orig[0].isupper() and orig[1:] == orig[1:].lower():
        return "cap"
    if "-" in orig and orig[0].isupper():
        return "compound"
    return "mixed"


def load(path):
    """Grammalecte forms kept for the lexicon: form → {total, rows (tag sets),
    lemmas, kinds (casings)}. Capitalized-only forms that are an accentless
    spelling of a more frequent word (CA → ça, Grace → grâce, Pres → près)
    are left out: typed in lowercase they are the missing-accent typo."""
    forms = {}
    for form, lemma, tags, notes, sub, total in grammalecte(path):
        if not keep_row(form, tags, notes, sub):
            continue
        low = form.lower()
        if low in ("quelqu'", "presqu'"):  # only in quelqu'un, presqu'île
            continue
        w = unclitic(low)
        e = forms.setdefault(w, {"total": 0, "rows": set(), "lemmas": set(), "kinds": set()})
        e["total"] = max(e["total"], total)
        e["rows"].add(frozenset(tags))
        orig = form[len(form) - len(w):]  # D'Holbach → Holbach
        kind = casing(orig[0].upper() + orig[1:] if w != low and form[0].isupper() else orig)
        e["kinds"].add(kind)
        # A name or an acronym is its own lemma: VA's forms are not the verb «va».
        e["lemmas"].add(unclitic(lemma.lower()) if kind == "lower" else "=" + orig)
    common = defaultdict(int)
    for w, e in forms.items():
        if "lower" in e["kinds"]:
            common[bare(w)] = max(common[bare(w)], e["total"])
    for w in [w for w, e in forms.items() if "lower" not in e["kinds"] and common.get(bare(w), -1) > e["total"]]:
        del forms[w]
    return forms


def forms_cmd(args):
    for w, e in load(args.grammalecte).items():
        for lem in sorted(e["lemmas"]):
            print(f"{w}\t{lem}")


def leipzig_words(path, lower=True):
    counts = Counter()
    with tarfile.open(path) as tar:
        m = next(m for m in tar.getmembers() if m.name.endswith("-words.txt"))
        for raw in tar.extractfile(m):
            p = raw.decode("utf-8", "replace").rstrip("\n").split("\t")
            if len(p) >= 3 and p[2].isdigit():
                w = norm(p[1])
                counts[w.lower() if lower else w] += int(p[2])
    return counts


# What follows a verb (or a noun) after a hyphen: dis-moi, a-t-il, ce jour-là.
SUBJECT = {"je", "tu", "il", "elle", "on", "nous", "vous", "ils", "elles", "ce"}
OBJECT = {"moi", "toi", "lui", "leur", "le", "la", "les", "en", "y", "nous", "vous"}
PERSON = {"je": "1sg", "tu": "2sg", "il": "3sg", "elle": "3sg", "on": "3sg", "ce": "3sg",
          "nous": "1pl", "vous": "2pl", "ils": "3pl", "elles": "3pl"}
FINITE = {"ipre", "iimp", "ipsi", "ifut", "cond", "spre", "simp"}


def hyphenated_ok(w, forms, tags_of):
    """A verb with its pronouns (dis-moi, a-t-il, vas-y, allez-vous-en, est-ce)
    or a word with là/ci (ce jour-là) or ex- (ex-femme) — not a stutter (je-je)
    or a misspelling (est-ce-que, etes-vous, est-t-elle, amuses-toi)."""
    head, *rest = w.split("-")
    if head not in forms or (len(head) < 2 and head != "a") or not rest or len(rest) > 3 or rest[0] == head:
        return False
    rows = tags_of[head]
    tags = set().union(*rows)
    if head == "ex":
        return "-".join(rest) in forms
    if rest in (["là"], ["ci"]):  # ce jour-là, celui-ci (not laissez-là)
        return bool(tags & {"nom", "adj", "prodem", "detdem"})
    if rest[0] == "t":  # a-t-il, parle-t-elle: only after a vowel
        return head[-1] in "ae" and len(rest) == 2 and rest[1] in ("il", "elle", "on") and "3sg" in tags
    if rest[0] in ("il", "elle", "on"):  # est-il, prend-on
        return head[-1] in "td" and "3sg" in tags and len(rest) == 1
    if rest[0] in SUBJECT and all(p in OBJECT for p in rest[1:]):  # avez-vous, allez-vous-en (not ose-tu)
        return any(PERSON[rest[0]] in r and r & FINITE for r in rows) or (
            rest[0] in ("nous", "vous") and "impe" in tags)
    if all(p in OBJECT for p in rest):  # imperative: dis-le-moi, vas-y, penses-y
        return "impe" in tags or (rest[0] in ("en", "y") and head.endswith("s")
                                   and any("impe" in r for r in tags_of.get(head[:-1], ())))
    return False


def freq_cmd(args):
    entries = load(args.grammalecte)
    forms = {w: e["total"] for w, e in entries.items()}
    tags_of = {w: e["rows"] for w, e in entries.items()}
    lexicalized = [w for w in forms if "'" in w[:-1]]  # aujourd'hui, quelqu'un
    freq = Counter()
    with open(args.freq, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            if len(p) == 2 and p[1].isdigit():
                freq[norm(p[0]).lower()] += int(p[1])
    # oe for œ (coeur 44598, cœur 43691): the count goes to the right spelling.
    merged = 0
    for w, c in list(freq.items()):
        for a, b in (("oe", "œ"), ("ae", "æ")):
            if a in w and w not in forms and w.replace(a, b) in forms:
                freq[w.replace(a, b)] += c
                merged += 1
    web = leipzig_words(args.leipzig)
    # The subtitles split at every apostrophe: quelqu' + un. Give the whole
    # words their prefix's count, shared by their web counts.
    by_prefix = defaultdict(list)
    for w in lexicalized:
        head = w[: w.index("'") + 1]
        if head in freq and w not in freq:
            by_prefix[head].append(w)
    for head, ws in by_prefix.items():
        shares = [web.get(w, 0) + 1 for w in ws]
        for w, s in zip(ws, shares):
            freq[w] = max(1, round(freq[head] * s / sum(shares)))
    extra = []
    for w, c in freq.items():
        if w not in forms and "-" in w and WORD.fullmatch(w) and c >= args.min_hyphenated \
                and hyphenated_ok(w, forms, tags_of) \
                and not (w.endswith("-là") and freq[w[:-1] + "a"] > c):  # tape-là: tape-la
            extra.append((c, w))
    extra.sort(reverse=True)
    with open(args.freq_out, "w", encoding="utf-8") as f:
        for w, c in freq.most_common():
            f.write(f"{w} {c}\n")
    with open(args.allowed_out, "w", encoding="utf-8") as f:
        for w in sorted(set(forms) | {w for _, w in extra}):
            f.write(w + "\n")
    if args.extra_out:
        with open(args.extra_out, "w", encoding="utf-8") as f:
            for c, w in extra:
                f.write(f"{w}\t{c}\n")
    print(f"freq: {len(forms)} Grammalecte forms, {len(extra)} hyphenated subtitle words beyond "
          f"them, {sum(len(v) for v in by_prefix.values())} split apostrophe words, {merged} oe → œ",
          file=sys.stderr)


def write_sentences(rows, out_path):
    n = 0
    with bz2.open(out_path, "wt", encoding="utf-8") as out:
        for sid, text in rows:
            out.write(f"{sid}\tfra\t{split_elision(text)}\n")
            n += 1
    return n


def tatoeba_cmd(args):
    def rows():
        with bz2.open(args.src, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) == 3:
                    yield p[0], p[2]
    print(f"{args.out}: {write_sentences(rows(), args.out)} sentences", file=sys.stderr)


SPAM = re.compile(r"(?i)serrur|vitrer|d[ée]pann|plombi|coffre.fort|24h/24|24h/7|7j/7|devis gratuit|"
                  r"chauffagiste|[ée]lectricien|d[ée]m[ée]nag|climatisation|porte blind|rideau m[ée]tal|"
                  r"varnish|cette page a été consultée|newsletter|cookies?\b|inscrivez-vous|cliquez ici|"
                  r"casino|viagra|cialis|pr[êe]t personnel|rachat de cr[ée]dit")


def leipzig_cmd(args):
    seen = set()
    dropped = Counter()

    def rows():
        with tarfile.open(args.src) as tar:
            m = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
            for raw in tar.extractfile(m):
                sid, _, text = raw.decode("utf-8", "replace").rstrip("\n").partition("\t")
                if SPAM.search(text):
                    dropped["spam"] += 1
                    continue
                key = re.sub(r"[\d\W]+", " ", text.lower()).strip()
                if key in seen:
                    dropped["repeated"] += 1
                    continue
                seen.add(key)
                yield sid, text
    n = write_sentences(rows(), args.out)
    print(f"{args.out}: {n} sentences, dropped {dict(dropped)}", file=sys.stderr)


def proper_cmd(args):
    with open(args.lexicon, encoding="utf-8") as f:
        lexicon = {line.split("\t", 1)[0] for line in f}
    kinds = {w: e["kinds"] for w, e in load(args.grammalecte).items()}
    # Spellings in the web corpus, not at the start of a sentence or quote.
    mid = defaultdict(Counter)
    with tarfile.open(args.leipzig) as tar:
        m = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(m):
            text = split_elision(raw.decode("utf-8", "replace").partition("\t")[2])
            for t in RUN.finditer(text):
                before = text[: t.start()].rstrip()
                if not before or before[-1] in ".!?:;«»\"“”()[]—–-•*|/>" or before[-1].isdigit():
                    continue
                w = t.group(0)
                key = w.lower()
                if key in kinds:
                    mid[key][casing(w)] += 1
    out = {}
    for key, ks in kinds.items():
        if key not in lexicon or len(key) < 2 or "'" in key or ks & {"mixed", "compound"}:
            continue
        seen = mid.get(key, Counter())
        n = sum(seen.values())
        if "lower" in ks:
            # Both in Grammalecte (Noël/noël, Internet/internet): the corpus decides.
            top = max(("cap", "upper"), key=lambda k: seen[k])
            if n >= 20 and seen[top] >= 0.9 * n:
                out[key] = 2 if top == "upper" else 1
            continue
        if n >= 10 and seen["lower"] >= 0.2 * n:
            continue  # written in lowercase too
        if "upper" in ks and ("cap" not in ks or seen["upper"] >= seen["cap"]):
            out[key] = 2
        else:
            out[key] = 1
    for key in sorted(out):
        print(f"{key}\t{out[key]}")
    print(f"proper: {sum(v == 1 for v in out.values())} Capitalized, "
          f"{sum(v == 2 for v in out.values())} ALL CAPS", file=sys.stderr)


def coverage_cmd(args):
    with open(args.lexicon, encoding="utf-8") as f:
        lexicon = {line.split("\t", 1)[0] for line in f}
    tot = Counter()
    missing = Counter()
    with bz2.open(args.src, "rt", encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) != 3 or not p[0].isdigit() or int(p[0]) % 50:
                continue
            for t in RUN.findall(split_elision(p[2]).lower()):
                t = t.strip("-")
                if not t or not WORD.fullmatch(t):
                    continue
                acc = bool(ACCENTED.search(t))
                ok = t in lexicon
                hy = ok or ("-" in t and all(x in lexicon or x == "t" for x in t.split("-")))
                tot["all"] += 1
                tot["all_ok"] += ok
                tot["all_hy"] += hy
                if acc:
                    tot["acc"] += 1
                    tot["acc_ok"] += ok
                if "'" in t:
                    tot["apo"] += 1
                    tot["apo_ok"] += ok
                if not ok:
                    missing[t] += 1
    pct = lambda a, b: f"{100.0 * tot[a] / max(1, tot[b]):.2f} %"
    print(f"held-out Tatoeba tokens: {tot['all']}; in lexicon {pct('all_ok', 'all')} "
          f"({pct('all_hy', 'all')} counting X-Y as X + Y); with accents {tot['acc']}: "
          f"{pct('acc_ok', 'acc')}; with an apostrophe {tot['apo']}: {pct('apo_ok', 'apo')}")
    print("most frequent missing:", " ".join(f"{w}:{c}" for w, c in missing.most_common(args.show)))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("forms")
    p.add_argument("grammalecte")
    p.set_defaults(fn=forms_cmd)
    p = sub.add_parser("freq")
    p.add_argument("grammalecte")
    p.add_argument("freq")
    p.add_argument("leipzig")
    p.add_argument("--min-hyphenated", type=int, default=20)
    p.add_argument("--freq-out", required=True)
    p.add_argument("--allowed-out", required=True)
    p.add_argument("--extra-out")
    p.set_defaults(fn=freq_cmd)
    for name, fn in (("tatoeba", tatoeba_cmd), ("leipzig", leipzig_cmd)):
        p = sub.add_parser(name)
        p.add_argument("src")
        p.add_argument("out")
        p.set_defaults(fn=fn)
    p = sub.add_parser("proper")
    p.add_argument("grammalecte")
    p.add_argument("leipzig")
    p.add_argument("lexicon")
    p.set_defaults(fn=proper_cmd)
    p = sub.add_parser("coverage")
    p.add_argument("src")
    p.add_argument("lexicon")
    p.add_argument("--show", type=int, default=60)
    p.set_defaults(fn=coverage_cmd)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
