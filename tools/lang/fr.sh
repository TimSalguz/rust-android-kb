#!/usr/bin/env bash
# French language pack: fetch the sources into data/fr/src/ (what is there is
# kept), build data/fr/{lexicon,proper,bigrams,endings,confusions,rules}.tsv,
# observations.tsv.bz2 and — with target/release/index-builder — the FSTs,
# then print stats. Idempotent; data/fr/README.md explains the choices.
#   fr_full.txt                     — hermitdave/FrequencyWords 2018, OpenSubtitles
#                                     counts (CC BY-SA 4.0): the frequency prior
#   lexique-grammalecte-fr-v7.7.zip — Grammalecte / Dicollecte (Olivier R. et al.),
#                                     every inflected form with its lemma and tags
#                                     (MPL 2.0, see data/fr/GRAMMALECTE-NOTICE)
#   fra_sentences.tsv.bz2           — Tatoeba sentences (CC BY 2.0 FR)
#   fra-fr_web_2013_1M.tar.gz       — Leipzig Corpora Collection, French web
#                                     (© Universität Leipzig / SAW / InfAI, CC BY)
#   data/fr/slang.tsv               — hand-curated (this repo)
# Heavy steps (the context model, the observations) run under $HEAVY, e.g.
#   HEAVY="nice -n 10" tools/lang/fr.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
D=data/fr
S=$D/src
PY=${PYTHON:-python3}
PREP="$PY tools/lang/fr_prep.py"
HEAVY=${HEAVY:-}
mkdir -p "$S"

fetch() {  # fetch FILE URL — once
    [ -s "$S/$1" ] || { echo "fetching $2" >&2; curl -fsSL -o "$S/$1.part" "$2" && mv "$S/$1.part" "$S/$1"; }
}
fetch fr_full.txt https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/fr/fr_full.txt
fetch lexique-grammalecte-fr-v7.7.zip https://grammalecte.net/dic/lexique-grammalecte-fr-v7.7.zip
fetch fra_sentences.tsv.bz2 https://downloads.tatoeba.org/exports/per_language/fra/fra_sentences.tsv.bz2
fetch fra-fr_web_2013_1M.tar.gz https://downloads.wortschatz-leipzig.de/corpora/fra-fr_web_2013_1M.tar.gz
G=$S/lexique-grammalecte-fr-v7.7.txt
[ -s "$G" ] || unzip -o -q "$S/lexique-grammalecte-fr-v7.7.zip" -d "$S"
LEIPZIG=$S/fra-fr_web_2013_1M.tar.gz

# Lexicon: every Grammalecte form; subtitle counts where seen, else 2 % of the
# lemma's most frequent form; hyphenated verb + pronoun forms (est-ce, dis-moi)
# from the subtitles; slang. Elided clitics (l' qu' jusqu') are words of their own.
$PREP forms "$G" > "$S/forms.tsv"
$PREP freq "$G" "$S/fr_full.txt" "$LEIPZIG" --freq-out "$S/freq.txt" \
    --allowed-out "$S/allowed.txt" --extra-out "$S/hyphenated.tsv"
$PY tools/make_lexicon.py "$S/freq.txt" --words "$S/allowed.txt" --keep-frequent 1000000000 \
    --forms "$S/forms.tsv" --form-share 0.02 --extra "$D/slang.tsv" -o "$D/lexicon.tsv"

# Capitals: Grammalecte's proper nouns and abbreviations, checked against the web corpus.
$PREP proper "$G" "$LEIPZIG" "$D/lexicon.tsv" > "$D/proper.tsv"

# Context model: sentences with the clitics split off (l'homme → l' homme),
# Tatoeba 0.6 + Leipzig web 0.4 (no news: it drags politics into the guesses).
TATOEBA=$S/fra_sentences_split.tsv.bz2
WEB=$S/fra-fr_web_2013_1M_split.tsv.bz2
[ "$TATOEBA" -nt tools/lang/fr_prep.py ] || $PREP tatoeba "$S/fra_sentences.tsv.bz2" "$TATOEBA"
[ "$WEB" -nt tools/lang/fr_prep.py ] || $PREP leipzig "$LEIPZIG" "$WEB"
$HEAVY $PY tools/build_bigrams.py "$TATOEBA:0.6" "$WEB:0.4" --lexicon "$D/lexicon.tsv" \
    --endings "$D/endings.tsv" --endings-any-script > "$D/bigrams.tsv"

# Real-word slips on AZERTY (the apostrophe key ends the bottom row), rules on the next word.
$PY tools/confusion_sets.py --lexicon "$D/lexicon.tsv" --rows azertyuiop qsdfghjklm "wxcvbn'" \
    > "$D/confusions.tsv"
# Plus the classic French pairs the tool can't find (a letter left out, or
# homophones): et/est, on/ont, son/sont, a/as, peu/peut, ses/ces, ce/se, sa/ça.
printf '%s\t%s\n' et est on ont son sont a as peu peut ses ces ce se sa ça |
    awk -F'\t' 'NR == FNR { seen[$1 FS $2]; seen[$2 FS $1]; next } !($0 in seen)' "$D/confusions.tsv" - \
    >> "$D/confusions.tsv"
$HEAVY $PY tools/export_observations.py "$TATOEBA" "$WEB" --confusions "$D/confusions.tsv" \
    --classes /nonexistent | bzip2 > "$D/observations.tsv.bz2"
bzcat "$D/observations.tsv.bz2" | $PY tools/decision_lists.py --confusions "$D/confusions.tsv" > "$D/rules.tsv"

# FSTs (the prebuilt index builder; build it with cargo first).
IB=target/release/index-builder
if [ -x "$IB" ]; then
    $IB "$D/lexicon.tsv" "$D/dict.fst"
    $IB --casing "$D/proper.tsv" "$D/casing.fst"
    $IB --bigrams "$D/bigrams.tsv" "$D/bigrams.fst" --endings "$D/endings.tsv" \
        --rules "$D/confusions.tsv" "$D/rules.tsv"
fi

echo "== data/fr"
for f in lexicon proper bigrams endings confusions rules; do
    printf '%-16s %8s lines\n' "$f.tsv" "$(wc -l < "$D/$f.tsv")"
done
for f in "$D"/*.fst; do
    [ -e "$f" ] && printf '%-16s %8s bytes\n' "$(basename "$f")" "$(stat -c %s "$f")"
done
echo "top words: $(sort -t$'\t' -k2,2nr "$D/lexicon.tsv" | head -30 | cut -f1 | tr '\n' ' ')"
$PREP coverage "$S/fra_sentences.tsv.bz2" "$D/lexicon.tsv" --show 30
