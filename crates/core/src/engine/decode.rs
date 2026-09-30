//! One way from what the finger gave to the words it may mean at a place,
//! whatever the finger did: tapped a word, began one, drew one, or nothing
//! yet ([`Engine::decode`]).

use super::{Candidate, Context, Engine, Hint, PREDICT_POOL, PREDICT_RULED_OUT};

/// Lemmas the previous word's lemma predicts, and forms of each, that join
/// the words seen after it.
const PREDICT_LEMMAS: usize = 16;
const PREDICT_FORMS: usize = 12;

/// What the finger gave for a word.
#[derive(Clone, Copy, Debug)]
pub enum Evidence<'a> {
    /// Letters tapped, the word finished; each tap's [`Hint`] (where it
    /// landed, how long it was held…), or none.
    Taps(&'a str, &'a [Hint]),
    /// Letters tapped exactly, the word going on: the words they begin.
    Begun(&'a str),
    /// A way drawn across the keys: its points, their times (ms), each
    /// letter's key center and a key's width, in the points' units.
    Drawn {
        points: &'a [(f32, f32)],
        times: Option<&'a [f32]>,
        keys: &'a [(char, f32, f32)],
        key_w: f32,
    },
    /// Nothing: the word is only expected (after the word before it).
    Nothing,
}

impl<D: AsRef<[u8]>> Engine<D> {
    /// The words `evidence` may mean at the place `ctx` was read for, best
    /// first: what fits the evidence ([`Engine::read`]), weighed by the
    /// place ([`Engine::weigh`]) — the same way whatever the evidence.
    pub fn decode(&self, ctx: &Context, evidence: Evidence) -> Vec<Candidate> {
        // The searches weigh by the place all through: a word it makes likely
        // isn't cut off for being rare on its own.
        match evidence {
            Evidence::Taps(typed, hints) => self.suggest_at(ctx, typed, hints).0,
            Evidence::Begun(typed) => self.complete_at(ctx, typed),
            Evidence::Drawn {
                points,
                times,
                keys,
                key_w,
            } => self.gesture_at(ctx, points, times, keys, key_w),
            Evidence::Nothing => {
                // Words expected, none typed: the chooser weighs them as
                // such — if it learned to.
                let predicts = self.chooser.as_ref().is_some_and(|c| c.predicts());
                let ctx = ctx.predicting(predicts);
                let mut cands = self.read_at(&ctx, evidence);
                self.weigh(&ctx, &mut cands);
                cands
            }
        }
    }

    /// What fits the evidence alone, cheapest first — each candidate's
    /// `edit` what the evidence says against it, `cost` that plus its prior
    /// on its own.
    pub fn read(&self, evidence: Evidence) -> Vec<Candidate> {
        self.read_at(&Context::default(), evidence)
    }

    /// [`Engine::read`], where the place may bring in words of its own: the
    /// ones the word before leads to, when nothing is typed yet.
    fn read_at(&self, ctx: &Context, evidence: Evidence) -> Vec<Candidate> {
        match evidence {
            Evidence::Taps(typed, hints) => self.suggest_hinted(typed, hints).0,
            Evidence::Begun(typed) => self.complete(typed),
            Evidence::Drawn {
                points,
                times,
                keys,
                key_w,
            } => self.gesture_timed(points, times, keys, key_w),
            // The words the word before leads to, less those the phrase's
            // grammar rules out (a case the preposition doesn't govern, an
            // adjective's gender or number — «к большим деньгам», not
            // «успехом»), each at its prior on its own.
            Evidence::Nothing => {
                let Some(prev) = ctx.prev() else {
                    return Vec::new();
                };
                // The words seen after it, and the forms of the lemmas its
                // lemma leads to (seen with it or not).
                let mut words: Vec<String> = self
                    .predict(prev, self.cfg.top_k * PREDICT_POOL)
                    .into_iter()
                    .map(|c| c.word)
                    .chain(self.lemma_words(ctx, PREDICT_LEMMAS, PREDICT_FORMS))
                    .collect();
                let mut seen = std::collections::HashSet::new();
                words.retain(|w| seen.insert(w.clone()));
                words
                    .into_iter()
                    .filter(|w| self.grammar_allows(ctx, w))
                    .filter_map(|w| {
                        Some(Candidate {
                            cost: self.word_cost(&w)?,
                            edit: 0.0,
                            word: w,
                        })
                    })
                    .collect()
            }
        }
    }

    /// Whether the phrase's grammar leaves a guessed word possible.
    fn grammar_allows(&self, ctx: &Context, word: &str) -> bool {
        self.phrase_nats(ctx, &self.readings(word)).0 > PREDICT_RULED_OUT
    }
}
