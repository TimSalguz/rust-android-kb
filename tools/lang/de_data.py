#!/usr/bin/env python3
"""German language pack helpers for tools/lang/de.sh (stdlib only).

  de_data.py lt JAR > lt.tsv
      Every entry of LanguageTool's german-pos-dict (Morfologik CFSA2 automaton
      german.dict inside the jar): `form<TAB>lemma<TAB>tag`.
  de_data.py kaikki GZ > kaikki.tsv
      Words and inflection-table forms of the German entries of the English
      Wiktionary (kaikki.org extraction): `form<TAB>lemma<TAB>pos<TAB>ok|bad<TAB>entry|form`,
      bad = superseded (daß), Swiss (Strasse), obsolete, regional or
      misspelled spellings; entry = the word's own entry, form = a table form.
  de_data.py forms --lt lt.tsv --kaikki kaikki.tsv --freq de_full.txt
                   [--english WORDLIST ...] [--names WORDLIST ...] [--casing casing.tsv]
                   [--corpus SENTENCES ...] [--drop-unattested]
                   --forms forms.tsv --readings readings.tsv --extra extra.tsv
      forms.tsv: `form<TAB>lemma key`, lowercase, for make_lexicon.py
      --words/--forms: the union of both morphological sources (current
      spellings only), plus compounds attested in the subtitles (split into
      known words, inflected like their last word); a lemma key is lemma|POS
      (a form shared by unrelated lemmas stands alone, so homographs don't
      inflate the share of unseen forms). readings.tsv: `form<TAB>flags` — L
      lowercase, C capitalized, A all-caps readings in the sources.
      extra.tsv: `word<TAB>count` — frequent corpus words the sources lack
      (names, loanwords), X's contractions, capitals-only abbreviations and
      ss spellings with counts corrected by the web corpus.
  de_data.py casing SENTENCES... > casing.tsv
      Case of non-sentence-initial occurrences: `word<TAB>lower<TAB>Cap<TAB>CAPS`.
  de_data.py proper --lexicon L --readings R --casing C > proper.tsv
      `word<TAB>1` (Capitalized) / `word<TAB>2` (ALL CAPS): nouns and names
      that are not also common in lowercase.
  de_data.py coverage --lexicon L TATOEBA.bz2
      Lexicon coverage of held-out Tatoeba tokens (ids divisible by 50).

SENTENCES: Tatoeba `*_sentences.tsv.bz2` (held-out ids skipped) or a Leipzig
`*.tar.gz` (its -sentences.txt).
"""
import argparse
import bz2
import gzip
import json
import re
import sys
import tarfile
import unicodedata
import zipfile
from collections import Counter, defaultdict

# Letters of the keyboard's alphabet (kbcore CHARSET): a-z, Latin-1, œ.
L = "a-zß-öø-ÿœ"
WORD = re.compile(f"[{L}'-]*[{L}][{L}'-]*")
EDGE = re.compile(r"^['-]|['-]$|--|''")
TOKEN = re.compile(r"[\w'’-]+|[^\w\s]")
HOLDOUT = 50


def nfc(s):
    return unicodedata.normalize("NFC", s)


def ok_word(w):
    return WORD.fullmatch(w) is not None and not EDGE.search(w)


def sentences(path):
    """Sentences of a Tatoeba export (held-out ids skipped) or a Leipzig archive."""
    if path.endswith(".bz2"):
        with bz2.open(path, "rt", encoding="utf-8") as f:
            for line in f:
                p = line.rstrip("\n").split("\t")
                if len(p) == 3 and p[0].isdigit() and int(p[0]) % HOLDOUT:
                    yield nfc(p[2])
        return
    with tarfile.open(path) as tar:
        member = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
        for raw in tar.extractfile(member):
            yield nfc(raw.decode("utf-8", "replace").rstrip("\n").split("\t", 1)[-1])


# ---------------------------------------------------------------- lt
def cmd_lt(args):
    """Walk the CFSA2 automaton (morfologik-fsa CFSA2.java) and print its
    sequences, decoding the SUFFIX-encoded lemma (first byte: 'A' + bytes to
    cut from the form, then the bytes to append)."""
    data = zipfile.ZipFile(args.jar).read("org/languagetool/resource/de/german.dict")
    assert data[:4] == b"\\fsa" and data[4] == 0xC6, "not a CFSA2 automaton"
    flags = int.from_bytes(data[5:7], "big")
    assert not flags & (1 << 8), "NUMBERS automata not supported"
    labels = data[8 : 8 + data[7]]
    arcs = data[8 + data[7] :]
    NEXT, LAST, FINAL, MASK = 0x80, 0x40, 0x20, 0x1F

    def skip_arc(o):
        f = arcs[o]
        o += 1 if f & MASK else 2
        if not f & NEXT:
            while arcs[o] & 0x80:
                o += 1
            o += 1
        return o

    def dest(a):
        if arcs[a] & NEXT:
            while not arcs[a] & LAST:
                a = skip_arc(a)
            return skip_arc(a)
        o = a + (1 if arcs[a] & MASK else 2)
        b = arcs[o]
        v, s = b & 0x7F, 7
        while b & 0x80:
            o += 1
            b = arcs[o]
            v |= (b & 0x7F) << s
            s += 7
        return v

    out = sys.stdout
    buf = bytearray(1024)
    stack = [(dest(0), 0)]
    n = 0
    while stack:
        a, d = stack.pop()
        while True:
            f = arcs[a]
            i = f & MASK
            buf[d] = labels[i] if i else arcs[a + 1]
            if f & FINAL:
                seq = bytes(buf[: d + 1])
                if not seq.startswith(b"#"):
                    form, enc, tag = seq.split(b"_", 2)
                    cut = (enc[0] - 65) & 0xFF
                    if cut == 255:
                        cut = len(form)
                    lemma = form[: len(form) - cut] + enc[1:]
                    out.write(f"{form.decode()}\t{lemma.decode()}\t{tag.decode().split()[0]}\n")
                    n += 1
            t = dest(a)
            if t:
                if not f & LAST:
                    stack.append((skip_arc(a), d))
                a, d = t, d + 1
                continue
            if f & LAST:
                break
            a = skip_arc(a)
    print(f"lt: {n} entries", file=sys.stderr)


# ---------------------------------------------------------------- kaikki
BAD_GLOSS = re.compile(
    r"^(formerly standard spelling|(obsolete|archaic|dated|eye dialect|censored|pronunciation|"
    r"nonstandard|misspelling|superseded)[\w ]* (spelling|form)|switzerland and liechtenstein "
    r"standard spelling|misspelling of|obsolete form of|archaic form of)", re.I)
BAD_TAGS = {"obsolete", "archaic", "misspelling", "Switzerland", "Liechtenstein",
            "Luxembourg", "Austria", "dialectal", "regional", "proscribed", "nonstandard",
            "Southern-Germany", "southern-Germany", "Northern-Germany", "Upper-German",
            "Bavaria", "Swiss-German", "Swiss", "error-unrecognized-form", "error-unknown-tag",
            "canonical", "hypercorrect", "pronunciation-spelling", "table-tags",
            "inflection-template", "class", "romanization", "dated", "poetic", "literary",
            "humorous", "vernacular", "Internet", "abbreviation", "auxiliary", "alternative"}
# An alternative spelling (alt-of sense) with one of these tags is not a word to keep.
ALT_BAD = {"obsolete", "archaic", "misspelling", "Switzerland", "Liechtenstein", "Luxembourg",
           "Austria", "nonstandard", "proscribed", "dialectal", "pronunciation-spelling"}
CASES = {"nominative", "genitive", "dative", "accusative", "singular", "plural"}
SKIP_POS = {"suffix", "prefix", "interfix", "infix", "circumfix", "character", "symbol",
            "punct", "phrase", "proverb", "prep_phrase"}


def cmd_kaikki(args):
    out = sys.stdout
    n = bad = 0
    with gzip.open(args.gz, "rt", encoding="utf-8") as f:
        for line in f:
            d = json.loads(line)
            word, pos = nfc(d["word"]), d.get("pos", "")
            if pos in SKIP_POS or " " in word:
                continue
            senses = d.get("senses") or [{}]
            is_bad = all(
                any(BAD_GLOSS.match(g) for g in s.get("glosses", [])[:1])
                or ("alt-of" in s.get("tags", []) and set(s.get("tags", [])) & ALT_BAD)
                or "misspelling" in s.get("tags", [])
                for s in senses
            )
            lemma = word
            for s in senses:
                for fo in s.get("form_of") or []:
                    lemma = nfc(fo.get("word", word))
                    break
                if lemma != word:
                    break
            status = "bad" if is_bad else "ok"
            out.write(f"{word}\t{lemma}\t{pos}\t{status}\tentry\n")
            n += 1
            bad += is_bad
            for fo in d.get("forms", []):
                form = nfc(fo.get("form", ""))
                tags = set(fo.get("tags", []))
                if not form or " " in form or tags & BAD_TAGS or "diminutive" in tags:
                    continue
                if pos == "noun" and not tags & CASES:
                    continue  # Chefin, Häuschen: another noun, not a form of this one
                out.write(f"{form}\t{word}\t{pos}\t{status}\tform\n")
    print(f"kaikki: {n} entries, {bad} superseded/regional spellings", file=sys.stderr)


# ---------------------------------------------------------------- forms
N, A, V, P, O = 1, 2, 4, 8, 16  # noun, adjective/participle, verb, name, other
LT_POS = {"SUB": N, "EIG": P, "ADJ": A, "PA1": A, "PA2": A, "VER": V}
KK_POS = {"noun": N, "name": P, "adj": A, "verb": V}
PARTICLES = set("""ab an auf aus bei da dar durch ein fort her hin hinter los mit nach neben
rein raus rüber runter rauf ran rum weg weiter wieder zu zurück zusammen über unter um vor
voran vorbei voraus entgegen gegen heim hoch fest frei statt teil hinein hinaus herein heraus
herum herunter herauf hinauf hinunter herüber hinüber hervor empor nieder dazu dabei davon
daran darauf daraus darein darum davor dahin daher drauf drin drum mal miss fehl kaputt
schief tot voll wach bereit gut schlecht hierher heraus ent emp zer ver be ge er un ur
super mega extra ultra anti pro ex vize""".split())
# Subtitle artifacts: hearing-impaired tags, release groups, color codes.
JUNK = {"sdh", "sdi", "uld", "sch", "subcentral", "dtv", "chwhite", "chyellow", "chcyan",
        "chgreen", "chred", "chmagenta", "untertitel", "subs", "sync", "corrected",
        "hälst", "immernoch", "garnicht", "garnichts", "nichtmal",
        "wieviel", "wieviele", "jedesmal", "mrs"}  # misspellings, pre-1996 spellings
# Chat abbreviations people write in lowercase: never capitalized.
CHAT = {"lol", "lg", "vg", "hdl", "hdgdl", "kp", "np", "omg", "wtf", "btw", "thx", "sry",
        "pls", "plz", "gg", "gn", "xd", "mfg", "sek", "rofl", "ggf", "usw", "etc", "zb", "dh",
        "bzw", "evtl", "vllt", "vlt", "iwie", "bspw", "eigtl"}
INSEPARABLE = {"be", "ge", "er", "ver", "zer", "ent", "emp", "miss", "un", "ur"}
NUMBERS = set("""ein eins zwei drei vier fünf sechs sieben acht neun zehn elf zwölf zwanzig
dreißig vierzig fünfzig hundert tausend million halb doppel""".split())


def lower_flag(form):
    if len(form) > 1 and form.isupper():
        return "A"
    if form[0].isupper() and form[1:] == form[1:].lower():
        return "C"
    if form == form.lower():
        return "L"
    return "M"


def read_freq(path):
    raw = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            if len(p) == 2 and p[1].isdigit():
                w = nfc(p[0].lower())
                raw[w] = max(raw.get(w, 0), int(p[1]))
    return raw


def variants(w):
    """Spellings of w with ß/umlauts restored: strasse → straße, fuer → für, fur → für."""
    out = set()
    for a, b in (("ss", "ß"), ("ae", "ä"), ("oe", "ö"), ("ue", "ü")):
        i = w.find(a)
        while i >= 0:
            out.add(w[:i] + b + w[i + 2 :])
            i = w.find(a, i + 1)
    if "ß" in w:
        out.add(w.replace("ß", "ss"))  # vergiß, paßt: pre-1996 spellings
    for i, ch in enumerate(w):
        if ch in "aou":
            out.add(w[:i] + {"a": "ä", "o": "ö", "u": "ü"}[ch] + w[i + 1 :])
    return out


def read_casing(path):
    casing = {}
    if path:
        with open(path, encoding="utf-8") as f:
            for line in f:
                w, lo, cap, caps = line.rstrip("\n").split("\t")
                casing[w] = (int(lo), int(cap), int(caps))
    return casing


def cmd_forms(args):
    raw = read_freq(args.freq)
    casing = read_casing(args.casing)
    # form → {(lemma, pos bit, lemma capitalized)}: a lemma is known by its
    # part of speech and case too, so "im" (in dem) and "IM" or the verb
    # "e-mailen" and the noun "E-Mailen" don't pool their counts.
    lemmas = defaultdict(set)
    flags = defaultdict(set)    # form → L/C/A/M
    pos = defaultdict(int)      # form → N|A|V|P|O
    by_lemma = defaultdict(set)  # (lemma, pos bit) → forms

    def add(form, lemma, p, flag):
        lem = lemma.lower()
        lemmas[form].add((lem, p, lemma[:1].isupper()))
        flags[form].add(flag)
        pos[form] |= p
        if p & (N | A | V):
            by_lemma[(lem, p)].add(form)

    # Wiktionary first: its clean spellings vouch for LanguageTool's.
    kk_ok, kk_bad, kk_entry = set(), set(), set()
    kk_rows = []
    bad_lemma, ok_lemma = set(), set()
    with open(args.kaikki, encoding="utf-8") as f:
        for line in f:
            form, lemma, p, status, kind = line.rstrip("\n").split("\t")
            lf = form.lower()
            if not ok_word(lf):
                continue
            if kind == "entry" and lemma == form:
                (bad_lemma if status == "bad" else ok_lemma).add(lf)
            if status == "bad":
                kk_bad.add(lf)
                continue
            kk_rows.append((lf, lemma, KK_POS.get(p, O), lower_flag(form), kind))
    bad_lemma -= ok_lemma
    rows = []
    for lf, lemma, p, flag, kind in kk_rows:
        if lemma.lower() in bad_lemma:  # grösser: a form of the Swiss "gross"
            kk_bad.add(lf)
            continue
        kk_ok.add(lf)
        if kind == "entry":
            kk_entry.add(lf)
        rows.append((lf, lemma, p, flag))
    kk_rows = rows
    # A superseded spelling with no entry of its own that is fine (vns: an
    # old "uns", in that word's table too; not Busse, also the plural of Bus).
    old = kk_bad - kk_entry
    for r in kk_rows:
        if r[0] not in old:
            add(*r)
    kk_ok -= old
    del kk_rows

    with open(args.lt, encoding="utf-8") as f:
        for line in f:
            form, lemma, tag = line.rstrip("\n").split("\t")
            lf = form.lower()
            if not ok_word(lf) or lf in old:
                continue
            t = tag.split(":")
            p = LT_POS.get(t[0], O)
            # Comparatives of participles (niesendsten) only when attested.
            if t[0] in ("PA1", "PA2") and ("KOM" in t or "SUP" in t) and lf not in raw and lf not in kk_ok:
                continue
            add(lf, lemma, p, lower_flag(form))
    # Pre-1996 ß (läßt) where the source knows the ss spelling (lässt).
    dropped = [w for w in lemmas if "ß" in w and w not in kk_ok and w.replace("ß", "ss") in lemmas]
    # Swiss ss (grössere) where the same lemma has the ß spelling (größere);
    # not Massen (Masse) next to Maßen (Maß).
    def lemma_words(w):
        return {lem for lem, _, _ in lemmas[w]}

    dropped += [w for w in lemmas if "ss" in w and w not in kk_entry and any(
        v in lemmas and lemma_words(v) & lemma_words(w)
        for v in variants(w) if "ß" in v and v.count("ss") < w.count("ss"))]
    for w in dropped:
        del lemmas[w], flags[w], pos[w]
    union = set(lemmas)
    print(f"forms: {len(union)} source forms ({len(old)} superseded spellings, "
          f"{len(dropped)} old ß / Swiss ss forms dropped)", file=sys.stderr)

    # ---- compounds attested in the subtitles, split into known words
    # A part must be a word seen in the subtitles; a short one (ems, bas,
    # mad), common: short parts are how names (Drummond, Maddocks) split.
    def freq_ok(w, f):
        return f >= (args.short_part if len(w) <= 3 else args.part)

    noun_forms = {w for w in union if pos[w] & N}
    head_ok = {w for w in union if pos[w] & (N | A | V) and len(w) >= 3 and freq_ok(w, raw.get(w, 0))}
    base = {}  # modifier → frequency of the word it comes from
    for w in PARTICLES | NUMBERS:
        base[w] = 10**9

    def add_base(m, f):
        if len(m) >= 2 and f > base.get(m, 0):
            base[m] = f

    for w in union:
        if len(w) < 3 or not pos[w] & (N | A | V):
            continue
        if pos[w] & N:
            add_base(w, raw.get(w, 0))  # Kinder-, Hunde-, Namens-
        for lem, p, _ in lemmas[w]:
            if len(lem) < 3 or not WORD.fullmatch(lem):
                continue
            fl = raw.get(lem, 0)
            if p & N:
                add_base(lem + "s", fl)  # Arbeits-
                if lem.endswith("e") and len(lem) > 3:
                    add_base(lem[:-1], fl)  # Schul-
            if p & A and w == lem:
                add_base(lem, fl)
            if p & V and lem.endswith("en") and len(lem) > 4:
                add_base(lem[:-2], fl)  # Schlaf-, Wasch-
            elif p & V and lem.endswith("n") and len(lem) > 3:
                add_base(lem[:-1], fl)
    base = {m for m, f in base.items() if freq_ok(m, f)}
    memo = {}

    def is_mod(s, depth=0):
        if s in base:
            return True
        if depth >= 2 or len(s) < 6:
            return False
        key = (s, depth)
        if key not in memo:
            memo[key] = any(s[:i] in base and is_mod(s[i:], depth + 1) for i in range(3, len(s) - 2))
        return memo[key]

    def split(w):
        """(modifier, head) with the longest known head, or None."""
        for i in range(2, len(w) - 2):
            head, mod = w[i:], w[:i]
            if head in head_ok and is_mod(mod):
                p = pos[head]
                if p & O and not p & N:
                    continue  # engein, darin: a function word is no head
                if mod in INSEPARABLE and (len(head) < 5 or not p & (A | V)):
                    continue  # begin, bebeutel, erkann
                if p & (N | A) or (p & V and (mod in PARTICLES or mod not in noun_forms)):
                    return mod, head
        return None

    english = set()
    for path in args.english:
        with open(path, "rb") as f:
            for line in f.read().decode("latin-1").splitlines():
                w = line.strip()
                if w and w == w.lower():
                    english.add(w)
    names = set()
    for path in args.names:
        with open(path, "rb") as f:
            names.update(w.strip().lower() for w in f.read().decode("latin-1").splitlines()
                         if w.strip()[:1].isupper())

    def usable_extra(w):
        """Seen in the web/Tatoeba text too — often, and capitalized if short
        (Liv, Jax: names; not mer, ser, que)."""
        lo, cap, caps = casing.get(w, (0, 0, 0))
        n = lo + cap + caps
        return n >= 20 and (len(w) > 3 or cap >= 0.9 * n)

    compounds = {}
    extra = {}
    for w, c in raw.items():
        if w in union or not ok_word(w) or c < args.min_compound:
            continue
        if any(v in union for v in variants(w)):
            continue  # strasse, fuer, fur, paßt: a substitute or a missing diacritic
        if w.startswith("l") and "i" + w[1:] in union:
            continue  # OCR: lch, lhr, lhnen
        if "-" in w:
            parts = w.split("-")
            if c >= 10 and parts[-1] in head_ok and all(
                    x in union or len(x) <= 3 or raw.get(x, 0) >= 100 for x in parts[:-1]):
                compounds[w] = (None, parts[-1])
            continue
        s = split(w) if w not in english else None
        if s:
            compounds[w] = s
        elif (c >= args.keep_frequent and len(w) > 2 and w not in english and w not in JUNK
              and not re.search(r"(.)\1\1|f{4}|^ch[0-9a-f]", w)
              and (w in names or usable_extra(w))):
            extra[w] = c  # names, loanwords (seen outside the subtitles too)
            if w in names:
                flags[w].add("C")
    # Inflect each attested compound like its last word.
    new_forms = defaultdict(set)
    for w, (mod, head) in compounds.items():
        hp = pos[head]
        new_forms[w].add((w, hp & (N | A | V) or N, bool(hp & N)))
        if mod is None:
            continue
        for lem, p, cap in lemmas[head]:
            if p & N and raw[w] >= args.inflect_noun_min and head in by_lemma.get((lem, N), ()):
                # Tabakplantage → Tabakplantagen: every form of the last noun.
                for f_ in by_lemma[(lem, N)]:
                    new_forms[mod + f_].add((mod + lem, N, True))
            if raw[w] < args.inflect_min:
                continue
            if p & A and head in by_lemma.get((lem, A), ()):
                # eiskalt → eiskalte, eiskalten: plain declension only.
                for f_ in by_lemma[(lem, A)]:
                    if f_[len(lem):] in ("", "e", "en", "er", "es", "em") and f_.startswith(lem):
                        new_forms[mod + f_].add((mod + lem, A, False))
            if p & V and mod in PARTICLES and head in by_lemma.get((lem, V), ()):
                # reinkommen → reinkommt, reingekommen (subordinate-clause forms).
                for f_ in by_lemma[(lem, V)]:
                    new_forms[mod + f_].add((mod + lem, V, False))
    added = 0
    for w, rows in new_forms.items():
        if w in union or not ok_word(w):
            continue
        for lem, p, cap in rows:
            lemmas[w].add((lem, p, cap))
            flags[w].add("C" if cap else "L")
            pos[w] |= p
        added += 1
    print(f"compounds: {len(compounds)} attested, {added} forms added", file=sys.stderr)
    if args.compounds:
        with open(args.compounds, "w", encoding="utf-8") as f:
            for w, (mod, head) in sorted(compounds.items()):
                f.write(f"{w}\t{raw[w]}\t{mod or ''}\t{head}\n")

    # ---- an ss form that is a word of its own but in the subtitles mostly
    # stands for a ß word (strasse — the dative of Strass, rhinestone — for
    # Straße) gets the share the web and Tatoeba text gives it (they spell ß).
    capped = 0
    for w in list(lemmas):
        if "ss" not in w:
            continue
        nw = sum(casing.get(w, ()))
        nv = max((sum(casing.get(v, ())) for v in variants(w) if "ß" in v and v in lemmas), default=0)
        if nv and nw < 0.2 * nv and raw.get(w, 0) > 1:
            del lemmas[w]
            c = round(raw[w] * nw / (nw + nv))
            if c > 2:
                extra[w] = c
            capped += 1
    print(f"ss forms capped: {capped}", file=sys.stderr)

    # ---- abbreviations written only in capitals (FBI, OP): their subtitle
    # count is shared with a lowercase word (IT/it, WE/we, NO/no), so it is
    # scaled by how often the corpus writes them in capitals; ones the corpus
    # rarely writes so (me, el) are no German words at all.
    abbr = kept = 0
    for w in list(lemmas):
        fl = flags[w]
        if "A" not in fl or fl & {"L", "C"}:
            continue
        abbr += 1
        del lemmas[w]
        lo, cap, caps = casing.get(w, (0, 0, 0))
        n = lo + cap + caps
        if n >= 5 and caps >= 0.5 * n:
            extra[w] = max(1, round(raw.get(w, caps) * caps / n))
        elif n < 5 and len(w) >= 4:
            extra[w] = max(1, raw.get(w, 1))
        else:
            continue
        kept += 1
    print(f"abbreviations: {kept} of {abbr} kept", file=sys.stderr)

    # ---- X's contractions (geht's, gibt's) attested in the sentences
    if args.corpus:
        apos = Counter()
        plain = Counter()
        for path in args.corpus:
            for text in sentences(path):
                for tok in re.findall(rf"[{L}]+(?:['’]s)?\b", text.lower()):
                    if tok.endswith(("'s", "’s")):
                        apos[tok[:-2]] += 1
                    else:
                        plain[tok] += 1
        n = 0
        for w, c in apos.items():
            if c >= 3 and w in lemmas and pos[w] & (V | O) and not pos[w] & N and w in raw:
                extra[w + "'s"] = max(1, round(raw[w] * min(0.05, c / max(plain[w], c))))
                n += 1
        print(f"contractions: {n} X's forms", file=sys.stderr)

    # make_lexicon.py gives an unseen form a share of its lemma's most frequent
    # form. A form spelled like a word of another lemma must not set that
    # maximum (hat — of haben — for the verb haten; einen, the article, for
    # the verb einen; essen for the name Essen): a form is listed under a
    # lemma only if all its lemmas are one word — same part of speech (or an
    # adjective and its noun, schön/Schöne; not wollen the verb and wollen
    # "woolen") and one lemma a prefix of the
    # other. Otherwise it stands alone.
    def related(g, h):
        if g == h:
            return True
        (a, pa, _), (b, pb, _) = g, h
        if (pa | pb) & (O | P | V) and pa != pb:
            return False  # a verb, a function word or a name only with its own kind
        if pa & (O | P):
            return False
        if len(a) > len(b):
            a, b = b, a
        return b.startswith(a) and len(b) - len(a) <= 2

    rows = {}
    for w in lemmas:
        gs = lemmas[w]
        keep = [f"{lem}|{p}{'C' if cap else ''}" for lem, p, cap in sorted(gs)
                if all(related((lem, p, cap), h) for h in gs)]
        rows[w] = keep or [f"{w}|alone"]
    # A lemma none of whose forms occurs in the subtitles, the web corpus or
    # Tatoeba (Zirconiumhalogeniden, aufgekrempeltere, Männinnen) is too rare
    # to be worth its place in the dictionary (about a third of the forms).
    seen = {}
    for w, keys in rows.items():
        hit = w in raw or w in casing
        for k in keys:
            seen[k] = seen.get(k, False) or hit
    rare = [w for w, keys in rows.items() if not any(seen[k] for k in keys)]
    if args.drop_unattested:
        for w in rare:
            del rows[w]
    with open(args.forms, "w", encoding="utf-8") as f:
        for w in sorted(rows):
            for k in rows[w]:
                f.write(f"{w}\t{k}\n")
    print(f"forms: {sum(k == [w + '|alone'] for w, k in rows.items())} forms of unrelated lemmas "
          f"stand alone; {len(rare)} forms of unattested lemmas"
          f"{' dropped' if args.drop_unattested else ''}", file=sys.stderr)
    with open(args.readings, "w", encoding="utf-8") as f:
        for w in sorted(flags):
            if flags[w]:
                f.write(f"{w}\t{''.join(sorted(flags[w]))}\n")
    with open(args.extra, "w", encoding="utf-8") as f:
        for w in sorted(extra):
            f.write(f"{w}\t{extra[w]}\n")
    print(f"forms: {len(rows)} forms, {len(extra)} extra words", file=sys.stderr)


# ---------------------------------------------------------------- casing
def cmd_casing(args):
    counts = defaultdict(lambda: [0, 0, 0])
    word = re.compile(f"[{L}ẞ'-]+", re.I)
    for path in args.sources:
        for text in sentences(path):
            prev = None
            for tok in TOKEN.findall(text):
                if word.fullmatch(tok) and tok[0].isalpha():
                    if prev is not None and (prev == "," or prev == ";" or prev[0].isalpha()):
                        f = lower_flag(tok)
                        if f != "M":
                            counts[tok.lower()]["LCA".index(f)] += 1
                prev = tok
    for w in sorted(counts):
        c = counts[w]
        sys.stdout.write(f"{w}\t{c[0]}\t{c[1]}\t{c[2]}\n")


# ---------------------------------------------------------------- proper
def cmd_proper(args):
    with open(args.lexicon, encoding="utf-8") as f:
        lexicon = {line.split("\t", 1)[0] for line in f}
    readings = {}
    with open(args.readings, encoding="utf-8") as f:
        for line in f:
            w, fl = line.rstrip("\n").split("\t")
            readings[w] = fl
    casing = read_casing(args.casing)
    out = sys.stdout
    stats = Counter()
    for w in sorted(lexicon):
        if len(w) < 2 or w in CHAT:
            continue
        fl = readings.get(w, "")
        lo, cap, caps = casing.get(w, (0, 0, 0))
        n = lo + cap + caps
        mark = None
        if fl:
            src_l, src_c, src_a = "L" in fl, "C" in fl, "A" in fl
            if src_a and not src_l and not src_c:
                mark = 1 if n >= 5 and cap > caps else 2
            elif src_c and not src_l and not src_a:
                mark = 1 if not (n >= 20 and lo >= 0.5 * n) else None
            elif src_a and src_c and not src_l:  # Bund/BUND, NASA/Nasa
                if n >= 10 and cap + caps >= 0.9 * n:
                    mark = 1 if cap >= caps else 2
            elif n >= 10:  # the sources disagree: the corpus decides
                if cap >= 0.9 * n and src_c:
                    mark = 1
                elif caps >= 0.9 * n and src_a:
                    mark = 2
        elif n >= 5:  # a corpus word the sources lack
            if cap >= 0.9 * n:
                mark = 1
            elif caps >= 0.9 * n and len(w) <= 6:
                mark = 2
        if mark:
            out.write(f"{w}\t{mark}\n")
            stats[mark] += 1
    print(f"proper: {stats[1]} Capitalized, {stats[2]} ALL CAPS", file=sys.stderr)


# ---------------------------------------------------------------- coverage
def cmd_coverage(args):
    with open(args.lexicon, encoding="utf-8") as f:
        lexicon = {line.split("\t", 1)[0] for line in f}
    tot = hit = utot = uhit = old = 0
    missing = Counter()
    with bz2.open(args.tatoeba, "rt", encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) != 3 or not p[0].isdigit() or int(p[0]) % HOLDOUT:
                continue
            for tok in TOKEN.findall(nfc(p[2]).lower().replace("’", "'")):
                tok = tok.strip("'-")
                if not tok or not WORD.fullmatch(tok):
                    continue
                ok = tok in lexicon
                tot += 1
                hit += ok
                if re.search("[äöüß]", tok):
                    utot += 1
                    uhit += ok
                if not ok:
                    missing[tok] += 1
                    old += "ß" in tok and tok.replace("ß", "ss") in lexicon
    print(f"held-out Tatoeba tokens: {hit}/{tot} = {100 * hit / tot:.2f} % in the lexicon; "
          f"with ä/ö/ü/ß: {uhit}/{utot} = {100 * uhit / max(utot, 1):.2f} %")
    print(f"  without {old} tokens in pre-1996 spelling (daß, muß — left out on purpose): "
          f"{100 * hit / (tot - old):.2f} %; with ä/ö/ü/ß: {100 * uhit / max(utot - old, 1):.2f} %")
    print("most frequent missing:", " ".join(f"{w}:{c}" for w, c in missing.most_common(args.show)))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("lt")
    p.add_argument("jar")
    p = sub.add_parser("kaikki")
    p.add_argument("gz")
    p = sub.add_parser("forms")
    p.add_argument("--lt", required=True)
    p.add_argument("--kaikki", required=True)
    p.add_argument("--freq", required=True)
    p.add_argument("--english", action="append", default=[])
    p.add_argument("--corpus", action="append", default=[])
    p.add_argument("--names", action="append", default=[], help="lists of capitalized names")
    p.add_argument("--casing", help="casing.tsv: corpus words seen outside the subtitles")
    p.add_argument("--min-compound", type=int, default=3)
    p.add_argument("--part", type=int, default=5, help="a compound part's least frequency")
    p.add_argument("--short-part", type=int, default=1000, help="the same for parts of ≤ 3 letters")
    p.add_argument("--inflect-noun-min", type=int, default=5,
                   help="noun compounds seen this often get their other forms")
    p.add_argument("--inflect-min", type=int, default=10,
                   help="adjective/verb compounds seen this often get their other forms")
    p.add_argument("--keep-frequent", type=int, default=300)
    p.add_argument("--drop-unattested", action="store_true",
                   help="leave out lemmas no form of which occurs in any corpus")
    p.add_argument("--forms", required=True)
    p.add_argument("--readings", required=True)
    p.add_argument("--extra", required=True)
    p.add_argument("--compounds", help="also list the attested compounds: word, count, split")
    p = sub.add_parser("casing")
    p.add_argument("sources", nargs="+")
    p = sub.add_parser("proper")
    p.add_argument("--lexicon", required=True)
    p.add_argument("--readings", required=True)
    p.add_argument("--casing", required=True)
    p = sub.add_parser("coverage")
    p.add_argument("--lexicon", required=True)
    p.add_argument("--show", type=int, default=60)
    p.add_argument("tatoeba")
    args = ap.parse_args()
    globals()["cmd_" + args.cmd](args)


if __name__ == "__main__":
    main()
