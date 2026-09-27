#!/usr/bin/env bash
# Spanish (es) language pack data → data/es/ (git-ignored except README.md,
# slang.tsv and RLA-ES-COPYRIGHT). Idempotent: sources already in data/es/src/
# are not fetched again; everything else is rebuilt.
#
# Sources (downloaded into data/es/src/):
#   es_full.txt, en_full.txt   — hermitdave/FrequencyWords 2018 (OpenSubtitles),
#                                CC BY-SA 4.0: the frequency prior; the English
#                                list only tells English words in Spanish
#                                subtitles apart
#   es.oxt → rla-es/es.{dic,aff} — RLA-ES v2.9 generic Spanish Hunspell
#                                dictionary (Santiago Bosio and contributors),
#                                GPLv3+ / LGPLv3+ / MPL 1.1+ at the user's
#                                choice — used under MPL 2.0 / LGPLv3+, see
#                                data/es/RLA-ES-COPYRIGHT: every word form
#   spa_sentences.tsv.bz2      — Tatoeba Spanish sentences (tatoeba.org
#                                contributors), CC BY 2.0 FR: context model,
#                                capitals, held-out coverage (ids % 50 == 0)
#   spa-mx_web_2015_1M.tar.gz  — Leipzig Corpora Collection, Spanish web
#                                (Mexico) 2015, 1M sentences, © Universität
#                                Leipzig / SAW / InfAI, CC BY: context model
#                                (the sentences written with accents),
#                                capitals, attestation of rare forms
#   data/es/slang.tsv          — hand-curated (this repo)
#
# Output: lexicon.tsv (+ dict.fst), proper.tsv (+ casing.fst), bigrams.tsv and
# endings.tsv (+ bigrams.fst with the rules), confusions.tsv,
# observations.tsv.bz2, rules.tsv. The FSTs are built when
# $INDEX_BUILDER (default target/release/index-builder) exists.
# Needs python3 (stdlib only), curl, bzip2. ~10 min, a few GB of RAM (bigrams).
set -euo pipefail
cd "$(dirname "$0")/../.."
D=data/es
S=$D/src
LEIPZIG=spa-mx_web_2015_1M
PY=${PYTHON:-python3}
IB=${INDEX_BUILDER:-target/release/index-builder}
mkdir -p "$S"

fetch() {  # file url
    [ -s "$S/$1" ] && return
    echo "fetching $1"
    curl -fsSL -o "$S/$1.part" "$2" && mv "$S/$1.part" "$S/$1"
}
FW=https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018
fetch es_full.txt $FW/es/es_full.txt
fetch en_full.txt $FW/en/en_full.txt
fetch es.oxt https://github.com/sbosio/rla-es/releases/download/v2.9/es.oxt
fetch spa_sentences.tsv.bz2 https://downloads.tatoeba.org/exports/per_language/spa/spa_sentences.tsv.bz2
fetch $LEIPZIG.tar.gz https://downloads.wortschatz-leipzig.de/corpora/$LEIPZIG.tar.gz
"$PY" -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2], ["es.dic", "es.aff", "LICENSE.md", "README.txt"])' \
    "$S/es.oxt" "$S/rla-es"

# 1. Every form of the Hunspell dictionary: `form<TAB>lemma<TAB>kind<TAB>case`,
# kind base/sfx/pfx/clitic (a verb with enclitic pronouns: dámelo — the
# affix flags À…ú), case l / C (a name) / U (an abbreviation). Hunspell's
# unmunch misreads this dictionary's UTF-8 flags, so the affixes are applied
# here (PFX/SFX with strip, add, condition, cross products and continuation
# classes).
"$PY" - "$S/rla-es/es.aff" "$S/rla-es/es.dic" > "$S/rla_forms.tsv" <<'PY'
import collections, re, sys
aff, dic = sys.argv[1:3]
rules, cross, kind_of = collections.defaultdict(list), {}, {}
with open(aff, encoding="utf-8") as f:
    for line in f:
        p = line.split()
        if len(p) < 4 or p[0] not in ("PFX", "SFX"):
            continue
        flag = p[1][0]
        if len(p) == 4 and p[2] in "YN":
            cross[flag], kind_of[flag] = p[2] == "Y", p[0]
            continue
        strip = "" if p[2] == "0" else p[2]
        add, _, cont = p[3].partition("/")
        cond = p[4] if len(p) > 4 else "."
        rx = re.compile("^" + cond if p[0] == "PFX" else cond + "$")
        rules[flag].append((strip, "" if add == "0" else add, cont, rx))


def is_clitic(flag):
    return kind_of.get(flag) == "SFX" and "À" <= flag <= "ÿ"


seen, n = set(), collections.Counter()


def emit(form, lemma, kind, case):
    if (form, lemma, kind) not in seen:
        seen.add((form, lemma, kind))
        n[kind] += 1
        sys.stdout.write(f"{form}\t{lemma}\t{kind}\t{case}\n")


def suffixes(word, flags, lemma, kind, case, depth=0):
    for fl in flags:
        if kind_of.get(fl) != "SFX":
            continue
        k = "clitic" if is_clitic(fl) else kind
        for strip, add, cont, rx in rules[fl]:
            if word.endswith(strip) and rx.search(word):
                form = word[: len(word) - len(strip)] + add
                emit(form, lemma, k, case)
                if cont and depth < 2:  # twofold suffixes: acción/S
                    suffixes(form, cont, lemma, k, case, depth + 1)


with open(dic, encoding="utf-8") as f:
    next(f)
    for line in f:
        word, _, flags = line.strip().partition("/")
        if not word or " " in word:
            continue
        flags = flags.split()[0] if flags else ""
        case = "U" if len(word) > 1 and word.isupper() else "C" if word[0].isupper() else "l"
        emit(word, word, "base", case)
        suffixes(word, flags, word, "sfx", case)
        for pf in flags:
            if kind_of.get(pf) != "PFX":
                continue
            for strip, add, cont, rx in rules[pf]:
                if word.startswith(strip) and rx.search(word):
                    pw = add + word[len(strip):]
                    emit(pw, pw, "pfx", case)
                    sf = [x for x in flags if kind_of.get(x) == "SFX" and cross[x] and cross[pf]]
                    suffixes(pw, sf + list(cont), pw, "pfx", case)
print(f"RLA-ES: {dict(n)}", file=sys.stderr)
PY

# 2. Inputs of make_lexicon.py: forms.tsv (every non-clitic form with its
# lemma, plus the clitic forms, diminutives, superlatives and -mente adverbs
# the corpora attest), allowed.txt (those + frequent subtitle words outside
# RLA-ES: names, loanwords, slang), freq.txt (the subtitle counts minus the
# spellings left out: aqui, tambien, fué, English words, OCR slips; the plain
# word of an accent pair scaled down where subtitlers dropped the accent: mas).
"$PY" - "$S" "$S/$LEIPZIG.tar.gz" <<'PY'
import bz2, collections, re, sys, tarfile, unicodedata

S, LEIPZIG = sys.argv[1:3]
KEEP_FREQUENT = 300     # subtitle words outside RLA-ES: names, loanwords, slang
CLITIC_MIN = 2          # corpus hits an RLA-ES clitic form needs (dámelo)
NEW_MIN = 10            # ... a clitic form, diminutive, superlative or -mente
                        # adverb RLA-ES doesn't generate
ENGLISH_RATIO = 8.0     # left out when this much more frequent in English subtitles
HOLDOUT = 50
LETTERS = "a-zß-öø-ÿœ"  # kbcore::alphabet::CHARSET (Latin part)
WORD = re.compile(f"[{LETTERS}'-]*[{LETTERS}][{LETTERS}'-]*")
ARTIFACT = re.compile(r"^['-]|['-]$|^(\w{1,2})-\1")
UNACCENT = str.maketrans("áéíóú", "aeiou")
# RLA-ES quirks: the neuter pronouns never take an accent (it lists them to
# derive ésa, éstos).
NOT_WORDS = {"éso", "ésto"}


def bare(w):
    """w without diacritics: canción → cancion, año → ano, què → que."""
    return "".join(c for c in unicodedata.normalize("NFD", w) if not unicodedata.combining(c))
ACCENT = str.maketrans("aeiou", "áéíóú")


def nfc(w):
    return unicodedata.normalize("NFC", w).lower()


def read_freq(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            if len(p) >= 2 and p[1].isdigit():
                w = nfc(p[0])
                out[w] = max(out.get(w, 0), int(p[1]))
    return out


es = read_freq(f"{S}/es_full.txt")
en = read_freq(f"{S}/en_full.txt")
es_total, en_total = sum(es.values()), sum(en.values())

# Other attestations: the Leipzig word list, the Tatoeba training sentences.
other = collections.Counter()
with tarfile.open(LEIPZIG) as tar:
    m = next(m for m in tar.getmembers() if m.name.endswith("-words.txt"))
    for raw in tar.extractfile(m):
        p = raw.decode("utf-8", "replace").rstrip("\n").split("\t")
        if len(p) >= 3 and p[2].isdigit():
            other[nfc(p[1])] += int(p[2])
TOKEN = re.compile(r"\w+(?:['-]\w+)*")
tatoeba = collections.Counter()
with bz2.open(f"{S}/spa_sentences.tsv.bz2", "rt", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) == 3 and p[0].isdigit() and int(p[0]) % HOLDOUT:
            tatoeba.update(TOKEN.findall(nfc(p[2])))
other.update(tatoeba)

rla, clitic = {}, {}  # form → lemma
common = set()        # forms of a lowercase entry (not only of a name)
with open(f"{S}/rla_forms.tsv", encoding="utf-8") as f:
    for line in f:
        form, lemma, kind, case = line.rstrip("\n").split("\t")
        form, lemma = nfc(form), nfc(lemma)
        # Single letters are symbols (s, g, N) or the old ó between digits.
        if not WORD.fullmatch(form) or (len(form) == 1 and form not in "aeouy") or form in NOT_WORDS:
            continue
        (clitic if kind == "clitic" else rla).setdefault(form, lemma)
        if case == "l":
            common.add(form)
for w in rla:
    clitic.pop(w, None)


def count(w):
    return es.get(w, 0)


# Enclitics: verb forms they attach to, and the accent rules.
verb_form = {f: l for f, l in rla.items() if re.search("(ar|er|ir|ír)$", l)}
CL = ("me", "te", "se", "nos", "os", "le", "les", "lo", "la", "los", "las")
VOWEL = set("aeiouáéíóúü")
STRONG = set("aeoáéóíú")  # í, ú in hiatus count as strong


def nuclei(w):
    """Index spans of the syllable nuclei (vowel groups, split between two
    strong vowels): mee → e|e, comiendo → o, ie, o."""
    out, i = [], 0
    while i < len(w):
        if w[i] in VOWEL:
            j = i + 1
            while j < len(w) and w[j] in VOWEL and not (w[j - 1] in STRONG and w[j] in STRONG):
                j += 1
            out.append((i, j))
            i = j
        else:
            i += 1
    return out


def stressed(w):
    """The stressed nucleus of a word, by the spelling rules."""
    ns = nuclei(w)
    for k, (i, j) in enumerate(ns):
        if any(ch in "áéíóú" for ch in w[i:j]):
            return k
    return max(0, len(ns) - 2) if w[-1] in VOWEL or w[-1] in "ns" else len(ns) - 1


def accent_ok(w, base):
    """Is w (base + enclitics) accented right? The stress stays on the verb's
    syllable: two or more syllables after it take an accent (dámelo,
    haciéndolo), one doesn't (dame, decime, comerlo) — except í, ú in hiatus
    (oírlo)."""
    k = stressed(base)
    ns = nuclei(w)
    if k >= len(ns):
        return False
    marks = [i for i, ch in enumerate(w) if ch in "áéíóú"]
    i, j = ns[k]
    if len(ns) - 1 - k >= 2:
        return len(marks) == 1 and i <= marks[0] < j
    if not marks:
        return True
    m = marks[0]
    hiatus = w[m] in "íú" and ((m and w[m - 1] in VOWEL) or (m + 1 < len(w) and w[m + 1] in VOWEL))
    return len(marks) == 1 and i <= m < j and hiatus


def last_vowel_accented(w):
    for i in range(len(w) - 1, -1, -1):
        if w[i] in "aeiou":
            return w[:i] + w[i].translate(ACCENT) + w[i + 1:]
    return w


def clitic_lemma(w):
    """The verb lemma if w is a verb form + 1–3 enclitics, accented right
    (dámelo, vámonos, decime, comeos)."""
    def strip(rest, cls):
        if cls:
            plain = rest.translate(UNACCENT)
            stems = [rest, plain, last_vowel_accented(plain)]  # decí + me → decime
            if cls[0] == "os":            # comed + os → comeos
                stems.append(plain + "d")
            if cls[0] in ("nos", "se"):   # vamos + nos → vámonos
                stems.append(plain + "s")
            for stem in stems:
                # a short stem is too often a name (carlo, marla)
                if stem in verb_form and (len(stem) >= 4 or stem != rest) and accent_ok(w, stem):
                    return verb_form[stem]
        if len(cls) < 3:
            for c in CL:
                if rest.endswith(c) and len(rest) > len(c):
                    r = strip(rest[: -len(c)], [c] + cls)
                    if r:
                        return r
        return None
    return strip(w, [])


unaccented = {}
for form, lemma in rla.items():
    unaccented.setdefault(form.translate(UNACCENT), lemma)
DIMINUTIVE = ("ecitos", "ecitas", "ecito", "ecita", "citos", "citas", "cito", "cita",
              "itos", "itas", "ito", "ita", "ísimos", "ísimas", "ísimo", "ísima")


def derived_lemma(w):
    """The lemma if w is an adverb in -mente (lentamente), a diminutive
    (cafecito, poquita) or a superlative (carísimo) of a dictionary word."""
    if w.endswith("mente"):
        return rla.get(w[:-5]) if len(w) > 8 else None
    if re.search("[áéíóú]", w) and not re.search("ísim[oa]s?$", w):
        return None  # the stress moves to the suffix: no other accent
    for suf in DIMINUTIVE:
        base = w[: -len(suf)]
        if w.endswith(suf) and len(base) >= 3:
            bases = {base}
            if base.endswith("qu"):   # poquito ← poco
                bases.add(base[:-2] + "c")
            if base.endswith("gu"):   # amiguito ← amigo
                bases.add(base[:-2] + "g")
            for b in bases:
                for end in ("", "o", "a", "e", "os", "as", "es", "s"):
                    if b + end in unaccented:
                        return unaccented[b + end]
    return None


# Spellings grouped by their letters without diacritics (aqui/aquí, tí/ti,
# fué/fue, què/que): the most frequent one is the word, the others are slips.
group_best = {}


def offer(w, c):
    k = bare(w)
    if c > group_best.get(k, (0, ""))[0]:
        group_best[k] = (c, w)


for w in rla:
    offer(w, count(w) + 1)   # a dictionary spelling wins ties
for w, c in es.items():
    if WORD.fullmatch(w):
        offer(w, c)


def minority_spelling(w):
    return group_best.get(bare(w), (0, w))[1] != w


# Names spelled like a dictionary word without its accent (Tio, Dificil —
# places — next to tío, difícil) or like a 3× more frequent name (Maria:
# María) would make the slip a word: they go. (Caín, Rumanía stay, however
# the subtitles spell them.)
spellings = collections.defaultdict(list)
for w in rla:
    spellings[bare(w)].append(w)


def shadowed(w):
    return any(v != w and (v in common or count(v) >= 3 * max(1, count(w)))
               for v in spellings[bare(w)])


for w in [w for w in rla if w not in common and shadowed(w)]:
    del rla[w]


def ocr_error(w):
    """Subtitle OCR reads l as I: ias (las), eI → ei (el)."""
    for i, ch in enumerate(w):
        if ch == "i":
            v = w[:i] + "l" + w[i + 1:]
            if v in rla and count(v) > 5 * count(w):
                return True
    return False


forms = dict(rla)
kept_clitic = {w: l for w, l in clitic.items() if count(w) >= CLITIC_MIN or other[w] >= CLITIC_MIN}
new_clitic, derived, corpus_only, dropped = {}, {}, {}, collections.Counter()
for w in set(es) | set(other):
    c = count(w)
    if w in rla or w in clitic or not WORD.fullmatch(w) or c + other[w] < NEW_MIN:
        continue
    if not minority_spelling(w) and not ocr_error(w):
        lemma = clitic_lemma(w)
        if lemma:
            new_clitic[w] = lemma
            continue
        lemma = derived_lemma(w)
        if lemma:
            derived[w] = lemma
            continue
    if c < KEEP_FREQUENT:
        continue
    if len(w) <= 2 or ARTIFACT.search(w):
        dropped["short/artifact"] += 1
    elif ocr_error(w):
        dropped["OCR l→i"] += 1
    elif minority_spelling(w):
        dropped["accent variant"] += 1
    elif en.get(w, 0) / en_total > ENGLISH_RATIO * c / es_total:
        dropped["English"] += 1
    else:
        corpus_only[w] = c
forms.update(kept_clitic)
forms.update(new_clitic)
forms.update(derived)

with open(f"{S}/forms.tsv", "w", encoding="utf-8") as f:
    for w in sorted(forms):
        f.write(f"{w}\t{forms[w]}\n")
keep = set(forms) | set(corpus_only)
with open(f"{S}/allowed.txt", "w", encoding="utf-8") as f:
    f.writelines(w + "\n" for w in sorted(keep))

# Subtitlers drop accents: the plain word of an accent pair is overcounted
# (mas for más, tenia for tenía). Where Tatoeba (curated) has the plain word
# over 5× rarer next to the accented one than the subtitles do, its count is
# scaled to Tatoeba's ratio.
accented = {}  # plain word → its most frequent accented spelling
for w in keep:
    p = bare(w)
    if p != w and p in keep and (p not in accented or count(w) > count(accented[p])):
        accented[p] = w
scaled = {}
for p, a in accented.items():
    cp, ca, tp, ta = count(p), count(a), tatoeba[p], tatoeba[a]
    ratio = (tp + 0.5) / (ta + 0.5)
    if 0 < cp < ca and tp + ta >= 20 and cp > 5 * ratio * ca:
        scaled[p] = max(3, round(ratio * ca))
with open(f"{S}/freq.txt", "w", encoding="utf-8") as f:
    for w, c in sorted(es.items(), key=lambda x: -x[1]):
        if w in keep:
            f.write(f"{w} {scaled.get(w, c)}\n")
top = sorted(scaled, key=lambda p: -count(p))[:12]
print(f"{len(scaled)} plain spellings of accent pairs scaled down: "
      + ", ".join(f"{p} {count(p)}→{scaled[p]} ({accented[p]} {count(accented[p])})" for p in top),
      file=sys.stderr)
print(f"RLA-ES forms {len(rla)}; clitic forms: {len(kept_clitic)} of RLA-ES's {len(clitic)} "
      f"attested, {len(new_clitic)} more from the corpora; {len(derived)} diminutives, "
      f"superlatives, -mente adverbs; {len(corpus_only)} subtitle words outside RLA-ES "
      f"(left out: {dict(dropped)})", file=sys.stderr)
PY

"$PY" tools/make_lexicon.py "$S/freq.txt" --words "$S/allowed.txt" --forms "$S/forms.tsv" \
    --keep-frequent 1000000000 --extra $D/slang.tsv -o $D/lexicon.tsv

# The Leipzig sentences for the context model and the slip rules, as a
# Tatoeba-style file (odd ids: none is held out), minus the ~4.5 % written
# without accents (tambien, informacion, estan) — they would teach «esta
# bien» for «está bien», «mas» for «más».
"$PY" - "$S/$LEIPZIG.tar.gz" $D/lexicon.tsv <<'PY' | bzip2 > "$S/$LEIPZIG.clean.tsv.bz2"
import re, sys, tarfile, unicodedata
with open(sys.argv[2], encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}
# Words spelled without their accent: the bare form of an accented word.
bare = lambda w: "".join(c for c in unicodedata.normalize("NFD", w) if not unicodedata.combining(c))
unaccented = {bare(w) for w in lexicon} - lexicon
TOKEN = re.compile(r"[^\W\d_]+")
n = kept = 0
with tarfile.open(sys.argv[1]) as tar:
    m = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
    for raw in tar.extractfile(m):
        text = unicodedata.normalize("NFC", raw.decode("utf-8", "replace").rstrip("\n").split("\t", 1)[-1])
        n += 1
        if not any(t in unaccented for t in TOKEN.findall(text.lower())):
            kept += 1
            sys.stdout.write(f"{2 * n + 1}\tspa\t{text}\n")
print(f"Leipzig: kept {kept} of {n} sentences (the rest lack accents)", file=sys.stderr)
PY

# 3. Capitals: `word<TAB>1` (Capitalized) / `word<TAB>2` (ALL CAPS), by how the
# word is written inside sentences (not first, not after punctuation other
# than a comma) in Tatoeba and the Leipzig corpus; for words rarely seen there,
# by how RLA-ES spells them.
"$PY" - "$S" "$S/$LEIPZIG.tar.gz" $D/lexicon.tsv $D/slang.tsv > $D/proper.tsv <<'PY'
import bz2, collections, re, sys, tarfile, unicodedata

S, LEIPZIG, LEXICON, SLANG = sys.argv[1:5]
HOLDOUT = 50
MIN_SEEN = 5             # occurrences inside sentences to decide by the corpora
CAP_SHARE = 0.9          # of them capitalized → a name
CAP_SHARE_COMMON = 0.97  # the same for a word RLA-ES also has in lowercase
UPPER_SHARE = 0.8        # of them in capitals → an abbreviation
# Spanish writes these in lowercase (a sentence or a date may capitalize them).
LOWER = set("""lunes martes miércoles jueves viernes sábado domingo enero febrero
marzo abril mayo junio julio agosto septiembre setiembre octubre noviembre
diciembre usted ustedes señor señora don doña""".split())

with open(LEXICON, encoding="utf-8") as f:
    lexicon = {line.split("\t", 1)[0] for line in f}
with open(SLANG, encoding="utf-8") as f:  # chat words stay as typed (sip, omg)
    slang = {line.split("\t", 1)[0] for line in f if not line.startswith("#")}

TOKEN = re.compile(r"[^\W\d_]+(?:['-][^\W\d_]+)*|\S")
lower, cap, upper = collections.Counter(), collections.Counter(), collections.Counter()


def count(text):
    letters = [ch for ch in text if ch.isalpha()]
    if not letters or sum(ch.isupper() for ch in letters) > 0.5 * len(letters):
        return  # a headline in capitals
    prev = None
    for tok in TOKEN.findall(unicodedata.normalize("NFC", text)):
        if tok[0].isalpha() and prev is not None and (prev[0].isalpha() or prev == ","):
            key = tok.lower()
            if key in lexicon:
                if tok == key:
                    lower[key] += 1
                elif len(tok) > 1 and tok == tok.upper():
                    upper[key] += 1
                elif tok[0].isupper():
                    cap[key] += 1
        prev = tok


with bz2.open(f"{S}/spa_sentences.tsv.bz2", "rt", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) == 3 and p[0].isdigit() and int(p[0]) % HOLDOUT:
            count(p[2])
with tarfile.open(LEIPZIG) as tar:
    m = next(m for m in tar.getmembers() if m.name.endswith("-sentences.txt"))
    for raw in tar.extractfile(m):
        count(raw.decode("utf-8", "replace").rstrip("\n").split("\t", 1)[-1])

spelled = collections.defaultdict(set)  # how RLA-ES spells a form: l, C, U
with open(f"{S}/rla_forms.tsv", encoding="utf-8") as f:
    for line in f:
        form, _lemma, _kind, case = line.rstrip("\n").split("\t")
        spelled[unicodedata.normalize("NFC", form).lower()].add(case)

CAPITAL, UPPER = 1, 2
casing = {}
for w in lexicon:
    if w in LOWER or w in slang or len(w) < 2:
        continue
    n = lower[w] + cap[w] + upper[w]
    how = spelled.get(w, set())
    if n >= MIN_SEEN:
        # In capitals: seen so ≥ 10 times, and not also a dictionary word in
        # lowercase (unid, leed, epa); two letters only as RLA-ES has them (UE).
        if (upper[w] >= max(10, UPPER_SHARE * n) and "l" not in how
                and (len(w) >= 3 or "U" in how)):
            casing[w] = UPPER
        elif (cap[w] >= 2 * upper[w]  # not TIC, Xi, NASA
              and cap[w] + upper[w] >= (CAP_SHARE_COMMON if "l" in how else CAP_SHARE) * n):
            casing[w] = CAPITAL
    elif how == {"U"}:
        casing[w] = UPPER
    elif how == {"C"}:
        casing[w] = CAPITAL
for w in sorted(casing):
    sys.stdout.write(f"{w}\t{casing[w]}\n")
print(f"proper.tsv: {len(casing)} words ({sum(v == UPPER for v in casing.values())} in capitals)",
      file=sys.stderr)
PY

# 4. Context model: word pairs and endings (Tatoeba outweighs the web; no news).
"$PY" tools/build_bigrams.py "$S/spa_sentences.tsv.bz2:0.6" "$S/$LEIPZIG.clean.tsv.bz2:0.4" \
    --lexicon $D/lexicon.tsv --endings $D/endings.tsv --endings-any-script > $D/bigrams.tsv

# 5. Real-word slips (docs/rules-format.md): pairs one slip or one accent apart
# (el/él, esta/está, año/ano), their contexts, decision lists.
"$PY" tools/confusion_sets.py --lexicon $D/lexicon.tsv --rows qwertyuiop asdfghjklñ zxcvbnm \
    > $D/confusions.tsv
"$PY" tools/export_observations.py "$S/spa_sentences.tsv.bz2" "$S/$LEIPZIG.clean.tsv.bz2" \
    --confusions $D/confusions.tsv --classes /nonexistent | bzip2 > $D/observations.tsv.bz2
bzcat $D/observations.tsv.bz2 | "$PY" tools/decision_lists.py --confusions $D/confusions.tsv > $D/rules.tsv

# 6. FSTs, when the index builder is there.
if [ -x "$IB" ]; then
    "$IB" $D/lexicon.tsv $D/dict.fst
    "$IB" --casing $D/proper.tsv $D/casing.fst
    "$IB" --bigrams $D/bigrams.tsv $D/bigrams.fst --endings $D/endings.tsv \
        --rules $D/confusions.tsv $D/rules.tsv
fi

# 7. Stats: sizes, coverage of the held-out Tatoeba sentences, the top words.
"$PY" - $D <<'PY'
import bz2, collections, os, re, sys, unicodedata
D = sys.argv[1]
lex = {}
with open(f"{D}/lexicon.tsv", encoding="utf-8") as f:
    for line in f:
        w, c = line.rstrip("\n").split("\t")
        lex[w] = int(c)


def lines(name):
    with open(f"{D}/{name}", encoding="utf-8") as f:
        return sum(1 for _ in f)


print(f"lexicon {len(lex)} forms; proper {lines('proper.tsv')}; bigrams {lines('bigrams.tsv')}; "
      f"endings {lines('endings.tsv')}; confusion pairs {lines('confusions.tsv')}; rules {lines('rules.tsv')}")
for fst in ("dict.fst", "casing.fst", "bigrams.fst"):
    if os.path.exists(f"{D}/{fst}"):
        print(f"{fst}: {os.path.getsize(f'{D}/{fst}') / 1e6:.2f} MB")
TOK = re.compile(r"[^\W\d_]+(?:['-][^\W\d_]+)*")
n = hit = na = ha = 0
miss = collections.Counter()
with bz2.open(f"{D}/src/spa_sentences.tsv.bz2", "rt", encoding="utf-8") as f:
    for line in f:
        p = line.rstrip("\n").split("\t")
        if len(p) != 3 or not p[0].isdigit() or int(p[0]) % 50:
            continue
        for t in TOK.findall(unicodedata.normalize("NFC", p[2])):
            w = t.lower()
            n += 1
            hit += w in lex
            if re.search("[áéíóúüñ]", w):
                na += 1
                ha += w in lex
            if w not in lex:
                miss[t] += 1
print(f"held-out Tatoeba tokens (ids % 50 == 0): {n}, in the lexicon {100 * hit / n:.2f} %; "
      f"with á é í ó ú ü ñ: {na}, {100 * ha / na:.2f} %")
print("most frequent missing:", " ".join(w for w, _ in miss.most_common(25)))
print("top words:", " ".join(sorted(lex, key=lambda w: -lex[w])[:40]))
PY
