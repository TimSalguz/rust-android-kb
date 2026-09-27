# data/pt — Brazilian Portuguese

Everything here except this file, `slang.tsv` and `VERO-COPYRIGHT` is
generated (not in git): `tools/lang/pt.sh` downloads the sources into
`data/pt/src/` (skipping what is there) and builds the rest (≈ 3 min; heavy
steps belong under the shared build lock). `FORCE=1` rebuilds every step.

| source | what | license |
|--------|------|---------|
| [hermitdave/FrequencyWords](https://github.com/hermitdave/FrequencyWords) 2018 `pt_br/pt_br_full.txt` (OpenSubtitles, 426 M tokens) | word counts — the frequency prior | CC BY-SA 4.0 |
| [VERO](https://github.com/LibreOffice/dictionaries/tree/master/pt_BR) 3.2, the LibreOffice pt_BR Hunspell dictionary (Raimundo Moura and team), `pt_BR.dic` + `pt_BR.aff`, expanded by `pt.sh` | every word form (Brazilian, post-1990 spelling): conjugations, plurals, enclitic forms | LGPLv3 / MPL (dual), see `VERO-COPYRIGHT` |
| [Tatoeba](https://tatoeba.org) `por_sentences.tsv.bz2` (tatoeba.org contributors; Brazilian and European mixed) | everyday sentences → word pairs; hyphenated-word counts; capitalization | CC BY 2.0 FR |
| [Leipzig Corpora Collection](https://wortschatz.uni-leipzig.de) `por-pt_web_2015_1M` — © Universität Leipzig / Sächsische Akademie der Wissenschaften / InfAI | web sentences and neighbour pairs → the context model; hyphenated-word counts; capitalization | CC BY |
| hermitdave 2018 `en/en_full.txt` | only to spot English left untranslated in the subtitles (nothing of it ships) | CC BY-SA 4.0 |
| `slang.tsv` (this repo) | chat abbreviations, interjections, loanwords (vc, pq, kkk, blz, printar, crush) | same as the code |

Leipzig has no Brazilian web corpus (only `por-br_newscrawl_2011`, news, and
European `por-pt_web_2014/2015`), so the web half of the context model is
European; Tatoeba is mixed. News corpora are left out (they fill the
next-word guesses with politics).

## How the pieces combine

- **Forms (VERO).** `pt.sh` expands the Hunspell dictionary itself (hunspell's
  `unmunch` misreads its UTF-8 flags: wrong prefixes, missed plurals) into
  2.8 M forms plus 7.6 M enclitic/mesoclitic ones, and keeps:
  - every form seen in the subtitles or the sentence corpora;
  - unseen forms of a lemma whose most frequent form was seen ≥ 30 times —
    conjugations, plurals, feminines, participles of common words. A derived
    word (diminutive, augmentative, -íssimo, -mente, -mento, a prefix: re-,
    des-) is a lemma of its own and needs ≥ 100, next to a base seen as often;
  - unseen *vós* forms (falásseis, falardes) are left out;
  - enclitic and mesoclitic forms (dá-me, fazê-lo, diga-me, trata-se,
    dir-se-ia) only when attested in Tatoeba/Leipzig; the hosts Hunspell
    lists for them as words (falá, fazê, falamo, falaremo) only when attested
    alone (dá, está, vê are words);
  - VERO's abbreviations only when the corpora write them in capitals half
    the time (TV, FBI, CPF) — else they are mostly typos and foreign words of
    the subtitles (nno, aime, elk) — or when in the short Brazilian list in
    `pt.sh` (INSS, FGTS, IBGE…); no Roman numerals of ≥ 3 letters; names and
    abbreviations that are English words of the subtitles (CAT, HIS, PIG) out.
- **European spellings out.** Of two spellings that differ by a silent c/p
  (facto/fato, contacto/contato, perspetiva/perspectiva), é/ê or ó/ô
  (bebé/bebê), registo/registro or connosco/conosco, the one ≥ 5× rarer in
  the (Brazilian) subtitles and ≥ 20× more at home in the (European) Leipzig
  web goes — 300 forms with the corpus words below: facto, excepto, secção,
  eléctrico, insecto, carácter, perspetiva, aspeto, registar, connosco…
- **Counts.** A form seen in the subtitles keeps its count; an unseen one gets
  0.5 % of its lemma's top form (`make_lexicon.py --forms --form-share
  0.005`; Portuguese verbs have ~70 forms, most rarely typed). A form that
  towers over its lemma (para ← parar, como ← comer) is split off first, so it
  doesn't lift the lemma's unseen forms. Hyphenated words (the subtitles split
  them) get Tatoeba + 0.3 × Leipzig counts scaled to the subtitles; clitic
  forms at most 2 % of their verb's top form.
- **Corpus words outside VERO** join at ≥ 300 (names, brands, loanwords:
  michelle, facebook, ok, hmm) when attested in the sentence corpora too, and
  not: a spelling without accents or before 1990 of a common word (nao, voce,
  tambem, idéia, vôo, pára, freqüente), a European spelling (óptimo, acção,
  connosco), an OCR slip (ihe → lhe), a subtitle credit (InSubs, resync), or an
  English word left untranslated (the, you — ≥ 4× more frequent in the
  English subtitles).
- **Slang** (`slang.tsv`) as is; a line with a model verb gets that verb's
  conjugation (`printar … falar` → printei, printou, printando).
- **Capitals** (`proper.tsv`, key → 1 Capitalized / 2 ALL CAPS), decided by
  VERO and by how Tatoeba + Leipzig write the word in mid-sentence: VERO's
  capitalized entries that no lowercase entry has (Brasil, Coimbra — not
  rosa, flor, graça, vitória), unless written in lowercase half the time; a
  word VERO also has in lowercase only when capitalized ≥ 95 % of the time
  (Deus), ≥ 98.5 % for words VERO has only in lowercase (VERO lacks many
  first names and has joão, paulo, pedro as rare common words or verb forms);
  corpus names outside VERO at ≥ 80 % (Jack, Michelle). ALL CAPS when written
  so ≥ 80 % of the time and the subtitles don't use the word much more than
  the web does (FBI, ONU, DNA — not doc, toc, jen); plus the Brazilian lists
  in `pt.sh` (CPF, INSS, ENEM; Detran, Anvisa). Months and weekdays stay
  lowercase; `slang.tsv` words stay as typed (eba, pet). Hyphenated names
  (Guiné-Bissau, Timor-Leste) and words of ≤ 2 letters (SP, RJ, TV) are left
  out. Pix stays lowercase (people write "fazer um pix"; the brand is "Pix").
- **Context model** — `tools/build_bigrams.py` over Tatoeba (0.6) and the
  Leipzig web (0.4), `--endings-any-script` (endings agree: -os after -os).
- **Real-word slips** — `tools/confusion_sets.py --rows qwertyuiop
  asdfghjklç zxcvbnm`: neighbor keys, a doubled letter, one accent left out
  (e/é, esta/está, pais/país, nos/nós, so/só, da/dá, mas/más, avo/avó,
  tem/têm, a/à); `pt.sh` appends the pairs that differ in more than one
  accent, which the tool doesn't make (avó/avô, vovó/vovô, mantém/mantêm,
  pôs/pós, maçã/maca). Then `export_observations.py` (no grammar classes) and
  `decision_lists.py`.

## Stats

| | |
|-|-|
| `lexicon.tsv` | 411 747 forms: 211 895 seen in the subtitles, 46 976 only in Tatoeba/Leipzig, 152 876 unseen forms of common lemmas; 24 % with an accent or ç; 28 173 hyphenated (22 695 enclitic/mesoclitic, 5 353 compounds), 36 with an apostrophe (d'água) |
| `proper.tsv` | 7 375: 7 061 Capitalized, 314 ALL CAPS |
| `bigrams.tsv` / `endings.tsv` | 335 751 pairs / 19 877 ending pairs |
| `confusions.tsv` | 13 995 pairs (1 454 differing only in accents) |
| `rules.tsv` | 16 913 rules for 12 207 pairs |
| `dict.fst` | 2.4 MB |
| `casing.fst` | 43 KB |
| `bigrams.fst` (pairs, endings, rules) | 4.5 MB |

Held-out Tatoeba sentences (ids divisible by 50, never used above): the
lexicon has 99.33 % of their 67 399 tokens, 99.27 % of the 11 110 with an
accent or ç. What it misses is mostly European spelling (quilómetros,
polónia, económico, facto, secção, contacto, prémio, ténis) and rare names.

Top of the list: que não o de a é você e eu um para está uma se com por ele
isso em do me mas como bem da no os ela na sim aqui mais tem meu seu muito…

## What the keyboard has to handle

- Accents: a quarter of the forms have one; people leave them out (nao,
  voce, entao, tambem), so a missing accent must be cheap — and many pairs are
  words both ways (e/é, esta/está, nos/nós, so/só, da/dá, pais/país, tem/têm,
  a/à, avó/avô): context decides (`rules.tsv`). c for ç too (cabeca → cabeça),
  although ç has its own key.
- The hyphen is part of a word: clitic forms (dá-me, fazê-lo, diga-me,
  trata-se) and compounds (segunda-feira, guarda-chuva, bem-vindo). The
  subtitles split them, so their counts are estimates. `fazelo` → fazê-lo
  already works as a correction.
- Casing is per word: a hyphenated name (Guiné-Bissau) would need each part
  capitalized, so such names are left out of `proper.tsv`.
- Ordinals (1º, 2ª, nº) use º/ª, which are not in the alphabet.
- Chat forms are words: pra, pro, tá, tô, cê, né, vc, pq, kkk.

## Known issues

- The frequency prior is film subtitles (translated, mostly American):
  character names are frequent (jack, sam), everyday Brazilian words somewhat
  underweighted, pre-1990 spellings frequent (idéia 91 k vs ideia 115 k — they
  are left out; the engine should turn them into the new spelling).
- The web half of the context model is European Portuguese (estou a fazer,
  diz-me, pequeno-almoço).
- Many attested clitic forms (22 k) come from the European web; Brazilians
  mostly write the pronoun before the verb (me diz).
- Some European-spelling decisions are wrong for rare words (capte → cate,
  lactentes, repto), and VERO keeps a few European words that are no
  spelling variants (ecrã, telemóvel).
- Coverage misses are mostly European spellings (quilómetros, económico,
  prémio) and rare names — by design.

The derived lexicon is distributed under CC BY-SA 4.0 (counts) together with
the terms of VERO (LGPLv3 / MPL) for its word forms; see `VERO-COPYRIGHT`.
