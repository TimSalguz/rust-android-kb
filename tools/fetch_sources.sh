#!/usr/bin/env bash
# Download the dictionary sources into data/ (not in git), for build_lexicon.sh:
#   data/freq/{ru,en}_full.txt  — hermitdave/FrequencyWords 2018 (CC BY-SA 4.0)
#   data/scowl/scowl-*/final    — SCOWL 2020.12.07
#   data/opencorpora_forms.tsv  — every OpenCorpora word form (needs
#                                 `pip install pymorphy3 pymorphy3-dicts-ru`)
#   data/leipzig/*.tar.gz,      — sentences for the context model: Leipzig web
#   data/tatoeba/*.tsv.bz2        corpora (~420 MB) and Tatoeba (~40 MB); only
#                                 with LEIPZIG=1 (build_lexicon.sh builds it)
#   data/hunspell_forms.txt     — every form of the LibreOffice ru_RU Hunspell
#                                 dictionary (needs `unmunch`; $HUNSPELL_DIR
#                                 holds ru_RU.dic, default /usr/share/hunspell)
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p data/freq data/scowl
for lang in ru en; do
    [ -s data/freq/${lang}_full.txt ] || curl -fsSL -o data/freq/${lang}_full.txt \
        "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/$lang/${lang}_full.txt"
done
if ! ls -d data/scowl/scowl-*/final >/dev/null 2>&1; then
    curl -fsSL "https://downloads.sourceforge.net/project/wordlist/SCOWL/2020.12.07/scowl-2020.12.07.tar.gz" \
        | tar xz -C data/scowl
fi
[ -s data/opencorpora_forms.tsv ] || python3 tools/opencorpora_forms.py > data/opencorpora_forms.tsv
if [ ! -s data/hunspell_forms.txt ]; then
    python3 tools/hunspell_forms.py "${HUNSPELL_DIR:-/usr/share/hunspell}" "${UNMUNCH:-unmunch}" > data/hunspell_forms.txt
fi
if [ "${LEIPZIG:-0}" = 1 ]; then
    mkdir -p data/leipzig data/tatoeba
    for n in rus-ru_web-public_2019_1M eng-com_web-public_2018_1M; do
        [ -s data/leipzig/$n.tar.gz ] || curl -fsSL -o data/leipzig/$n.tar.gz \
            "https://downloads.wortschatz-leipzig.de/corpora/$n.tar.gz"
    done
    for l in rus eng; do
        [ -s data/tatoeba/${l}_sentences.tsv.bz2 ] || curl -fsSL -o data/tatoeba/${l}_sentences.tsv.bz2 \
            "https://downloads.tatoeba.org/exports/per_language/$l/${l}_sentences.tsv.bz2"
    done
fi
