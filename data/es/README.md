# data/es — Spanish

`tools/lang/es.sh` downloads the sources into `data/es/src/` (skipping what is
there) and builds everything in this directory; only this README,
`slang.tsv` and `RLA-ES-COPYRIGHT` are in git. Neutral Spanish for Spain and
Latin America: tú, usted, vosotros and vos forms alike.

| source | what | license |
|--------|------|---------|
| [hermitdave/FrequencyWords](https://github.com/hermitdave/FrequencyWords) 2018 `es_full` (OpenSubtitles) | word counts — the frequency prior; `en_full` only tells English words in Spanish subtitles apart | CC BY-SA 4.0 |
| [RLA-ES](https://github.com/sbosio/rla-es) v2.9 generic Spanish Hunspell dictionary ([`es.oxt`](https://github.com/sbosio/rla-es/releases/download/v2.9/es.oxt): `es.dic`, `es.aff`), Santiago Bosio and contributors | every word form: 68.5 k one-word entries → 690 k forms + 44 k with enclitic pronouns | GPLv3+ / LGPLv3+ / MPL 1.1+ at the user's choice — used under MPL 2.0 / LGPLv3+, see `RLA-ES-COPYRIGHT` |
| [Tatoeba](https://tatoeba.org) [`spa_sentences`](https://downloads.tatoeba.org/exports/per_language/spa/spa_sentences.tsv.bz2) (tatoeba.org contributors) | everyday sentences → word pairs, endings, capitals, slip rules; held-out coverage | CC BY 2.0 FR |
| [Leipzig Corpora Collection](https://wortschatz.uni-leipzig.de): [`spa-mx_web_2015_1M`](https://downloads.wortschatz-leipzig.de/corpora/spa-mx_web_2015_1M.tar.gz) — © Universität Leipzig / Sächsische Akademie der Wissenschaften / InfAI | web sentences (Mexico) → word pairs, endings, capitals, slip rules | CC BY |
| `slang.tsv` (this repo) | chat abbreviations and laughter (q, xq, tb, jaja, porfa, finde, tqm), internet words and loanwords (emoji, likear, influencer, streamear, tiktok, wasap, ok) | same as the code |

## Lexicon

`lexicon.tsv` — `word<TAB>count`, lowercase, NFC:

- Every RLA-ES form. The affixes are expanded by the script (Hunspell's
  `unmunch` misreads this dictionary's UTF-8 affix flags and invents forms
  like *esponjarlo*). A form seen in the subtitles keeps its count; an unseen
  one gets 2 % of its lemma's most frequent form, at least 1
  (`make_lexicon.py --forms`). Voseo (tenés, sabés, vení) and vosotros
  (tenéis, comed) forms are in.
- Verbs with enclitic pronouns (dámelo, decirte, comiéndoselo, vámonos) only
  where the corpora attest them: RLA-ES's own clitic forms seen twice in the
  subtitles, the Leipzig or the Tatoeba text; any other verb form + 1–3
  clitics seen 10 times, if accented by the rule (the stress stays on the
  verb's syllable: two syllables after it take an accent — dámelo,
  haciéndolo — one doesn't — dame, decime, comerlo; comed + os → comeos,
  vamos + nos → vámonos). Likewise diminutives, superlatives and -mente
  adverbs of dictionary words (cafecito, carísimo, agradecidamente).
- Subtitle words outside RLA-ES only when frequent (≥ 300: names, loanwords,
  slang — michael, cojones, mami), and not
  - a less frequent spelling of a word that differs in diacritics only
    (aqui, tambien, dia, fué, dió, tí, què): people leave accents out, the
    keyboard puts them back;
  - an English word (8× more frequent in the English subtitles: the, you);
  - a subtitle OCR slip, l read as I (ia, ias, ei);
  - one or two letters.
- Left out of RLA-ES: one-letter symbols (s, g, N), the old «ó», éso/ésto,
  and names spelled like a common word without its accent (Maria, Tio,
  Dificil).
- Subtitlers drop accents, so the plain word of an accent pair is
  overcounted there (mas for más, tenia for tenía): where Tatoeba has it over
  5× rarer, relative to the accented word, than the subtitles do, its count is
  scaled to Tatoeba's ratio (135 words: mas 103 445 → 5 572).
- Pre-2010 spellings that the subtitles use more than the new ones stay
  (sólo, éste, guión next to solo, este, guion): both are words.

## Capitals

`proper.tsv` → `casing.fst`: a word is Capitalized when, inside sentences
(not first, not after punctuation other than a comma) of the Tatoeba and
Leipzig text, ≥ 90 % of ≥ 5 occurrences are (≥ 97 % if RLA-ES also has it in
lowercase: not rosa, pilar, dios), in capitals when ≥ 80 % of ≥ 10 are (ONU,
DNI, UE, FBI, siglo XXI) unless RLA-ES has it in lowercase (unid, leed,
epa); rarer words follow RLA-ES (its names Capitalized, its abbreviations in
capitals). Month and day names, usted, señor, don stay lowercase; slang
(sip, omg, hey) is left alone. EE. UU. is not a word here; EEUU is.

## Context model

`build_bigrams.py` mixes the Tatoeba sentences (weight 0.6; held-out ids
% 50 and the Tom/Mary sentences skipped) with the Leipzig web corpus (0.4).
The Mexican web corpus: the generic `spa_web_2016_1M` has lost most of its
accents (tambin, informacin) and there is no Spain web corpus; no news. Its
sentences written without accents (a word like tambien, informacion, estan:
~4.5 %) are dropped — they would teach «esta bien», «mas» — so it is fed as
a Tatoeba-style file (`src/spa-mx_web_2015_1M.clean.tsv.bz2`, all ids odd:
none held out). Endings (`--endings-any-script`) carry gender and number
agreement (la casa blanca, los perros).

## Real-word slips

`docs/rules-format.md`: `confusions.tsv` — pairs of the top 80 000 words one
neighbouring key (qwertyuiop / asdfghjklñ / zxcvbnm), one doubled letter or
one diacritic apart: el/él, tu/tú, si/sí, que/qué, esta/está, como/cómo,
se/sé, mas/más, mi/mí, aun/aún, año/ano, papa/papá, hablo/habló;
`observations.tsv.bz2` — their contexts in Tatoeba and the Leipzig text (no
grammar classes); `rules.tsv` — `decision_lists.py` on the next word.

## Stats

| file | size |
|------|------|
| `lexicon.tsv` | 709 k forms: 673 k RLA-ES, 28.6 k with enclitics (23.5 k of RLA-ES's 43.5 k attested + 5.1 k from the corpora), 2 k diminutives / superlatives / -mente adverbs, 5.5 k frequent subtitle words, 150 slang |
| `dict.fst` | 2.5 MB |
| `proper.tsv` → `casing.fst` | 10 959 words (92 in capitals), 0.07 MB |
| `bigrams.tsv`, `endings.tsv` | 338 k word pairs for 41.7 k words, 41 k ending pairs |
| `confusions.tsv` | 15 205 pairs, 4 098 of them one diacritic apart |
| `rules.tsv` | 30 388 rules for 13 283 pairs |
| `bigrams.fst` (pairs + endings + rules) | 4.9 MB |

Held-out Tatoeba sentences (ids % 50 == 0, never trained on): 64 957 word
tokens, 99.2 % in the lexicon; of the 8 661 with á é í ó ú ü ñ, 98.8 %. The
misses are mostly names (Tatoeba, Ziri, Yahvé, Kioto), English words and
rare or misspelled words. The top of the list: de que no a la el y es en lo
un por qué me una los se te con para está mi pero sí si bien eso su las yo.

## Known issues

- Subtitles are mostly translated films: English names are frequent (jack,
  michael) and everyday Spanish words may be underweighted.
- RLA-ES gives enclitic forms to some verbs only; a valid but unattested one
  (fertilizarlas) is not in the lexicon.
- RLA-ES prefix derivations (re-, des-, anti-, super-, micro-…) are all in,
  seen or not.
- The web text is Mexican: voseo pairs (vos tenés) come from Tatoeba only.
- `decision_lists.py` looks at the next word only. The strongest cue for
  qué/cómo/dónde/cuándo/quién — «¿» right before (`p-1=¿`) — is in the
  observations but not in the rules; and a rare member of a pair (ano next
  to año: 59 observations) gets strong rules from 2–3 occurrences.

The derived lexicon: word forms from RLA-ES under MPL 2.0 / LGPLv3+ (see
`RLA-ES-COPYRIGHT`), counts from FrequencyWords under CC BY-SA 4.0.
