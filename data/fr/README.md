# data/fr — French

Built by `tools/lang/fr.sh` (French-specific steps in `tools/lang/fr_prep.py`,
the rest with the shared `tools/*.py`). The script downloads the sources into
`data/fr/src/` once and rebuilds everything else; only this README,
`slang.tsv` and `GRAMMALECTE-NOTICE` are in git.

| source | what | license |
|--------|------|---------|
| [Grammalecte](https://grammalecte.net) / Dicollecte, `lexique-grammalecte-fr-v7.7.zip` (Olivier R. and contributors) | every inflected form (~450 k) with lemma, part of speech, notes and corpus frequencies | MPL 2.0, see `GRAMMALECTE-NOTICE` |
| [hermitdave/FrequencyWords](https://github.com/hermitdave/FrequencyWords) 2018 `fr/fr_full.txt` (OpenSubtitles, 318 M tokens) | word counts — the frequency prior | CC BY-SA 4.0 |
| [Tatoeba](https://tatoeba.org) `fra_sentences.tsv.bz2` (tatoeba.org contributors) | everyday sentences → word pairs, endings, real-word rules | CC BY 2.0 FR |
| [Leipzig Corpora Collection](https://wortschatz.uni-leipzig.de) `fra-fr_web_2013_1M` — © Universität Leipzig / Sächsische Akademie der Wissenschaften / InfAI | web sentences → the same (the newest France web corpus there — 2011 and 2013; `fra-ch`/`fra-ca` are Swiss/Canadian, the rest news or Wikipedia) | CC BY |
| `slang.tsv` (this repo) | chat abbreviations, informal spellings, loanwords, brands, words newer than the subtitles (mdr, tkt, jsp, y'a, emoji, whatsapp, covid) | same as the code |

The derived word lists are distributed under the MPL 2.0 (the Grammalecte
word forms) and CC BY-SA 4.0 (the frequencies).

## Lexicon — `lexicon.tsv`

`word<TAB>count`, lowercase, NFC, 453 731 words:

- Every Grammalecte form (448 858) — except the spellings only the 1990 reform
  uses (`connaitre`, `ile`, `gout`, `chaine`: typed that way they are almost
  always a missing accent, and the keyboard should put it back; reform
  spellings shared with the traditional dictionary, like `évènement`, stay),
  the doubtful entries (sub-dictionary X: `peuton`, `dessication`), the
  `aimè-je` forms, error entries, symbols (`kVA`, `dacal`, the letter names
  `el`, `em`; common units `km kg cm ml mg ha kcal` stay), roman numerals,
  ordinals (`XIXe`), single letters other than `a à y ô`, two-letter codes that
  are not acronyms, and capitalized words that are an accentless spelling of a
  more frequent word (`CA` → it would make `ca` a word instead of `ça`;
  `Grace`/grâce, `Pres`/près, `Detroit`/détroit).
- Counts: a form seen in the subtitles keeps its count; an unseen one gets 2 %
  of its lemma's most frequent form (`make_lexicon.py --forms`; names and
  acronyms are their own lemma, so `ZVA` does not inherit the count of the
  verb `va`). Subtitles spelling `coeur`, `soeur` (44 598 and 32 041 — as often
  as `cœur`) count for the œ word (182 such merges).
- 4 751 hyphenated verb + pronoun forms that Grammalecte leaves to its
  tokenizer, seen ≥ 20 times in the subtitles: `est-ce` (707 256), `avez-vous`,
  `a-t-il`, `vas-y`, `dis-le-moi`, `allez-vous-en`, `ce jour-là`, `ex-femme`.
  Checked against the tags: the person must agree (`oses-tu`, not `ose-tu`),
  `-t-` only after a vowel (not `est-t-elle`), an imperative before
  `moi/le/lui/en/y` (not `amuses-toi`), `-là` after a noun (not `laissez-là`).
- Other subtitle words outside Grammalecte are **not** taken: even at ≥ 1000
  occurrences they are TV-series names (dinozzo, lorelai), OCR errors (lci,
  iui), accentless spellings (etre, tres, cest) and English. `slang.tsv` adds
  the real ones (142 lines), and lifts words newer than the 2018 subtitles
  (covid, télétravail, influenceur: 1–21 there).
- Every inflected form: `balbutierions`, `prud'homales`, `cœurs`, `réveille-toi`.
- The top: de je est pas le que la vous tu un c' à et il a l' ne les j' en on ça une d' ce qu' pour ai n' des.

Held-out Tatoeba sentences (ids divisible by 50, never trained on; clitics
split as below): **99.19 %** of 113 166 tokens are in the lexicon (99.34 %
counting `X-Y` as `X` + `Y`), **99.20 %** of the 14 653 tokens with an accent,
99.97 % of those with an apostrophe. Missing: mostly Kabyle names (Tatoeba has
many Kabyle contributors: ziri, yanni, tizi), reform spellings (plait,
connait, brulé — left out on purpose) and rare inversions (loge-t-il).

## Elision: the apostrophe

The elided clitics **c' ç' d' j' l' m' n' s' t' qu' jusqu' lorsqu' puisqu'
quoiqu'** are words of their own, written with the apostrophe, and the word
after them is a separate word: `l'homme` = `l'` + `homme`, `qu'est-ce` =
`qu'` + `est-ce`, `s'il` = `s'` + `il`. The subtitle counts are already split
this way (`c'` 4 184 576, `l'` 3 675 406); the Tatoeba and web sentences are
split before counting (`fr_prep.py tatoeba|leipzig`), so the pairs are
`l'` → école, eau, air, homme, histoire…; `j'` → ai, étais, aime; `qu'` → il,
elle, on.

Every other apostrophe is inside a word: `aujourd'hui`, `quelqu'un(e)`,
`presqu'île`, `prud'homme`, `entr'ouvert`, `chef-d'œuvre`, `p'tit`, `y'a`,
`va-t'en`. No lexicon word starts with a clitic + apostrophe — Grammalecte's
few such words lose the clitic (`c'est-à-dire` → `c'` + `est-à-dire`,
`n'importe` → `n'` + `importe`, `m'as-tu-vu` → `m'` + `as-tu-vu`) — so the rule
is one regular expression, the same everywhere:
`(?i)^(jusqu|lorsqu|puisqu|quoiqu|qu|[cçdjlmnst])'(?=letter)`.
’ (U+2019) and ʼ (U+02BC) are written `'`.

What the keyboard does:

- The apostrophe key types `'`. If the word being typed, lowercased, is one of
  the stems `c ç d j l m n s t qu jusqu lorsqu puisqu quoiqu`, the apostrophe
  ends it: commit `l'` as typed (no correction, no space) and start a new word
  right after it, with `l'` as the previous word (next-word guesses, context
  for correcting the next word). Otherwise the apostrophe is a letter of the
  word (`aujourd'` → aujourd'hui, `quelqu'` → quelqu'un).
- After a clitic, no space before a picked suggestion, and no automatic
  capital: `L'homme`, not `L'Homme` (a name keeps its own: `l'Europe`).
- Backspace right after `l'` should reopen `l'` as the word being typed.
- Pasted or typed ’ is read as `'`. (Whether the keyboard writes ' or ’ is a
  setting; the dictionary uses '.)
- Grammalecte marks the words that take no elision (`pel`: haut, héros, huit,
  onze, oui, hasard): `le héros`, not `l'héros` — not exported yet; the word
  pairs already prefer vowels and mute h after `l'`.

## Accents, œ, hyphens

Only proper spellings are in the lexicon: `être`, `ça`, `très`, `déjà`,
`cœur`, not `etre`, `ca`, `tres`, `deja`, `coeur`. The engine must make a
missing accent cheap (e → é è ê ë, a → à â, c → ç, u → ù û ü, i → î ï,
o → ô, y → ÿ) **and the ligatures**: `oe` → `œ`, `ae` → `æ` as one cheap
step, or `voeux` becomes `veux` (one deletion) instead of `vœux`. Words that differ only in an accent are both words and go to
the context rules: a/à, ou/où, la/là, du/dû, sur/sûr, des/dès, and the verbs
passe/passé, parle/parlé, arrive/arrivé.

Hyphenated forms are lexicon words when frequent; for rarer ones
(`promènera-t-il`) the engine should accept `X-Y` (and `X-t-Y`) when `X` and
`Y` are words rather than "correct" it.

## Capitals — `proper.tsv`

`word<TAB>1` (Capitalized) / `word<TAB>2` (ALL CAPS), 6 640 words: forms
Grammalecte writes only with a capital (names, places, brands: paris, londres,
macron, google, pâques, mme, dr) or only in capitals (sncf, tgv, onu, ue,
fbi, adn, ia), minus those the web corpus writes in lowercase in the middle
of a sentence at least a fifth of the time; plus words Grammalecte has both
ways that the web corpus capitalizes ≥ 90 % of the time mid-sentence (noël,
not internet, dieu). Months and days are lowercase in French and are not
there. Compound names (`Saint-Étienne`, `Aix-en-Provence`) and mixed case (`iPhone`,
`McDonald`) can't be written with one flag and are left out.

## Context model — `bigrams.tsv`, `endings.tsv`

`tools/build_bigrams.py` over the split sentences: Tatoeba (weight 0.6; its
Tom/Marie sentences skipped) and the Leipzig web corpus (0.4) minus spam
(43 094 locksmith, plumber, "devis gratuit"… sentences — 4.3 % of a
2013 web crawl) and repeated templates. The web corpus goes in as sentences,
not as Leipzig's own neighbour pairs, which keep `l'homme` whole. 358 279
pairs for 39 116 left words; endings with `--endings-any-script`: 47 299
pairs (je → -ai -is, not -ez; tu → -as -es -is; les → -es, not -te).

## Real-word slips — `confusions.tsv`, `observations.tsv.bz2`, `rules.tsv`

`tools/confusion_sets.py --rows azertyuiop qsdfghjklm "wxcvbn'"` (AZERTY; the
apostrophe key ends the bottom row): 13 962 pairs — neighbour keys (je/ne,
le/me, des/ses, c'/d', l'/n'), doubled letters, accent-only pairs (a/à,
ou/où, la/là, sur/sûr, passe/passé). The classic French pairs the tool can't
find (a letter left out, or homophones) are appended: et/est, on/ont,
son/sont, a/as, peu/peut, ses/ces, ce/se, sa/ça. Observations from the split
Tatoeba + web sentences (no grammar classes for French), rules by
`tools/decision_lists.py` on the next word.

## Sizes

| file | entries | FST |
|------|---------|-----|
| `lexicon.tsv` → `dict.fst` | 453 731 words | 2.0 MB |
| `proper.tsv` → `casing.fst` | 6 411 Capitalized + 229 ALL CAPS | 43 KB |
| `bigrams.tsv` + `endings.tsv` + `confusions.tsv` + `rules.tsv` → `bigrams.fst` | 358 279 pairs, 47 299 ending pairs, 13 970 confusion pairs, 35 056 rules (12 825 pairs) | 4.9 MB |
| `observations.tsv.bz2` | 12.5 M records | 62 MB (not shipped) |

## Known issues

- The subtitles are translated films and series: American first names are
  frequent (john, jack, michael), everyday French chat is underweighted.
- The web corpus is from 2013 (the newest France web corpus at Leipzig); the
  next-word guesses know little of the last decade (covid, visio are in the
  lexicon but have next to no pairs).
- Reform spellings (1990) that only drop a circumflex are left out on
  purpose; users of the reform spelling will see `connait` corrected to
  `connaît`.
- Hyphenated inversions of rarer verbs are missing (see above).
