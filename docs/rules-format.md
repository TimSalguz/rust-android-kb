# Context rules: what the keyboard exports, what it expects back

The keyboard applies context rules; it never learns them. Rules are derived
offline from corpora — by `tools/decision_lists.py` (a plain baseline) or by any
outside rule learner (e.g. one that picks the shortest default-with-exceptions
law for each record) — and shipped as a table. At run time a rule
that fires adds evidence (log-odds, nats) to one reading of a word; the
noisy-channel score (typing/swipe geometry, word frequency, word pairs,
grammar classes) stays in charge.

## 1. Confusion sets — `data/confusions.tsv`

One pair per line, two lowercase words, tab-separated (a word may be in
several pairs: в/а, в/я, в/ы); the line number (from 0) is the set's id. Words any of which the user may have meant when one
of them came out:

- one slip on the phone keyboard: a letter replaced by a neighboring key
  (из/их, а/в, о/а, не/ее);
- the same gesture path: a doubled letter (ввод/вод, касса/каса);
- both frequent enough to matter (top 80 000 Russian and 40 000 English
  words by count).

Built by `tools/confusion_sets.py` from `data/lexicon.tsv` and the layout.

## 2. Observations — `data/observations.tsv.bz2`

One record per occurrence of a set's word in running text (Tatoeba and the
Leipzig web corpus, held-out Tatoeba sentences — ids divisible by 50 —
excluded):

```
set_id <TAB> truth <TAB> feature <TAB> feature …
```

`truth` is the word actually written (a member of the set). Features describe
its context, as far as the keyboard can see it when it decides:

| feature | meaning |
|---------|---------|
| `w-1=<word>` | previous word (lowercase); absent at sentence start |
| `w-2=<word>` | the word before that |
| `w+1=<word>` | next word (known only when the word is re-read after the next one) |
| `t-1=<tag>` | a grammatical tag of the previous word: one feature per tag of its likeliest readings — POS (`NOUN`, `ADJF`, `VERB`, `PREP`, …), gender (`masc`, `femn`, `neut`), number (`sing`, `plur`), case (`nomn`, `gent`, `datv`, `accs`, `ablt`, `loct`) |
| `t+1=<tag>` | the same for the next word |
| `c-1=<id>` / `c+1=<id>` | the grammar class of the previous / next word (`data/classes.tsv`: a class is a set of tag readings) |
| `bos` / `eos` | sentence start / end right before / after the word |
| `p-1=<char>` | punctuation right before the word (`,` `.` `?` …) |

Tags are OpenCorpora's (pymorphy3), coarse: POS, gender, number, case,
person, tense. A word unknown to OpenCorpora has no `t` features.

## 3. Rules — `data/rules.tsv`

```
set_id <TAB> feature <TAB> word <TAB> strength
```

- `feature` is one of the features above.
- `strength`: the evidence `feature` gives for `word` over the set's other
  member — the log likelihood ratio `ln P(feature | word) / P(feature |
  other)`, in nats, positive, at most 8. Not the posterior odds: the keyboard
  adds it to its own odds, which already weigh how frequent each word is and
  what the words before it say (word pairs, grammar classes) — so rules on
  the next word (`w+1`, `t+1`, `c+1`) and on punctuation or sentence ends
  bring the most; left-context rules only help where the language model is
  blind.
- Rules of a set are applied like a decision list: the matching rule with the
  largest strength decides (a more specific exception listed stronger than
  the general default wins). At most one rule fires per set and word.
- Keep a set's list short (≤ 20 rules); a rule should earn its place (fewer
  errors on held-out text, or a shorter description of the corpus).

Example:

```
17	t+1=gent	из	2.4
17	t+1=VERB	их	2.1
4	w+1=принципе	в	6.0
4	t+1=loct	в	3.2
```

The keyboard's baseline, `tools/decision_lists.py`, uses only `w+1` and
`c+1` (the grammar class id) and keeps rules seen ≥ 10 times with a ratio
≥ 1 nat, 20 per pair.

## 4. How the keyboard checks a rule table

`tools/eval_typing.py` (whole keyboard on held-out sentences: how many
correctly typed words it changes, how many sloppy taps it gets right) and
`tools/eval_context.py` (typo correction with context). A table is kept only
if it doesn't change more correct words and gets more wrong ones right.
