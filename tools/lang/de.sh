#!/usr/bin/env bash
# German (de) language pack: fetch the sources into data/de/src/ (what is
# there already is kept), build every data file under data/de/, print stats.
# Idempotent; helpers in tools/lang/de_data.py (stdlib Python).
#
# Sources (all redistributable; see data/de/README.md):
#   de_full.txt                  hermitdave/FrequencyWords 2018 (OpenSubtitles)
#                                — word counts, the frequency prior; CC BY-SA 4.0
#   german-pos-dict-1.2.4.jar    LanguageTool german-pos-dict (Daniel Naber; data
#                                from Morphy, extended by korrekturen.de) — ~5 M
#                                tagged forms; CC BY-SA 4.0 (Maven Central)
#   kaikki-German.jsonl.gz       German entries of the English Wiktionary,
#                                extracted by kaikki.org (wiktextract) —
#                                inflection tables, compounds, names;
#                                CC BY-SA 4.0 + GFDL (Wiktionary contributors)
#   deu_sentences.tsv.bz2        Tatoeba German sentences — context model,
#                                capitalization; CC BY 2.0 FR
#   deu-de_web-public_2019_1M    Leipzig Corpora Collection, German web 2019 —
#                                context model, capitalization; CC BY
#   SCOWL 2020.12.07 (data/scowl or data/de/src/scowl) — English words and
#                                names, only to keep English out; permissive
#   data/de/slang.tsv            hand-curated (this repo)
#
# Outputs: data/de/{lexicon,proper,bigrams,endings,confusions,rules}.tsv,
# observations.tsv.bz2, and the FSTs dict.fst, casing.fst, bigrams.fst
# (built with the prebuilt target/release/index-builder; skipped if absent).
set -euo pipefail
cd "$(dirname "$0")/../.."
D=data/de
S=$D/src
W=$S/work
PY=${PYTHON:-python3}
DE="$PY tools/lang/de_data.py"
# No step needs the shared heavy-build lock: each takes < 2 min and < 2 GB
# (the forms step ~1.5 min / 1.5 GB, bigrams 25 s, observations 1 min).
mkdir -p "$S" "$W"

fetch() {  # fetch URL FILE: download unless present
    [ -s "$2" ] && return
    echo "fetching $1" >&2
    curl -fsSL -o "$2.part" "$1" && mv "$2.part" "$2"
}
fetch https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/de/de_full.txt "$S/de_full.txt"
fetch https://repo1.maven.org/maven2/de/danielnaber/german-pos-dict/1.2.4/german-pos-dict-1.2.4.jar "$S/german-pos-dict-1.2.4.jar"
fetch https://kaikki.org/dictionary/German/kaikki.org-dictionary-German.jsonl.gz "$S/kaikki-German.jsonl.gz"
fetch https://downloads.tatoeba.org/exports/per_language/deu/deu_sentences.tsv.bz2 "$S/deu_sentences.tsv.bz2"
fetch https://downloads.wortschatz-leipzig.de/corpora/deu-de_web-public_2019_1M.tar.gz "$S/deu-de_web-public_2019_1M.tar.gz"
SCOWL=$(ls -d data/scowl/scowl-*/final 2>/dev/null | tail -1 || true)
if [ -z "$SCOWL" ]; then
    SCOWL=$(ls -d $S/scowl/scowl-*/final 2>/dev/null | tail -1 || true)
fi
if [ -z "$SCOWL" ]; then
    mkdir -p $S/scowl
    curl -fsSL "https://downloads.sourceforge.net/project/wordlist/SCOWL/2020.12.07/scowl-2020.12.07.tar.gz" | tar xz -C $S/scowl
    SCOWL=$(ls -d $S/scowl/scowl-*/final | tail -1)
fi
TATOEBA=$S/deu_sentences.tsv.bz2
LEIPZIG=$S/deu-de_web-public_2019_1M.tar.gz

fresh() {  # fresh TARGET SOURCE...: TARGET exists and is newer than every SOURCE
    local t=$1; shift
    [ -s "$t" ] || return 1
    for s in "$@"; do [ "$t" -nt "$s" ] || return 1; done
}

# 1. The morphological sources as plain tables.
fresh $W/lt.tsv $S/german-pos-dict-1.2.4.jar tools/lang/de_data.py ||
    $DE lt $S/german-pos-dict-1.2.4.jar > $W/lt.tsv
fresh $W/kaikki.tsv $S/kaikki-German.jsonl.gz tools/lang/de_data.py ||
    $DE kaikki $S/kaikki-German.jsonl.gz > $W/kaikki.tsv
# How each word is capitalized inside sentences (Tatoeba + web).
fresh $W/casing.tsv $TATOEBA $LEIPZIG tools/lang/de_data.py ||
    $DE casing $TATOEBA $LEIPZIG > $W/casing.tsv

# 2. Word forms: both sources, attested compounds with their forms, frequent
# corpus words the sources lack (names, loanwords), X's contractions; not the
# lemmas no corpus has ever seen (a third of the forms, 0.07 % of the text).
english=(); names=()
for l in 10 20 35 40 50; do
    english+=(--english "$SCOWL/english-words.$l")
    for f in "$SCOWL"/{english,american,british}-{upper,proper-names}.$l; do
        [ -f "$f" ] && names+=(--names "$f")
    done
done
$DE forms --lt $W/lt.tsv --kaikki $W/kaikki.tsv --freq $S/de_full.txt \
    "${english[@]}" "${names[@]}" --casing $W/casing.tsv --corpus $TATOEBA --corpus $LEIPZIG \
    --drop-unattested --forms $W/forms.tsv --readings $W/readings.tsv --extra $W/extra.tsv \
    --compounds $W/compounds.tsv

# 3. The lexicon: every form; a form seen in the subtitles keeps its count,
# an unseen one gets 2 % of its lemma's most frequent form. Corpus words come
# only through forms.tsv and extra.tsv (--keep-frequent is off: de_data.py
# filtered them — English, OCR errors like lch, ae/oe/ue/ss substitutes).
$PY tools/make_lexicon.py $S/de_full.txt --words $W/forms.tsv --forms $W/forms.tsv \
    --keep-frequent 1000000000 --extra $D/slang.tsv --extra $W/extra.tsv -o $W/lexicon.raw.tsv
# Single letters are no words in German: kept (typed alone they stay) but rare
# (their subtitle counts are clipped articles: 's, 'n).
awk -F'\t' 'BEGIN{OFS="\t"} length($1) == 1 && $2 > 20 {$2 = 20} {print}' $W/lexicon.raw.tsv > $D/lexicon.tsv

# 4. Capitals: nouns and names (not also common in lowercase), abbreviations.
$DE proper --lexicon $D/lexicon.tsv --readings $W/readings.tsv --casing $W/casing.tsv > $D/proper.tsv

# 5. Context model: everyday sentences outweigh the web; no news.
corpora=($TATOEBA:0.6 $LEIPZIG:0.4)
$PY tools/build_bigrams.py "${corpora[@]}" --lexicon $D/lexicon.tsv \
    --endings $D/endings.tsv --endings-any-script > $D/bigrams.tsv

# 6. Real-word slips: pairs one key apart on QWERTZ, what follows them, rules.
$PY tools/confusion_sets.py --lexicon $D/lexicon.tsv --rows qwertzuiopü asdfghjklöä yxcvbnm > $D/confusions.tsv
$PY tools/export_observations.py $TATOEBA $LEIPZIG --confusions $D/confusions.tsv \
    --classes /nonexistent | bzip2 > $D/observations.tsv.bz2
bzcat $D/observations.tsv.bz2 | $PY tools/decision_lists.py --confusions $D/confusions.tsv > $D/rules.tsv

# 7. FSTs (prebuilt index-builder; the app build makes its own).
IB=target/release/index-builder
if [ -x $IB ]; then
    $IB $D/lexicon.tsv $D/dict.fst
    $IB --casing $D/proper.tsv $D/casing.fst
    $IB --bigrams $D/bigrams.tsv $D/bigrams.fst --endings $D/endings.tsv \
        --rules $D/confusions.tsv $D/rules.tsv
fi

# Stats.
echo "== data/de"
printf "lexicon %s forms, proper %s (%s ALL CAPS), bigrams %s, endings %s, confusions %s, rules %s\n" \
    "$(wc -l < $D/lexicon.tsv)" "$(wc -l < $D/proper.tsv)" "$(grep -c $'\t2$' $D/proper.tsv)" \
    "$(wc -l < $D/bigrams.tsv)" "$(wc -l < $D/endings.tsv)" "$(wc -l < $D/confusions.tsv)" "$(wc -l < $D/rules.tsv)"
ls -l $D/*.fst 2>/dev/null | awk '{printf "%s %.1f MB\n", $NF, $5 / 1048576}'
$DE coverage --lexicon $D/lexicon.tsv $TATOEBA --show 40
echo "top 40:"; sort -t$'\t' -k2,2nr $D/lexicon.tsv | head -40 | cut -f1 | tr '\n' ' '; echo
