# data

This directory holds the Russian + English dictionary (the "base" pack). The
other languages have packs of their own, each with its sources and licenses:
[`de`](de/README.md), [`fr`](fr/README.md), [`es`](es/README.md),
[`pt`](pt/README.md) (built by `tools/lang/<code>.sh`).

The dictionary is built from open sources only: `tools/fetch_sources.sh`
downloads them (into `data/`, not in git), `tools/build_lexicon.sh` merges them
into `data/lexicon.tsv` (`word<TAB>count`, ~3.2 M words; CI does the same).

| source | what | license |
|--------|------|---------|
| [hermitdave/FrequencyWords](https://github.com/hermitdave/FrequencyWords) 2018 `ru_full`, `en_full` (OpenSubtitles) | word counts — the frequency prior | CC BY-SA 4.0 |
| [OpenCorpora](http://opencorpora.org) dictionary (via `pymorphy3-dicts-ru`, rev. 417150) | every Russian word form (~3 M): `балалаечного`, `проверяемыми` | CC BY-SA 3.0 |
| LibreOffice ru_RU Hunspell dictionary (A. Lebedev), expanded with `unmunch` | a second source of Russian forms (~1.4 M; +105 k words OpenCorpora lacks: балдёж, канальчики) | BSD-style, see `HUNSPELL-RU-COPYRIGHT` |
| [SCOWL](http://wordlist.aspell.net) 2020.12.07, levels ≤ 70 | English words, forms, contractions, names | permissive, see `SCOWL-COPYRIGHT` |
| [Tatoeba](https://tatoeba.org) sentence exports `rus_sentences`, `eng_sentences` (tatoeba.org contributors) | everyday sentences → word pairs for the context model (`bigrams.tsv`, `tools/build_bigrams.py`) | CC BY 2.0 FR |
| [Leipzig Corpora Collection](https://wortschatz.uni-leipzig.de): rus-ru_web-public_2019_1M, eng-com_web-public_2018_1M — © 2025 Universität Leipzig / Sächsische Akademie der Wissenschaften / InfAI | significant neighbour pairs → the context model | CC BY |
| `slang.tsv` (this repo) | chat slang, abbreviations, loanwords (го, тс, кринж, lol) | same as the code |

How the pieces combine:

- Russian: every OpenCorpora form, plus every Hunspell ru_RU form (count from
  the subtitles if seen, else 1), plus the е spelling of every word with ё
  (people type еще, актерская — that's a word, not a typo to fix). A form seen in the subtitles keeps its
  count; an unseen one gets 2 % of its lemma's most frequent form (at least 1),
  so forms of common words are plausible and forms of rare ones stay rare.
  Subtitle words outside OpenCorpora join only when frequent (≥ 300: names,
  slang, loanwords) — rarer ones are mostly typos — and never when they are
  two words run together by a common mistake: не with a verb (незнаю),
  вобщем, всмысле, кто-то without its hyphen.
- English: SCOWL up to level 70, with subtitle counts where seen and a floor by
  level otherwise; contractions get high floors (subtitle tokenization splits
  them at the apostrophe).
- `slang.tsv` — hand-curated; counts are moderate made-up frequencies. Add a
  line to stop autocorrect from "fixing" a word you use.
- `SCOWL-COPYRIGHT`, `HUNSPELL-RU-COPYRIGHT` — notices that must accompany
  the derived word lists.

Context model: `tools/build_bigrams.py` mixes everyday Tatoeba sentences
(weight 0.6; the ones about its stock characters Tom, Mary and Boston are
skipped) with the Leipzig web corpora (0.4). News corpora are left out: they
filled the next-word guesses with politics. A pair is kept where the previous
word changes a word's odds at least e-fold (count ≥ 3), up to 64 continuations
per word plus the 8 likeliest predecessors of each word (so «в принципе» is
there although «в» has thousands of continuations); a pair spelled with ё is
also kept spelled with е (people type «еще»), unless that is a pair of its own
(все ≠ всё): ~685 k pairs, 7.8 MB as an FST with the endings. `tools/eval_context.py`, typo correction top-1 without → with context and
next-word top-3, on held-out sentences:

| held out | correction | next word, top-3 |
|----------|-----------|------------------|
| Tatoeba (every 50th sentence, never trained on) | 86.4 % → 91.5 % | 19.7 % |
| Russian Wikipedia | 85.7 % → 89.3 % | 10.2 % |

(The previous news + web model: 88.4 % / 13.3 % and 88.2 % / 12.6 %.)

Endings: `build_bigrams.py --endings` also counts which ending follows which in
the Tatoeba sentences — the left side a short word itself (в, на, для govern
the case) or the last two letters of a longer one, the right side the last two
letters: ~49 k pairs stored in the same FST (keys under a leading 0). Where
the word pair itself is unknown, `P(word)` is scaled by `e^(PMI of the
endings)` (`w_endings` = 1.0, the best of 0 / 0.7 / 1.0 / 1.5 / 2.0), so forms
agree in case and number: «неведомых дорожкаж» → дорожках (not дорожка).
Correction with context: Tatoeba 91.5 → 92.3 %, Wikipedia 89.3 → 89.6 %.

Capitals: `tools/proper_nouns.py` lists the words always written with a
capital (`proper.tsv` → `casing.fst`, ~430 k forms): OpenCorpora forms whose
every reading is a name, surname, place, organisation or trademark (Москва,
Ивана — not вера, роза, орёл, which are also ordinary words), their
abbreviations in capitals (США, ФБР), and SCOWL's capitalized English words
that don't also exist in lowercase (London, Monday, I'm).

Known bias: subtitles are mostly translated films, so some everyday words are
underweighted (москва: 477). A broader frequency source would help.

The derived lexicon is distributed under CC BY-SA 4.0.
