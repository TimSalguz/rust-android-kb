# German (de) language pack data

`tools/lang/de.sh` downloads the sources into `data/de/src/` (not in git;
what is there is kept) and builds everything here; helpers are in
`tools/lang/de_data.py` (stdlib Python). The shared tools do the rest:
`make_lexicon.py`, `build_bigrams.py`, `confusion_sets.py`,
`export_observations.py`, `decision_lists.py`, `index-builder`.

| source | what | license |
|--------|------|---------|
| [LanguageTool german-pos-dict](https://github.com/languagetool-org/german-pos-dict) 1.2.4 ([Maven Central](https://repo1.maven.org/maven2/de/danielnaber/german-pos-dict/1.2.4/)) — Daniel Naber; data from Morphy (W. Lezius), extended by korrekturen.de | ~5 M tagged forms (4.99 M form/lemma/tag entries, 466 k distinct forms): every inflection of nouns, verbs, adjectives, participles; part of speech (SUB = noun, EIG = name) | CC BY-SA 4.0 |
| [Wiktionary](https://en.wiktionary.org) (English edition), German entries, via [kaikki.org](https://kaikki.org/dictionary/German/) (wiktextract) | 349 k entries with full declension/conjugation tables; many words LanguageTool lacks (Augenblick, Schlafzimmer, herein, tschüss, iwie); marks superseded, Swiss and obsolete spellings | CC BY-SA 4.0 + GFDL |
| [hermitdave/FrequencyWords](https://github.com/hermitdave/FrequencyWords) 2018 `de_full.txt` (OpenSubtitles) | 1.16 M word counts (156 M tokens) — the frequency prior; attests compounds | CC BY-SA 4.0 |
| [Tatoeba](https://tatoeba.org) `deu_sentences.tsv.bz2` | everyday sentences → word pairs, endings, capitalization, rules | CC BY 2.0 FR |
| [Leipzig Corpora Collection](https://wortschatz.uni-leipzig.de) `deu-de_web-public_2019_1M` — © Universität Leipzig / SAW / InfAI | web sentences → word pairs, capitalization, rules (no news corpus: it fills predictions with politics) | CC BY |
| SCOWL 2020.12.07 (`data/scowl`, see `data/SCOWL-COPYRIGHT`) | only to recognize English words (kept out) and names (kept) among the subtitle words | permissive |
| `slang.tsv` (this repo) | ~130 chat words: lol, vllt, lg, hdl, iwie, nen, digga, cringe, emoji, streamen, googeln… | same as the code |

Attributions: `NOTICE`. The derived word lists are CC BY-SA 4.0.

## How the pieces combine

**Lexicon** (`lexicon.tsv`, `word<TAB>count`, lowercase, NFC):

- every form of both morphological sources, minus spellings that are not
  current German: pre-1996 ß (daß, muß, läßt, Kuß), Swiss/Liechtenstein ss
  (Strasse, grösser), obsolete/regional table variants (vns, frey, net), and
  comparatives of participles nobody writes (niesendsten) unless attested;
- **compounds attested in the subtitles** (seen ≥ 3 times, 73.6 k; LanguageTool
  leaves compounds to a splitter, so it lacks even Schlafzimmer): a word
  the sources lack that splits into known words — modifier (a noun form,
  noun + Fugen-s, a stem: Schul-, Schlaf-, a particle: rein-, weg-, ab-,
  up to three parts) + a last word that is a noun, adjective or (after a
  particle) verb — Tabakplantage, Polizeiabsperrung, Geburtstagsparty,
  reinkommen. Parts of ≤ 3 letters must be common words (names like
  Drummond or Maddocks otherwise split too). Seen ≥ 5 times, a noun compound
  gets every form of its last noun (Tabakplantagen); ≥ 10 times, an adjective
  or particle-verb compound its plain declension or conjugation. No compound
  is generated that the corpus never wrote;
- corpus words the sources lack, only when frequent (≥ 300) and not English
  (SCOWL), not OCR junk (lch, lhr for Ich, Ihr), not a substitute spelling
  (fuer, schoen, strasse) and seen outside the subtitles too (Leipzig or
  Tatoeba) or an SCOWL name: mostly names (Danny, Chloe), a few colloquial
  forms (hätt, möcht, irgendwelchen);
- contractions with 's attested in the sentences (geht's, gibt's, hab's, mach's),
  counted as their share of the plain word (the subtitles split them);
- abbreviations the sources write only in capitals (FBI, OP, EU): their
  subtitle count is scaled by how often the web text writes them in capitals
  (it/IT, we/WE share one lowercase count); ones rarely written so (me, el,
  no) are dropped;
- `slang.tsv` — added as is (a word also in the dictionary keeps the larger
  count);
- **not** the lemmas none of whose forms occurs in any corpus (subtitles,
  web, Tatoeba): 258 k forms such as Zirconiumhalogeniden, aufgekrempeltere,
  Männinnen — a third of the forms for 0.07 % of the held-out text, and
  0.6 MB of FST.

Counts: a form seen in the subtitles keeps its count; an unseen one gets 2 %
of its lemma's most frequent form (`make_lexicon.py --forms`), where a lemma
is known by its part of speech and case, and a form spelled like a word of
another lemma doesn't set that maximum (hat for the verb "haten", einen for
the verb "einen", wollen for the adjective "wollen" = woolen) — otherwise
their unseen forms would get thousands. Single letters are kept at count 20.

**Capitals** (`proper.tsv` → `casing.fst`): all German nouns are
capitalized, so this lists every noun form and name — the forms whose
readings in the sources are all capitalized (LanguageTool SUB/EIG,
Wiktionary nouns and names, noun compounds) — unless the web/Tatoeba text
mostly writes them in lowercase in mid-sentence. A word with both a noun
and a lowercase reading (essen/Essen, leben/Leben, morgen/Morgen, arm/Arm,
recht/Recht, Sie/sie, nominalized infinitives and adjectives) is listed only
when ≥ 90 % of its mid-sentence occurrences (≥ 10) are capitalized — so
none of those. ALL CAPS (`2`): abbreviations (USA, EU, ADAC, ZDF, WLAN, PC,
SMS); chat abbreviations (lol, lg, hdl, kp, mfg) stay as typed. Corpus words
the sources lack: by the corpus (≥ 90 % capitalized) or, for SCOWL names,
capitalized.

**Context model**: `build_bigrams.py` over Tatoeba (0.6; every 50th sentence
held out) and the Leipzig web corpus (0.4), `--endings-any-script` (German
endings agree too: -en/-em/-er after articles).

**Real-word slips**: `confusion_sets.py --rows qwertzuiopü asdfghjklöä yxcvbnm`
(pairs one QWERTZ key apart, a doubled letter, or an umlaut apart:
schon/schön, fur/für is not a pair — "fur" is not a word);
`export_observations.py` without grammar classes (no German morphology
analyzer in the tool); `decision_lists.py` on the next word.

## Stats

`tools/lang/de.sh` from the downloaded sources: ~3 min, 1.5 GB peak (no step
needs the heavy-build lock).

| | |
|---|---|
| lexicon | 600 k forms: 295 k seen in the subtitles; 180 k in both sources, 98 k LanguageTool only, 168 k Wiktionary only; 73.5 k attested compounds + 80 k of their forms; 825 corpus words (mostly names), 97 contractions with 's, 368 abbreviations, ~130 slang; 142 k with ä/ö/ü/ß |
| left out | 258 k forms of unattested lemmas, 12.9 k superseded/Swiss spellings, 1.5 k old ß / Swiss ss forms |
| proper.tsv | 294 k Capitalized (nouns, names), 388 ALL CAPS |
| bigrams.tsv / endings.tsv | 315 k pairs for 38 k left words / 25 k ending pairs |
| confusions / rules | 10 040 pairs (top 80 k words) / 30 204 rules for 9 603 pairs |
| FSTs | dict ≈ 3.4 MB, casing ≈ 1.1 MB, bigrams (+endings, rules) ≈ 4.4 MB — measured with the letters mapped one-to-one onto ones the prebuilt `index-builder` knows; rebuild with the Latin-1 alphabet |
| held-out Tatoeba (ids ÷ 50, never trained on) | 98.94 % of tokens in the lexicon (99.15 % without the 263 pre-1996 spellings daß, muß…); tokens with ä/ö/ü/ß 95.56 % (98.32 %) |

Top of the list: ich sie das ist du nicht die es und der wir was zu er ein in
ja mir mit wie den mich auf dass aber eine so hat hier haben für sind war von
wenn dich ihr nein habe an. Missing held-out tokens besides old spellings:
Tatoeba's own names (Tatoeba, Yanni, Layla, Ziri), Japanese places, rare
compounds (Schokobären, Flughafenbusse).

The FST is bigger per word than the Russian one (~5.6 vs 2.3 MB per million):
long compound stems share little, and the per-word frequency values stop
suffixes from merging (the same list with equal values: 1.6 MB).

## Known issues

- Pre-1996 spellings (daß, muß, wieviel) are left out on purpose, though
  Tatoeba still has many (daß ×151 in the held-out part): typed, they get
  corrected like typos (daß → dass needs ß→ss, an edit the engine could make
  cheap).
- The subtitles are translated TV: the names kept are TV names (Danny,
  Sheldon), and modern chat words are underweighted (hence `slang.tsv`).
- Compounds: only those the subtitles attest; the long tail of new
  compounds (every Kaffeetasse-Henkel) is not in the lexicon — the engine
  would need compound splitting to accept them.
- Mixed-case words (E-Mail, U-Bahn, T-Shirt, GmbH, MfG, iPhone) are in the
  lexicon but not in `proper.tsv`: its two marks can't express them.
- Words with a noun and a lowercase reading (Essen/essen, Leben/leben,
  Morgen/morgen, Arm/arm) are left as typed: capitalizing them needs context
  (after an article or preposition: das Essen, am Morgen).
- Wiktionary is extracted weekly by kaikki.org: a rebuild later may differ
  slightly (the download is kept in `data/de/src/`).
