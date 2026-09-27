#!/usr/bin/env bash
# Rebuild data/lexicon.tsv (`word<TAB>count`, ~3 M words) from open sources
# (tools/fetch_sources.sh downloads them):
#   $FREQ_DIR/{ru,en}_full.txt — hermitdave/FrequencyWords 2018 (CC BY-SA 4.0)
#   data/opencorpora_forms.tsv — tools/opencorpora_forms.py (OpenCorpora, CC BY-SA)
#   data/hunspell_forms.txt    — tools/hunspell_forms.py (LibreOffice ru_RU, BSD-style)
#   data/scowl/scowl-*/final   — SCOWL 2020.12.07 (Kevin Atkinson, permissive)
#   data/slang.tsv             — hand-curated (this repo)
# Russian: every OpenCorpora form, with corpus counts where seen; corpus words
# outside OpenCorpora only when frequent (names, slang, loanwords). English:
# SCOWL ≤ 70 with corpus counts.
set -euo pipefail
cd "$(dirname "$0")/.."
FREQ_DIR=${FREQ_DIR:-data/freq}
SCOWL=$(ls -d data/scowl/scowl-*/final | tail -1)

# English: SCOWL up to level 70 (US + GB + common), contractions, frequent
# names. Words unseen in the corpus get a floor count by level.
declare -A FLOOR=([10]=200 [20]=100 [35]=30 [40]=15 [50]=8 [55]=5 [60]=3 [70]=1)
# Subtitle counts split contractions at the apostrophe (don + 't), so they
# get their own, high floors: they are among the most common English words.
declare -A CONTRACTION=([10]=50000 [35]=20000 [40]=2000 [50]=2000 [60]=2000 [70]=2000)
lists=()
for level in 10 20 35 40 50 55 60 70; do
    for kind in english-words american-words british-words english-upper american-upper british-upper; do
        f=$SCOWL/$kind.$level
        [ -f "$f" ] && lists+=(--wordlist "$f:${FLOOR[$level]}")
    done
    f=$SCOWL/english-contractions.$level
    [ -f "$f" ] && lists+=(--wordlist "$f:${CONTRACTION[$level]}")
done

# Slang inflected by analogy with a model word (needs pymorphy3).
${PYTHON:-python3} tools/expand_slang.py data/slang.tsv > data/slang_forms.tsv

python3 tools/make_lexicon.py "$FREQ_DIR/ru_full.txt" "$FREQ_DIR/en_full.txt" \
    --words data/opencorpora_forms.tsv --cyrillic-words-only --latin-lists-only \
    --forms data/opencorpora_forms.tsv --wordlist data/hunspell_forms.txt:1 \
    --extra data/slang.tsv --extra data/slang_forms.tsv "${lists[@]}" --yo-variants \
    -o data/lexicon.tsv

# Names, places and abbreviations the keyboard writes with capitals.
${PYTHON:-python3} tools/proper_nouns.py > data/proper.tsv

# The context model, when its corpora are there (LEIPZIG=1 fetch_sources.sh):
# everyday sentences (Tatoeba) outweigh the web; no news (it drags politics
# into the guesses).
corpora=(data/tatoeba/rus_sentences.tsv.bz2:0.6 data/leipzig/rus-ru_web-public_2019_1M.tar.gz:0.4
         data/tatoeba/eng_sentences.tsv.bz2:0.6 data/leipzig/eng-com_web-public_2018_1M.tar.gz:0.4)
have=1
for c in "${corpora[@]}"; do [ -s "${c%:*}" ] || have=0; done
if [ $have = 1 ]; then
    python3 tools/build_bigrams.py "${corpora[@]}" --endings data/endings.tsv > data/bigrams.tsv
    # Grammar classes (OpenCorpora readings) and which tag follows which.
    ${PYTHON:-python3} tools/build_classes.py data/tatoeba/rus_sentences.tsv.bz2 \
        data/leipzig/rus-ru_web-public_2019_1M.tar.gz --classes data/classes.tsv \
        --class-tags data/class_tags.tsv --readings data/word_readings.tsv data/readings.tsv \
        > data/tag_pairs.tsv
    # Government frames and agreement weights for the phrase grammar.
    python3 tools/build_frames.py data/tatoeba/rus_sentences.tsv.bz2 \
        data/leipzig/rus-ru_web-public_2019_1M.tar.gz > data/frames.tsv
    # Where commas go.
    python3 tools/build_commas.py data/tatoeba/rus_sentences.tsv.bz2 \
        data/leipzig/rus-ru_web-public_2019_1M.tar.gz > data/commas.tsv
    # Sense classes: which words go together in a sentence (needs numpy).
    ${PYTHON:-python3} tools/build_topics.py data/tatoeba/rus_sentences.tsv.bz2 \
        data/leipzig/rus-ru_web-public_2019_1M.tar.gz --words data/topic_words.tsv \
        > data/topic_pairs.tsv
    # Context rules on the next word for words one slip apart
    # (docs/rules-format.md): pairs, observations, decision lists.
    python3 tools/confusion_sets.py > data/confusions.tsv
    ${PYTHON:-python3} tools/export_observations.py "${corpora[@]%:*}" | bzip2 > data/observations.tsv.bz2
    bzcat data/observations.tsv.bz2 | python3 tools/decision_lists.py > data/rules.tsv
fi
