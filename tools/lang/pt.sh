#!/usr/bin/env bash
# Brazilian Portuguese (pt-BR) language pack: fetches the sources into
# data/pt/src/ (skipping what is there) and builds data/pt/ (see data/pt/README.md).
# Idempotent; `FORCE=1` rebuilds every step.
#
# Sources (only official downloads):
#   pt_br_full.txt         hermitdave/FrequencyWords 2018 pt_br (OpenSubtitles)   CC BY-SA 4.0
#   pt_BR.dic, pt_BR.aff   VERO 3.2, LibreOffice pt_BR Hunspell dictionary        LGPLv3 / MPL (dual),
#                          (Raimundo Moura and team), from LibreOffice/dictionaries   see data/pt/VERO-COPYRIGHT
#   por_sentences.tsv.bz2  Tatoeba Portuguese sentences (tatoeba.org contributors) CC BY 2.0 FR
#   por-pt_web_2015_1M     Leipzig Corpora Collection web corpus (Portugal;        CC BY
#                          there is no Brazilian web corpus, only news) — © Universität Leipzig /
#                          Sächsische Akademie der Wissenschaften / InfAI
#   en_full.txt            hermitdave/FrequencyWords 2018 en, only to spot English left
#                          untranslated in the subtitles (nothing of it ships)       CC BY-SA 4.0
#   data/pt/slang.tsv      hand-curated (this repo)
# Outputs (data/pt/): lexicon.tsv, proper.tsv, bigrams.tsv, endings.tsv,
# confusions.tsv, observations.tsv.bz2, rules.tsv, and with target/release/index-builder
# dict.fst, casing.fst, bigrams.fst.
set -euo pipefail
cd "$(dirname "$0")/../.."
D=data/pt
SRC=$D/src
PY=${PYTHON:-python3}
mkdir -p "$SRC"

fetch() {  # fetch FILE URL
    [ -s "$SRC/$1" ] && return
    echo "fetch $1"
    curl -fsSL -o "$SRC/$1.part" "$2"
    mv "$SRC/$1.part" "$SRC/$1"
}
# stale OUT IN... — true when OUT must be (re)built.
stale() {
    local out=$1; shift
    [ "${FORCE:-0}" = 1 ] || [ ! -s "$out" ] && return 0
    for f in "$@"; do [ "$f" -nt "$out" ] && return 0; done
    return 1
}

# update FILE — move FILE.new over FILE unless identical (keeps the later steps fresh).
update() {
    if cmp -s "$1.new" "$1"; then rm "$1.new"; else mv "$1.new" "$1"; fi
}

# Brazilian abbreviations and acronym names the (European) corpora can't vouch for.
export BR_ABBR="cpf cnpj cnh cep inss fgts ibge iptu ipva icms stf stj tse oab enem sbt cbf mec usp ufrj puc clt ong"
export BR_NAMES="detran procon anvisa anatel embrapa unicamp petrobras embraer"

HD=https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018
LO=https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR
fetch pt_br_full.txt "$HD/pt_br/pt_br_full.txt"
if [ -s data/freq/en_full.txt ]; then EN=data/freq/en_full.txt; else fetch en_full.txt "$HD/en/en_full.txt"; EN=$SRC/en_full.txt; fi
fetch pt_BR.dic "$LO/pt_BR.dic"
fetch pt_BR.aff "$LO/pt_BR.aff"
fetch README_pt_BR.txt "$LO/README_pt_BR.txt"
fetch por_sentences.tsv.bz2 https://downloads.tatoeba.org/exports/per_language/por/por_sentences.tsv.bz2
LEIPZIG=por-pt_web_2015_1M
fetch $LEIPZIG.tar.gz https://downloads.wortschatz-leipzig.de/corpora/$LEIPZIG.tar.gz
TATOEBA=$SRC/por_sentences.tsv.bz2
WEB=$SRC/$LEIPZIG.tar.gz

# --- 1. Tokens of the sentence corpora --------------------------------------
# word<TAB>tatoeba<TAB>leipzig<TAB>lower<TAB>Capital<TAB>UPPER (the last three
# in mid-sentence only). Held-out Tatoeba sentences (id % 50 == 0) are skipped.
if stale $SRC/attested.tsv $TATOEBA $WEB; then
$PY - $TATOEBA $WEB $SRC/attested.tsv <<'PY'
import bz2, re, sys, tarfile, unicodedata
from collections import defaultdict

tatoeba, leipzig, out = sys.argv[1:4]
TOK = re.compile(r"[^\W\d_]+(?:['’-][^\W\d_]+)*|[^\w\s]")
tat, lei = defaultdict(int), defaultdict(int)
case = defaultdict(lambda: [0, 0, 0])


def feed(text, counts):
    initial = True
    for tok in TOK.findall(unicodedata.normalize("NFC", text)):
        if not tok[0].isalpha():
            if tok in ".!?:;…—–\"«»“”(":
                initial = True
            continue
        tok = tok.replace("’", "'")
        w = tok.lower()
        counts[w] += 1
        if not initial:
            c = case[w]
            if tok == w:
                c[0] += 1
            elif len(tok) > 1 and tok == tok.upper():
                c[2] += 1
            elif tok[0].isupper() and tok[1:] == tok[1:].lower():
                c[1] += 1
        initial = False


with bz2.open(tatoeba, "rt", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) == 3 and p[0].isdigit() and int(p[0]) % 50:
            feed(p[2], tat)
with tarfile.open(leipzig) as tar:
    m = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
    for raw in tar.extractfile(m):
        feed(raw.decode("utf-8", "replace").rstrip("\n").split("\t", 1)[-1], lei)
with open(out, "w", encoding="utf-8") as f:
    for w in sorted(set(tat) | set(lei)):
        c = case.get(w, (0, 0, 0))
        f.write(f"{w}\t{tat.get(w, 0)}\t{lei.get(w, 0)}\t{c[0]}\t{c[1]}\t{c[2]}\n")
print(f"attested: {len(tat)} Tatoeba, {len(lei)} Leipzig words", file=sys.stderr)
PY
fi

# --- 2. VERO forms ------------------------------------------------------------
# The dictionary expanded here (unmunch misreads its UTF-8 flags): every form
# seen in the subtitles or the sentence corpora, plus the unseen forms of
# lemmas seen ≥ 30 times (derived words — diminutives, -íssimo, -mente,
# prefixes — count as lemmas of their own: ≥ 100, next to a base seen as
# often). Enclitic/mesoclitic forms (dá-me, fazê-lo, dir-se-ia) only when
# attested; the hosts Hunspell needs for them (falá, fazê, falamo) only when
# attested alone; unseen vós forms (falásseis) left out.
# Out: form<TAB>lemma<TAB>kind (i/d/c/h)<TAB>capitalized entry (0/1); and the
# forms of the slang verbs (slang.tsv lines with a model verb).
if stale $SRC/vero_forms.tsv $SRC/pt_BR.dic $SRC/pt_BR.aff $SRC/pt_br_full.txt $SRC/attested.tsv $D/slang.tsv; then
$PY - $SRC/pt_BR.aff $SRC/pt_BR.dic $SRC/pt_br_full.txt $SRC/attested.tsv $SRC/vero_forms.tsv \
    --slang $D/slang.tsv --slang-out $SRC/slang_forms.tsv <<'PY'
import argparse, re, sys, unicodedata
from collections import defaultdict

ap = argparse.ArgumentParser()
for a in ("aff", "dic", "freq", "attested", "out"):
    ap.add_argument(a)
ap.add_argument("--min-top", type=int, default=30)
ap.add_argument("--min-sub", type=int, default=100)
ap.add_argument("--slang")
ap.add_argument("--slang-out")
args = ap.parse_args()

FORBIDDEN, WARN = "ý", "~"          # VERO: forbidden, rare/incorrect (pára)
VERB = set("acdefghituw")           # conjugations
INFL = set("ABCDF") | VERB          # + plural, gender
MIXED = set("EGLb")                 # inflection + degree forms
CLITIC = set("kmnopqrsv")           # ênclises e mesóclises
DEGREE = re.compile(r"(inh[ao]s?|zinh[ao]s?|zão|zões|zon[ao]s?|õezões|ezões|íssim[ao]s?|"
                    r"érrim[ao]s?|mente|d(?:ão|ões|ona|onas))$")
VOS = re.compile(r"(ais|eis|is|áveis|íeis|astes|estes|istes|[áéêí]reis|[aei]reis|[aei]ríeis|"
                 r"[áêí]sseis|[aei]rdes)$")
STEM = re.compile(r"([áêô]|(?<!ssi)mo)$")   # clitic hosts, for the slang verbs


def nfc(s):
    return unicodedata.normalize("NFC", s)


rules = {"SFX": defaultdict(lambda: defaultdict(list)), "PFX": defaultdict(lambda: defaultdict(list))}
cross = {}
with open(args.aff, encoding="utf-8-sig") as f:
    for line in f:
        p = nfc(line).split()
        if len(p) < 4 or p[0] not in rules:
            continue
        if len(p) == 4 and p[2] in "YN":
            cross[(p[0], p[1])] = p[2] == "Y"
            continue
        kind, flag, strip, add, cond = p[:5]
        strip = "" if strip == "0" else strip
        add = add.split("/")[0]
        add = "" if add == "0" else add
        if kind == "PFX":
            sub = flag
        elif flag in CLITIC:
            sub = "c"
        elif flag in INFL or (flag in MIXED and not DEGREE.search(add)):
            sub = "v" if flag in VERB else None
        else:
            sub = flag
        rules[kind][flag][cond].append((strip, add, sub))
compiled = {}
for kind, by_flag in rules.items():
    for flag, by_cond in by_flag.items():
        compiled[(kind, flag)] = [
            (None if cond == "." else re.compile(("(?:%s)$" if kind == "SFX" else "^(?:%s)") % cond), rs)
            for cond, rs in by_cond.items()]


def suffixes(word, flag):
    for rx, rs in compiled[("SFX", flag)]:
        if rx is None or rx.search(word):
            for strip, add, sub in rs:
                if word.endswith(strip):
                    yield (word[: len(word) - len(strip)] if strip else word) + add, sub


def prefixes(word, flag):
    for rx, rs in compiled[("PFX", flag)]:
        if rx is None or rx.search(word):
            for strip, add, _ in rs:
                if word.startswith(strip):
                    yield strip, add


freq = {}
with open(args.freq, encoding="utf-8") as f:
    for line in f:
        p = line.split()
        if len(p) == 2:
            freq[nfc(p[0].lower())] = int(p[1])
att = {}
with open(args.attested, encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        att[p[0]] = int(p[1]) + int(p[2])


def seen(w):
    return freq.get(w, 0) or att.get(w, 0)


slang = []
if args.slang:
    with open(args.slang, encoding="utf-8") as f:
        for line in f:
            p = nfc(line).rstrip("\n").split("\t")
            if not line.startswith("#") and len(p) >= 3 and p[2]:
                slang.append((p[0], int(p[1]), p[2]))
models = {m for _, _, m in slang}
model_flags = {}
stats = defaultdict(int)
out = open(args.out, "w", encoding="utf-8")
with open(args.dic, encoding="utf-8-sig") as f:
    next(f)
    for line in f:
        line = nfc(line.rstrip("\n"))
        if not line:
            continue
        word, _, flags = line.partition("/")
        if FORBIDDEN in flags or WARN in flags:
            continue
        if word in models:
            model_flags[word] = flags
        cap = "1" if word[:1].isupper() else "0"
        lword = word.lower()
        forms = {lword: (word, "h" if "-" in lword else "i")}   # form → (lemma, kind)
        verb_forms, clitics, suffixed = set(), set(), []
        for fl in flags:
            if ("SFX", fl) not in compiled:
                continue
            cr = cross.get(("SFX", fl), False)
            for form, sub in suffixes(word, fl):
                lf = form.lower()
                if sub == "c":
                    clitics.add(lf)
                else:
                    if sub == "v":
                        verb_forms.add(lf)
                    if sub in (None, "v"):
                        forms.setdefault(lf, (word, "h" if "-" in lf else "i"))
                    else:
                        forms.setdefault(lf, (f"{word}#{sub}", "d"))
                if cr:
                    suffixed.append((lf, sub))
        for fl in flags:
            if ("PFX", fl) not in compiled:
                continue
            pre = list(prefixes(lword, fl))
            for strip, add in pre:
                forms.setdefault(add + lword[len(strip):], (f"{word}#{fl}", "d"))
            if cross.get(("PFX", fl), False):
                for strip, add in pre:
                    for lf, sub in suffixed:
                        if lf.startswith(strip):
                            pf = add + lf[len(strip):]
                            if sub == "c":
                                clitics.add(pf)
                                continue
                            if sub == "v":
                                verb_forms.add(pf)
                            d = sub if sub not in (None, "v") else ""
                            forms.setdefault(pf, (f"{word}#{fl}{d}", "d"))
        stats["clitic forms"] += len(clitics)
        for c in clitics:
            if att.get(c) and c not in forms:
                forms[c] = (word, "c")
        # Hunspell lists the hosts of clitics as words: falá (falá-lo), fazê,
        # falamo (falamo-nos), falaremo. Only attested ones stay (dá, está, vê).
        for h in [w for w in verb_forms if w in forms and w != lword and att.get(w, 0) < 20]:
            if (h.endswith("mo") and h + "s" in verb_forms) or \
                    (h[-1] in "áêô" and h[:-1] + {"á": "ar", "ê": "er", "ô": "or"}[h[-1]] in forms):
                del forms[h]
                stats["clitic hosts dropped"] += 1
        tops = defaultdict(int)
        for w, (lemma, kind) in forms.items():
            if kind != "c":
                tops[lemma] = max(tops[lemma], seen(w))
        base_top = tops.get(word, 0)
        for w, (lemma, kind) in forms.items():
            stats["forms"] += kind != "c"
            if kind == "c" or seen(w):
                ok = True
            elif w in verb_forms and VOS.search(w):
                ok = False
            elif lemma == word:
                ok = tops[lemma] >= args.min_top
            else:
                ok = tops[lemma] >= args.min_sub and base_top >= args.min_sub
            if ok:
                out.write(f"{w}\t{lemma}\t{kind}\t{cap}\n")
                stats[f"kept {'inflected derived clitic hyphenated'.split()['idch'.index(kind)]}"] += 1
out.close()

# Slang verbs conjugated like their model: the word keeps its count, the
# other forms get a fifth.
if args.slang_out:
    sf = {}
    for word, count, model in slang:
        if model not in model_flags:
            print(f"slang: no model {model} for {word}", file=sys.stderr)
            continue
        sf[word] = max(sf.get(word, 0), count)
        for fl in model_flags[model]:
            if ("SFX", fl) in compiled:
                for form, sub in suffixes(word, fl):
                    if sub in (None, "v") and not (VOS.search(form) or STEM.search(form) or DEGREE.search(form)):
                        sf[form] = max(sf.get(form, 0), max(1, count // 5))
    with open(args.slang_out, "w", encoding="utf-8") as f:
        for w in sorted(sf):
            f.write(f"{w}\t{sf[w]}\n")
    stats["slang forms"] = len(sf)
print("VERO: " + ", ".join(f"{k} {v}" for k, v in sorted(stats.items())), file=sys.stderr)
PY
fi

# --- 3. Lexicon inputs --------------------------------------------------------
# freq.txt: subtitle counts of VERO forms and of frequent corpus words
# outside VERO (names, loanwords) — not misspellings (nao, voce, idéia, vôo),
# European spellings (óptimo, acção; in VERO too: facto, contacto, perspetiva,
# bebé), OCR slips (ihe), subtitle credits or untranslated English (the, you).
# forms.tsv: form<TAB>lemma (a form towering over its lemma — para ← parar,
# como ← comer — is split off so it doesn't lift the unseen forms).
# hyphen.tsv: hyphenated words with counts from the sentence corpora.
SEL=$SRC/sel
if stale $SEL/forms.tsv $SRC/vero_forms.tsv $EN $D/slang.tsv; then
$PY - $SRC/vero_forms.tsv $SRC/pt_br_full.txt $SRC/attested.tsv $EN $D/slang.tsv $SEL <<'PY'
import argparse, os, re, sys, unicodedata
from collections import defaultdict

ap = argparse.ArgumentParser()
for a in ("vero", "freq", "attested", "en_freq", "slang", "outdir"):
    ap.add_argument(a)
ap.add_argument("--keep-frequent", type=int, default=300)
ap.add_argument("--english-ratio", type=float, default=4.0)
ap.add_argument("--split-ratio", type=float, default=20.0)
args = ap.parse_args()

LETTERS = re.compile(r"[a-zß-öø-ÿœ'-]*[a-zß-öø-ÿœ][a-zß-öø-ÿœ'-]*")   # kbcore CHARSET, Latin
PT = re.compile(r"[a-zçáâãàéêíóôõú]+(?:-[a-zçáâãàéêíóôõú]+)*")
JUNK = re.compile(r"subs?\b|subs|sub(?:pack|makers|s)|sync|legender|team|rip$|fbr$|^www")
LEIPZIG_WEIGHT = 0.3   # the Leipzig web is European (pequeno-almoço, guarda-redes)
BR_ABBR = set(os.environ["BR_ABBR"].split())


def nfc(s):
    return unicodedata.normalize("NFC", s)


def plain(w):
    return "".join(c for c in unicodedata.normalize("NFD", w) if not unicodedata.combining(c))


def read_freq(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            if len(p) == 2:
                w = nfc(p[0].lower())
                out[w] = max(out.get(w, 0), int(p[1]))
    return out


freq, en = read_freq(args.freq), read_freq(args.en_freq)
pt_total, en_total = sum(freq.values()), sum(en.values())
tat, lei, upper_share = {}, {}, {}
with open(args.attested, encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        tat[p[0]], lei[p[0]] = int(p[1]), int(p[2])
        n = int(p[3]) + int(p[4]) + int(p[5])
        if n >= 3:
            upper_share[p[0]] = int(p[5]) / n
scale = pt_total / (sum(tat.values()) + sum(lei.values()))


def att(w):
    return tat.get(w, 0) + lei.get(w, 0)


slang = set()
with open(args.slang, encoding="utf-8") as f:
    slang = {nfc(l.split("\t", 1)[0]) for l in f if not l.startswith("#") and "\t" in l}
vero, kinds, common, abbrs = defaultdict(set), {}, set(), set()
with open(args.vero, encoding="utf-8") as f:
    for line in f:
        w, lemma, kind, cap = line.rstrip("\n").split("\t")
        if not LETTERS.fullmatch(w):
            continue  # nº, cm², µg
        vero[w].add(lemma)
        kinds.setdefault(w, kind)
        if cap == "0":
            common.add(w)
        entry = lemma.split("#")[0]
        if len(entry) >= 2 and entry == entry.upper() and w == entry.lower():
            abbrs.add(w)
dropped = []
# VERO's abbreviations (NNO, AIME, ELK, JT) show up in the subtitles as
# typos and foreign words: kept only when the corpora write them in capitals
# half the time (TV, FBI, CPF, USP), or in BR_ABBR.
for w in sorted(abbrs - common):
    if upper_share.get(w, 0) < 0.5 and w not in BR_ABBR and w not in slang:
        dropped.append((w, "abbreviation", freq.get(w, 0)))
        del vero[w]


def variants(w):
    """Spellings that differ between Brazil and Portugal: a silent c/p (facto /
    fato, perspetiva / perspectiva), é/ê, ó/ô (bebé / bebê), registo / registro,
    connosco / conosco."""
    for m in re.finditer(r"[cp](?=[cçt])", w):
        yield w[: m.start()] + w[m.end():]
    for m in re.finditer(r"[éêóô]", w):
        yield w[: m.start()] + {"é": "ê", "ê": "é", "ó": "ô", "ô": "ó"}[m.group()] + w[m.end():]
    if "regist" in w and "registr" not in w:
        yield w.replace("regist", "registr")
    if "connosc" in w:
        yield w.replace("connosc", "conosc")


def rarer_european(w, v):
    """The spelling to drop: ≥ 5× rarer in the (Brazilian) subtitles, and
    ≥ 20× more at home in the (European) Leipzig web than there."""
    d, r = (w, v) if freq.get(w, 0) >= freq.get(v, 0) else (v, w)
    sd, sr = freq.get(d, 0), freq.get(r, 0)
    if sd < 5 * max(sr, 1) or lei.get(r, 0) < 3:
        return None
    skew = ((lei.get(r, 0) + 1) / (lei.get(d, 0) + 1)) / ((sr + 1) / (sd + 1))
    return r if skew >= 20 else None


ROMAN = re.compile(r"m{0,4}(cm|cd|d?c{0,3})(xc|xl|l?x{0,3})(ix|iv|v?i{0,3})")


def english(w, c):
    """Much more frequent in English subtitles (and clearly an English word
    there: small counts are names left untranslated either way)."""
    e = en.get(w, 0)
    return c and e >= 1000 and (e / en_total) / (c / pt_total) > args.english_ratio


# Names and abbreviations of VERO that are really English words of the
# subtitles (CAT, HIS, PIG), and Roman numerals (MCMLXVIII).
for w in [w for w in vero if w not in common]:
    roman = len(w) >= 3 and ROMAN.fullmatch(w)   # not cm, mm, ml, xl
    if roman or english(w, freq.get(w, 0)):
        dropped.append((w, "roman" if roman else "english", freq.get(w, 0)))
        del vero[w]
for w in sorted(common):
    if len(w) >= 4 and w in vero:
        for v in variants(w):
            if v in common and v in vero:
                r = rarer_european(w, v)
                if r:
                    dropped.append((r, "european", v if r == w else w))
                    del vero[r]
                    break

by_plain = defaultdict(list)   # common words only: Julia and Júlia are both names
for w in vero:
    if w in common:
        by_plain[plain(w)].append(w)


def why_not(w, c):
    """Why a corpus word outside VERO stays out (None: it joins)."""
    if w in slang:
        return None
    if c < args.keep_frequent:
        return "rare"
    if not PT.fullmatch(w) or len(w) <= 2:
        return "letters"
    if JUNK.search(w) or att(w) < 2:
        return "unattested"
    others = [v for v in by_plain.get(plain(w), ()) if v != w]
    if others:
        return "accents:" + max(others, key=lambda v: freq.get(v, 0))
    for v in variants(w):
        if v in vero and freq.get(v, 0) >= 5 * c:
            return "european:" + v
    for a, b in (("i", "l"), ("l", "i"), ("rn", "m"), ("ii", "ll")):
        for m in re.finditer(a, w):
            v = w[: m.start()] + b + w[m.end():]
            if v in vero and freq.get(v, 0) >= 5 * c:
                return "ocr:" + v
    if english(w, c):
        return "english"
    return None


os.makedirs(args.outdir, exist_ok=True)
keep = {}
for w, c in freq.items():
    if w in vero:
        keep[w] = c
    else:
        r = why_not(w, c)
        if r is None:
            keep[w] = c
        elif r != "rare":
            dropped.append((w, r, c))
with open(f"{args.outdir}/freq.txt", "w", encoding="utf-8") as f:
    for w, c in sorted(keep.items(), key=lambda x: -x[1]):
        f.write(f"{w} {c}\n")

groups = defaultdict(list)
for w, lemmas in vero.items():
    if "-" not in w:
        for lemma in lemmas:
            groups[lemma].append(w)
split = 0
top = {}
with open(f"{args.outdir}/forms.tsv", "w", encoding="utf-8") as f:
    for lemma, ws in sorted(groups.items()):
        counts = sorted((freq.get(w, 0) for w in ws), reverse=True) + [0]
        top[lemma] = counts[1] if len(ws) > 1 else counts[0]
        for w in sorted(ws):
            key = lemma
            if len(ws) > 2 and freq.get(w, 0) == counts[0] > args.split_ratio * max(counts[1], 1):
                key = f"{lemma}#{w}"
                split += 1
            f.write(f"{w}\t{key}\n")
n = 0
with open(f"{args.outdir}/hyphen.tsv", "w", encoding="utf-8") as f:
    for w, lemmas in sorted(vero.items()):
        if "-" in w:
            c = max(1, round((tat.get(w, 0) + LEIPZIG_WEIGHT * lei.get(w, 0)) * scale))
            if kinds[w] == "c":   # clitics: at most 2 % of their verb's top form
                c = min(c, max(1, round(0.02 * max(top.get(l, 0) for l in lemmas))))
            f.write(f"{w}\t{c}\n")
            n += 1
with open(f"{args.outdir}/dropped.tsv", "w", encoding="utf-8") as f:
    for row in dropped:
        f.write("\t".join(map(str, row)) + "\n")
reasons = defaultdict(int)
for row in dropped:
    reasons[row[1].split(":")[0]] += 1
print(f"select: {len(keep)} corpus words, {len(groups)} lemmas ({split} forms split off), "
      f"{n} hyphenated; dropped " + ", ".join(f"{k} {v}" for k, v in sorted(reasons.items())),
      file=sys.stderr)
PY
fi

# --- 4. Lexicon -----------------------------------------------------------------
# Seen forms keep their count, unseen ones get 0.5 % of their lemma's top form
# (Portuguese verbs have ~70 forms, most rarely typed); corpus words outside
# VERO join at ≥ 300.
$PY tools/make_lexicon.py $SEL/freq.txt --words $SEL/forms.tsv --forms $SEL/forms.tsv \
    --form-share 0.005 --keep-frequent 300 --extra $SEL/hyphen.tsv --extra $D/slang.tsv \
    --extra $SRC/slang_forms.tsv -o $D/lexicon.tsv.new
update $D/lexicon.tsv

# --- 5. Capitals ------------------------------------------------------------------
$PY - $SRC/vero_forms.tsv $SRC/attested.tsv $D/lexicon.tsv $D/slang.tsv > $D/proper.tsv.new <<'PY'
"""word<TAB>1 (Capitalized) / 2 (ALL CAPS):
- forms of VERO's capitalized entries (Maria, Brasil, Coimbra) that no
  lowercase entry has (rosa, flor, graça, vitória, janeiro stay as typed),
  unless the corpora write them in lowercase half the time in mid-sentence;
  entries in capitals (CPF, ONU, FBI) in capitals when the corpora write
  them so ≥ 80 % of the time and the subtitles don't use them much more than
  the web (not eba, doc, toc, jen); slang.tsv words as typed unless the
  corpora capitalize them (Facebook);
- a name that is also a common word only when the corpora capitalize it
  ≥ 95 % of the time in mid-sentence (Deus); a word VERO has in lowercase
  only when ≥ 98.5 % (VERO lacks many first names, and has joão, paulo, pedro
  as rare common words or verb forms);
- corpus words outside VERO (names from the subtitles: Jack, Michelle; FBI)
  capitalized ≥ 80 % of the time in mid-sentence.
Hyphenated words and words of ≤ 2 letters stay out (Guiné-Bissau would come out
Guiné-bissau; SP, RJ, TV are typed in lowercase in chats). BR_ABBR in
capitals, BR_NAMES capitalized (Detran, Anvisa)."""
import os
import sys

vero_path, att_path, lex_path, slang_path = sys.argv[1:5]
lexicon = {}
for line in open(lex_path, encoding="utf-8"):
    w, c = line.rstrip("\n").split("\t")
    lexicon[w] = int(c)
sub_total = sum(lexicon.values())
slang = {l.split("\t", 1)[0] for l in open(slang_path, encoding="utf-8") if not l.startswith("#")}
BR_ABBR, BR_NAMES = set(os.environ["BR_ABBR"].split()), set(os.environ["BR_NAMES"].split())
case, web = {}, {}
with open(att_path, encoding="utf-8") as f:
    for line in f:
        w, t, l, lo, ca, up = line.rstrip("\n").split("\t")
        case[w] = (int(lo), int(ca), int(up))
        web[w] = int(t) + int(l)
web_total = sum(web.values())


def in_capitals(w, n, up):
    """The corpora write it in capitals, and the subtitles don't use it much
    more than the web does — as something else: toc, jen, duh, umm."""
    return (n >= 5 and up >= 0.8 * n
            and lexicon[w] / sub_total <= 2 * web.get(w, 0) / web_total)

common, proper, upper = set(), set(), set()
with open(vero_path, encoding="utf-8") as f:
    for line in f:
        w, lemma, _kind, cap = line.rstrip("\n").split("\t")
        if cap == "0":
            common.add(w)
        else:
            proper.add(w)
            entry = lemma.split("#")[0]
            if len(entry) >= 2 and entry == entry.upper() and w == entry.lower():
                upper.add(w)
out = {}
for w in sorted(lexicon):
    if "-" in w or "'" in w or len(w) <= 2:
        continue
    lo, ca, up = case.get(w, (0, 0, 0))
    n = lo + ca + up
    if w in BR_ABBR or w in BR_NAMES:
        out[w] = 2 if w in BR_ABBR else 1
    elif w in slang:   # as typed (eba, pet, pix), or Capitalized if the corpora say so (Facebook)
        if n >= 3 and ca >= 0.8 * n:
            out[w] = 1
    elif w in upper and w not in common:
        if in_capitals(w, n, up):   # abbreviations only when the corpora agree (not eba, doc, cof)
            out[w] = 2
    elif w in proper and w not in common:
        if not (n >= 5 and lo >= 0.5 * n):
            out[w] = 2 if in_capitals(w, n, up) else 1
    elif w in proper:
        if n >= 20 and ca + up >= 0.95 * n:
            out[w] = 2 if in_capitals(w, n, up) else 1
    elif w in common:
        if n >= 20 and ca + up >= 0.985 * n:
            out[w] = 2 if in_capitals(w, n, up) else 1
    elif n >= 3 and ca + up >= 0.8 * n:
        out[w] = 2 if in_capitals(w, n, up) else 1
for w, v in out.items():
    print(f"{w}\t{v}")
caps = sum(v == 2 for v in out.values())
print(f"proper: {len(out) - caps} capitalized, {caps} in capitals", file=sys.stderr)
PY
update $D/proper.tsv

# --- 6. Context model -------------------------------------------------------------
# Everyday Tatoeba sentences (0.6) + the Leipzig web (0.4); endings of Latin
# words too (-os after -os: agreement).
if stale $D/bigrams.tsv $D/lexicon.tsv; then
    $PY tools/build_bigrams.py $TATOEBA:0.6 $WEB:0.4 --lexicon $D/lexicon.tsv \
        --endings $D/endings.tsv --endings-any-script > $D/bigrams.tsv
fi

# --- 7. Real-word slips: pairs, observations, decision lists -------------------------
if stale $D/rules.tsv $D/lexicon.tsv; then
    $PY tools/confusion_sets.py --lexicon $D/lexicon.tsv --rows qwertyuiop asdfghjklç zxcvbnm > $D/confusions.tsv
    # The tool pairs a word with its spelling minus one accent (esta/está,
    # pais/país); add the pairs that differ in more (avó/avô, mantém/mantêm,
    # maca/maçã): both frequent (≥ 100), ≥ 3 letters.
    $PY - $D/lexicon.tsv $D/confusions.tsv <<'PY'
import sys, unicodedata
from collections import defaultdict

lex, conf = sys.argv[1:3]
words = sorted(((int(c), w) for w, c in (l.rstrip("\n").split("\t") for l in open(lex, encoding="utf-8"))),
               reverse=True)[:80000]
have = set()
for line in open(conf, encoding="utf-8"):
    a, b = line.rstrip("\n").split("\t")
    have |= {(a, b), (b, a)}
by_plain = defaultdict(list)
for c, w in words:
    if c >= 100 and len(w) >= 3:
        by_plain["".join(ch for ch in unicodedata.normalize("NFD", w) if not unicodedata.combining(ch))].append(w)
new = [(a, b) for ws in by_plain.values() for i, a in enumerate(ws) for b in ws[i + 1:] if (a, b) not in have]
with open(conf, "a", encoding="utf-8") as f:
    f.writelines(f"{a}\t{b}\n" for a, b in new)
print(f"{len(new)} more accent pairs: " + " ".join(f"{a}/{b}" for a, b in new[:12]), file=sys.stderr)
PY
    $PY tools/export_observations.py $TATOEBA $WEB --confusions $D/confusions.tsv --classes /nonexistent \
        | bzip2 > $D/observations.tsv.bz2
    bzcat $D/observations.tsv.bz2 | $PY tools/decision_lists.py --confusions $D/confusions.tsv > $D/rules.tsv
fi

# --- 8. FSTs (with the prebuilt index-builder) ---------------------------------------
IB=${INDEX_BUILDER:-target/release/index-builder}
if [ -x "$IB" ]; then
    stale $D/dict.fst $D/lexicon.tsv "$IB" && "$IB" $D/lexicon.tsv $D/dict.fst
    stale $D/casing.fst $D/proper.tsv "$IB" && "$IB" --casing $D/proper.tsv $D/casing.fst
    stale $D/bigrams.fst $D/bigrams.tsv $D/endings.tsv $D/rules.tsv "$IB" && \
        "$IB" --bigrams $D/bigrams.tsv $D/bigrams.fst --endings $D/endings.tsv \
            --rules $D/confusions.tsv $D/rules.tsv
fi

# --- 9. Stats ---------------------------------------------------------------------------
$PY - $TATOEBA $D <<'PY'
"""Counts, FST sizes, and lexicon coverage of held-out Tatoeba tokens (ids
divisible by 50, never trained on)."""
import bz2, os, re, sys, unicodedata

tatoeba, d = sys.argv[1:3]


def lines(name):
    try:
        with open(f"{d}/{name}", encoding="utf-8") as f:
            return sum(1 for _ in f)
    except OSError:
        return 0


words = {}
with open(f"{d}/lexicon.tsv", encoding="utf-8") as f:
    for line in f:
        w, c = line.rstrip("\n").split("\t")
        words[w] = int(c)
TOK = re.compile(r"[^\W\d_]+(?:['’-][^\W\d_]+)*")
ACC = re.compile(r"[áâãàéêíóôõúüç]")
n = hit = na = hita = 0
with bz2.open(tatoeba, "rt", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) == 3 and p[0].isdigit() and int(p[0]) % 50 == 0:
            for tok in TOK.findall(unicodedata.normalize("NFC", p[2])):
                w = tok.replace("’", "'").lower()
                n += 1
                hit += w in words
                if ACC.search(w):
                    na += 1
                    hita += w in words
print(f"lexicon: {len(words)} forms ({sum('-' in w for w in words)} hyphenated); "
      f"proper: {lines('proper.tsv')}; bigrams: {lines('bigrams.tsv')}; endings: {lines('endings.tsv')}; "
      f"confusion pairs: {lines('confusions.tsv')}; rules: {lines('rules.tsv')}")
def size(path):
    n = os.path.getsize(path)
    return f"{n / 1e6:.1f} MB" if n >= 1e6 else f"{n / 1e3:.0f} KB"


print("FSTs: " + ", ".join(f"{x} {size(f'{d}/{x}')}"
                          for x in ("dict.fst", "casing.fst", "bigrams.fst") if os.path.exists(f"{d}/{x}")))
print(f"held-out Tatoeba coverage: {100 * hit / n:.2f} % of {n} tokens; "
      f"with accents/ç: {100 * hita / na:.2f} % of {na}")
print("top: " + " ".join(sorted(words, key=words.get, reverse=True)[:30]))
PY
