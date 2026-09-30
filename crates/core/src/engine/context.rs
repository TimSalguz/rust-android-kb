//! What the text before a word says about it — read once for a place, and
//! the one way any word there is weighed, however it came: typed, completed,
//! drawn or predicted.

use fst::raw::CompiledAddr;
use fst::{IntoStreamer, Streamer};

use super::{ending_prefix, remap, Candidate, Engine, BIGRAM_SEP, UNSEEN_CLASS_PAIR};
use crate::gram::{self, AttrWeights, Frame, Walk};

/// The most the classes or the endings of two words move the word's own
/// odds ([`Engine::weigh`]): PMI clamped to this, nats.
const FIT_CLAMP: f32 = 4.0;
/// …and the sense classes, their mean PMI.
const TOPIC_CLAMP: f32 = 1.5;
/// Words looked through under a lemma's stem for its forms.
const LEMMA_SCAN: usize = 256;

/// What the text before the cursor says about the next word
/// ([`Engine::context`]): the word right before it (the word pairs), the
/// phrase back to the word governing it (agreement, government, the
/// subject), what the sentence is about (sense classes). [`Engine::weigh`]
/// weighs any word by all of it.
#[derive(Clone, Debug, Default)]
pub struct Context {
    prev: Option<String>,
    /// The previous word's frame (the case, or the infinitive, it takes)
    /// and its class's tags: the word pairs' grammar.
    prev_frame: Option<Frame>,
    prev_tags: Option<Vec<u8>>,
    /// The previous word's row in the lemma vectors.
    prev_lemma: Option<u32>,
    /// Where the previous word's pairs start in the context model: the node
    /// after `previous\0` and the outputs on the way there — a search goes
    /// down the pairs along with the dictionary.
    pub(crate) pairs: Option<(CompiledAddr, u64)>,
    /// The most the word pairs' grammar may raise a word's own odds (nats,
    /// weighted), and the most the phrase and the sentence may take off its
    /// cost: what a search allows for before it knows the word.
    pub(super) fit_max: f32,
    pub(super) bonus_max: f32,
    /// The phrase's words with their readings.
    phrase: Vec<(String, Vec<u64>)>,
    walk: Walk,
    attr: Option<AttrWeights>,
    /// The governing word's frame, across the adjectives between.
    across: Option<Frame>,
    /// The previous word's frame when it governs: what comes right after it.
    next: Option<Frame>,
    /// The clause's verb's frame after a noun phrase («дал книгу [другу]»).
    clause: Option<Frame>,
    /// The sentence's sense classes, once each.
    topics: Vec<u64>,
    /// What the chooser read of the sentence, and the most its f may take
    /// off a word's cost (weighted, over τ).
    chooser: Option<(std::sync::Arc<crate::chooser::Reading>, f32)>,
}

impl Context {
    /// The word right before, lowercase (None at the start or after
    /// punctuation).
    pub fn prev(&self) -> Option<&str> {
        self.prev.as_deref()
    }

    /// The place without the chooser's reading: for what it wasn't taught
    /// (the words expected before any is typed).
    pub(crate) fn without_chooser(&self) -> Context {
        let mut c = self.clone();
        c.bonus_max -= c.chooser.take().map_or(0.0, |x| x.1);
        c
    }

    /// Whether the place says anything (with a context model to say it).
    pub(crate) fn speaks(&self) -> bool {
        self.prev.is_some() || !self.phrase.is_empty() || !self.topics.is_empty()
    }
}

impl<D: AsRef<[u8]>> Engine<D> {
    /// Read a place: `prev`, the word right before it (None across
    /// punctuation); `phrase`, the words back to the punctuation before
    /// (at most five, in order); `sentence`, the words of the sentence so
    /// far — lowercase.
    pub fn context(&self, prev: Option<&str>, phrase: &[String], sentence: &[String]) -> Context {
        if self.bigrams.is_none() {
            return Context {
                prev: prev.map(str::to_lowercase),
                ..Context::default()
            };
        }
        let phrase: Vec<(String, Vec<u64>)> = phrase
            .iter()
            .map(|w| (w.to_lowercase(), self.readings(&w.to_lowercase())))
            .collect();
        let mut walk = gram::walk(&phrase);
        walk.attribute_disagree = walk
            .nearest_attribute
            .and_then(|i| self.attribute_weights(&phrase[i].0))
            .map(|(_, disagree)| disagree);
        // A preposition's frame by what it hangs on says more than its
        // own: the word right before it, when the two are common together
        // («что за [головная боль]», «благодарю за [помощь]»), else the
        // sense class of the verb before it («бежала за [велосипедом]»,
        // «заплатила за [велосипед]»).
        let through = walk.governor.and_then(|g| {
            let r = &phrase[g].1;
            if r.is_empty() || !r.iter().all(|&x| x & gram::bit::PREP != 0) {
                return None;
            }
            let prep = &phrase[g].0;
            let paired = g
                .checked_sub(1)
                .and_then(|b| self.paired_frame(&phrase[b].0, prep));
            paired
                .or_else(|| {
                    let v = gram::verb_before(&phrase, g)?;
                    self.through_frame(self.topic(&phrase[v].0)?, prep)
                })
                .map(|f| (g, f))
        });
        let frame_of = |g: usize| match through {
            Some((t, f)) if t == g => Some(f),
            _ => self.frame(&phrase[g].0),
        };
        let across = walk
            .governor
            .filter(|&g| g + 1 < phrase.len() && gram::governing(&phrase[g].1))
            .and_then(frame_of);
        let last = phrase.len().checked_sub(1);
        let next = last
            .filter(|&l| gram::governing(&phrase[l].1))
            .and_then(frame_of);
        let clause = walk.clause.and_then(|v| self.clause_frame(&phrase[v].0));
        let mut topics: Vec<u64> = sentence
            .iter()
            .filter_map(|w| self.topic(&w.to_lowercase()))
            .collect();
        topics.sort_unstable();
        topics.dedup();
        let prev = prev.map(str::to_lowercase);
        let attr = self.attr_weights();
        let cfg = &self.cfg;
        let prev_frame = match (through, last) {
            (Some((g, f)), Some(l)) if g == l => Some(f),
            _ => prev.as_deref().and_then(|p| self.frame(p)),
        };
        let prev_tags = prev
            .as_deref()
            .and_then(|p| self.word_class(p))
            .map(|c| self.class_tags(c));
        let prev_lemma = prev.as_deref().and_then(|p| self.lemma(p));
        let fit_max = prev.as_deref().map_or(0.0, |p| {
            self.fit_max_of(p, prev_frame.as_ref(), prev_tags.as_deref())
                + self.lemma_max(prev_lemma)
        });
        let phrase_max = if phrase.is_empty() {
            0.0
        } else {
            match &attr {
                Some(a) => {
                    let w = if walk.subject.is_some() {
                        cfg.w_subject
                    } else {
                        cfg.w_phrase
                    };
                    w.max(0.0) * gram::phrase_score_max(&walk, across.as_ref(), clause.as_ref(), a)
                }
                None => 0.0,
            }
        };
        let topic_max = if topics.is_empty() {
            0.0
        } else {
            cfg.w_topic.max(0.0) * TOPIC_CLAMP
        };
        let chooser = self.read_sentence(sentence);
        let chooser_max = chooser.as_ref().map_or(0.0, |c| c.1);
        Context {
            prev_frame,
            prev_tags,
            prev_lemma,
            pairs: prev.as_deref().and_then(|p| self.pairs_of(p)),
            fit_max,
            bonus_max: phrase_max + topic_max + chooser_max,
            prev,
            phrase,
            walk,
            attr,
            across,
            next,
            clause,
            topics,
            chooser,
        }
    }

    /// What each word of the phrase is to the word coming, by the grammar
    /// (in order; the phrase is the sentence's tail): 1 the word governing
    /// it, 2 the verb of its clause, 3 the adjective nearest it, 4 another
    /// word of the phrase — the chooser's prior on what the word links to.
    pub fn roles(&self, ctx: &Context) -> Vec<u8> {
        let mut roles = vec![4u8; ctx.phrase.len()];
        let w = &ctx.walk;
        for (i, role) in [(w.nearest_attribute, 3), (w.clause, 2), (w.governor, 1)] {
            if let Some(r) = i.and_then(|i| roles.get_mut(i)) {
                *r = role;
            }
        }
        roles
    }

    /// The chooser's reading of the sentence so far (`sentence`, its words
    /// before the place), with the most its f may take off a cost.
    fn read_sentence(
        &self,
        sentence: &[String],
    ) -> Option<(std::sync::Arc<crate::chooser::Reading>, f32)> {
        let (chooser, lemmas) = (self.chooser.as_ref()?, self.lemmas.as_ref()?);
        let w = self.cfg.w_chooser;
        if w <= 0.0 {
            return None;
        }
        let words: Vec<(u32, u32)> = sentence
            .iter()
            .map(|word| {
                let word = word.to_lowercase();
                let l = self.lemma(&word).unwrap_or(lemmas.unk());
                (l, self.word_class(&word).unwrap_or(0) as u32)
            })
            .collect();
        let r = chooser.reading(lemmas, &words);
        let clamp = self.cfg.chooser_clamp.max(0.0);
        let most = (w * r.most.max(0.0) / chooser.tau()).min(clamp) * self.cfg.chooser_bound;
        Some((r, most))
    }

    /// What the chooser takes off a candidate's cost at the place (nats,
    /// weighted; negative: it adds): its f over τ.
    fn chooser_fit(&self, ctx: &Context, c: &Candidate, class: u64) -> f32 {
        let (Some((r, _)), Some(chooser), Some(lemmas)) =
            (&ctx.chooser, &self.chooser, &self.lemmas)
        else {
            return 0.0;
        };
        let unk = lemmas.unk();
        let l = self.lemma(&c.word);
        let prior = (c.cost - c.edit) / self.cfg.w_lm.max(1e-6);
        let feats = [
            c.edit.min(crate::chooser::EDIT_CAP),
            prior / 10.0,
            if l.is_some_and(|x| x != unk) {
                0.0
            } else {
                1.0
            },
        ];
        let f = chooser.score(lemmas, r, l.unwrap_or(unk), class as u32, &feats);
        let clamp = self.cfg.chooser_clamp.max(0.0);
        (self.cfg.w_chooser * f / chooser.tau()).clamp(-clamp, clamp)
    }

    /// The most the word pairs' grammar may raise any word's own odds after
    /// `prev` (nats, weighted): the most its frame, its class's tags or its
    /// ending say for any word ([`Engine::weigh`]).
    fn fit_max_of(&self, prev: &str, frame: Option<&Frame>, tags: Option<&[u8]>) -> f32 {
        let cfg = &self.cfg;
        let Some(bigrams) = self.bigrams.as_ref() else {
            return 0.0;
        };
        let pmi = |v: u64| v as f32 * cfg.prior_scale - 8.0;
        let most = |lo: Vec<u8>, hi: Vec<u8>| {
            let mut stream = bigrams.range().ge(lo).lt(hi).into_stream();
            let mut best: Option<f32> = None;
            while let Some((_, v)) = stream.next() {
                best = Some(best.map_or(pmi(v), |b: f32| b.max(pmi(v))));
            }
            best
        };
        let mut classes = frame.map(|f| {
            f.cases
                .iter()
                .copied()
                .chain([f.other, f.infn])
                .fold(f32::MIN, f32::max)
        });
        if let Some(ta) = tags {
            let mut m = UNSEEN_CLASS_PAIR;
            for &x in ta {
                let hi = match x.checked_add(1) {
                    Some(y) => vec![BIGRAM_SEP, 4, y],
                    None => vec![BIGRAM_SEP, 5],
                };
                if let Some(v) = most(vec![BIGRAM_SEP, 4, x], hi) {
                    m = m.max(v);
                }
            }
            classes = Some(classes.map_or(m, |c| c.max(m)));
        }
        let class_part =
            classes.map_or(f32::MIN, |c| cfg.w_classes * c.clamp(-FIT_CLAMP, FIT_CLAMP));
        // An ending pair not kept counts 0.
        let ending = ending_prefix(prev).and_then(|lo| {
            let mut hi = lo.clone();
            *hi.last_mut()? += 1;
            most(lo, hi)
        });
        let ending_part = cfg.w_endings * ending.unwrap_or(0.0).clamp(0.0, FIT_CLAMP);
        class_part.max(ending_part)
    }

    /// The most the lemma vectors may raise any word's own odds after a
    /// word of lemma `c` (nats, weighted).
    fn lemma_max(&self, c: Option<u32>) -> f32 {
        match (&self.lemmas, c) {
            (Some(l), Some(c)) => self.cfg.w_lemma.max(0.0) * l.most(c).clamp(0.0, FIT_CLAMP),
            _ => 0.0,
        }
    }

    /// How much likelier `word`'s lemma comes after the previous word's
    /// than on its own (nats, weighted; 0 without vectors for either).
    fn lemma_fit(&self, ctx: &Context, word: &str) -> f32 {
        let (Some(lemmas), Some(c)) = (&self.lemmas, ctx.prev_lemma) else {
            return 0.0;
        };
        if self.cfg.w_lemma <= 0.0 {
            return 0.0;
        }
        self.lemma(word).map_or(0.0, |l| {
            self.cfg.w_lemma * lemmas.pmi(c, l).clamp(-FIT_CLAMP, FIT_CLAMP)
        })
    }

    /// Words the lemma vectors expect after the previous word: the forms of
    /// the `lemmas` lemmas likeliest after its lemma, each lemma's `forms`
    /// commonest — the grammar picks among them ([`Engine::weigh`]).
    pub(super) fn lemma_words(&self, ctx: &Context, lemmas: usize, forms: usize) -> Vec<String> {
        let (Some(vectors), Some(bigrams), Some(c)) = (&self.lemmas, &self.bigrams, ctx.prev_lemma)
        else {
            return Vec::new();
        };
        if self.cfg.w_lemma <= 0.0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (l, _) in vectors.top(c, lemmas) {
            let lo = [&[BIGRAM_SEP, 25][..], vectors.stem(l)].concat();
            let mut hi = lo.clone();
            if let Some(last) = hi.last_mut() {
                *last += 1;
            }
            let mut of_lemma: Vec<(f32, String)> = Vec::new();
            // A short stem («в», «бы») lies over many words: its own forms
            // come first, the rest is not looked through.
            let mut stream = bigrams.range().ge(&lo).lt(&hi).into_stream();
            let mut seen = 0;
            while let Some((key, id)) = stream.next() {
                seen += 1;
                if seen > LEMMA_SCAN {
                    break;
                }
                if id as u32 != l {
                    continue;
                }
                let word = crate::alphabet::ids_to_string(&key[2..]);
                if let Some(cost) = self.word_cost(&word) {
                    of_lemma.push((cost, word));
                }
            }
            of_lemma.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.extend(of_lemma.into_iter().take(forms).map(|(_, w)| w));
        }
        out
    }

    /// Where `previous`'s word pairs start in the context model.
    fn pairs_of(&self, previous: &str) -> Option<(CompiledAddr, u64)> {
        let fst = self.bigrams.as_ref()?.as_fst();
        let mut key = remap(previous)?;
        if key.is_empty() {
            return None;
        }
        key.push(BIGRAM_SEP);
        let mut node = fst.root();
        let mut acc = 0u64;
        for b in key {
            let t = node.transition(node.find_input(b)?);
            acc += t.out.value();
            node = fst.node(t.addr);
        }
        Some((node.addr(), acc))
    }

    /// Weigh words at the place `ctx` was read for, best first: each one's
    /// prior becomes the context's — the word pairs mixed with the word on
    /// its own, the phrase's grammar, the sentence's sense. A candidate's
    /// `cost − edit` must be its plain prior (`w_lm·(−log P(word))`).
    pub fn weigh(&self, ctx: &Context, cands: &mut [Candidate]) {
        if self.bigrams.is_none() {
            return;
        }
        for c in cands.iter_mut() {
            c.cost += self.shift(ctx, c);
        }
        cands.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
    }

    /// How much the place moves a candidate's cost.
    pub(crate) fn shift(&self, ctx: &Context, c: &Candidate) -> f32 {
        let mut cost = c.cost;
        // The word's grammar once: its readings for the phrase, its class for
        // the chooser.
        let grammar = if ctx.prev.is_some() || !ctx.phrase.is_empty() || ctx.chooser.is_some() {
            self.word_grammar(&c.word)
        } else {
            None
        };
        let readings = if ctx.prev.is_some() || !ctx.phrase.is_empty() {
            self.set_readings(grammar.map_or(0, |g| g.1))
        } else {
            Vec::new()
        };
        if let Some(prev) = ctx.prev.as_deref().filter(|_| self.cfg.w_lm > 0.0) {
            let unigram = (c.cost - c.edit) / self.cfg.w_lm;
            let pair = self.bigram_nats(prev, &c.word);
            cost = c.edit
                + self.cfg.w_lm * self.pair_nats(ctx, prev, &c.word, &readings, unigram, pair);
        }
        if !ctx.phrase.is_empty() {
            let (s, w) = self.phrase_nats(ctx, &readings);
            cost -= w * s;
        }
        cost -= self.chooser_fit(ctx, c, grammar.map_or(0, |g| g.0));
        if !ctx.topics.is_empty() && self.cfg.w_topic > 0.0 {
            if let Some(t) = self.topic(&c.word) {
                let s = ctx
                    .topics
                    .iter()
                    .map(|&x| self.topic_pmi(t, x))
                    .sum::<f32>()
                    / ctx.topics.len() as f32;
                cost -= self.cfg.w_topic * s.clamp(-TOPIC_CLAMP, TOPIC_CLAMP);
            }
        }
        cost - c.cost
    }

    /// The prior part of a cost, `−log P` in nats, after the previous word:
    /// `λ·P(w | prev) + (1 − λ)·P'(w)` — `pair`, `−log P(w | prev)` if the
    /// pair is in the context model; `P'`, the word's own odds (`unigram`
    /// nats) moved by how well the two go together: their forms — the
    /// previous word's frame or the two classes where both are classed, the
    /// endings otherwise — and their lemmas (the lemma vectors).
    fn pair_nats(
        &self,
        ctx: &Context,
        prev: &str,
        word: &str,
        readings: &[u64],
        unigram: f32,
        pair: Option<f32>,
    ) -> f32 {
        let cfg = &self.cfg;
        let l = cfg.ctx_lambda.clamp(0.0, 0.99);
        let frame = ctx
            .prev_frame
            .filter(|_| cfg.w_classes > 0.0)
            .map(|f| f.next(readings))
            .filter(|&v| v != 0.0);
        let classes = || {
            let ta = ctx.prev_tags.as_ref()?;
            self.class_pmi_tags(ta, word)
                .filter(|_| cfg.w_classes > 0.0)
        };
        let fit = match frame.or_else(classes) {
            Some(pmi) => cfg.w_classes * pmi.clamp(-FIT_CLAMP, FIT_CLAMP),
            None => cfg.w_endings * self.ending_pmi(prev, word).clamp(-FIT_CLAMP, FIT_CLAMP),
        };
        let p_uni = (fit + self.lemma_fit(ctx, word) - unigram).exp();
        let p_ctx = pair.map_or(0.0, |b| (-b).exp());
        -(l * p_ctx + (1.0 - l) * p_uni).max(f32::MIN_POSITIVE).ln()
    }

    /// The least a word can cost at the place, before the search knows it:
    /// `u` a lower bound of its own prior (nats), `c` of its pair's with the
    /// previous word (None: no pair below). As `w_lm·nats`, less the most the
    /// phrase and the sentence may give.
    pub(crate) fn prior_floor(&self, ctx: &Context, u: f32, c: Option<f32>) -> f32 {
        let cfg = &self.cfg;
        let nats = if ctx.prev.is_some() && cfg.w_lm > 0.0 {
            let l = cfg.ctx_lambda.clamp(0.0, 0.99);
            let p = l * c.map_or(0.0, |c| (-c).exp()) + (1.0 - l) * (ctx.fit_max - u).exp();
            -p.max(f32::MIN_POSITIVE).ln()
        } else {
            u
        };
        cfg.w_lm * nats - ctx.bonus_max
    }

    /// How the phrase weighs a word (its readings), in nats, and the weight
    /// that goes with it: with the learned frames and agreement weights
    /// ([`gram::phrase_score`], `w_phrase`), or — an older model without
    /// them — only what the grammar rules out ([`gram::misfit`], -1). A
    /// predicate against its subject weighs `w_subject`: the words between
    /// («девочка быстро побежала») hide the subject from the word pairs, and
    /// its weights compare with chance, not with what the pairs know.
    pub(super) fn phrase_nats(&self, ctx: &Context, word: &[u64]) -> (f32, f32) {
        if ctx.phrase.is_empty() || word.is_empty() {
            return (0.0, 0.0);
        }
        let Some(attr) = &ctx.attr else {
            let s = if gram::misfit(&ctx.phrase, word) {
                -1.0
            } else {
                0.0
            };
            return (s, self.cfg.w_phrase);
        };
        let w = if ctx.walk.subject.is_some() {
            self.cfg.w_subject
        } else {
            self.cfg.w_phrase
        };
        (
            gram::phrase_score(
                &ctx.walk,
                ctx.across.as_ref(),
                ctx.clause.as_ref(),
                attr,
                word,
            ),
            w,
        )
    }

    /// How well the place takes `word` by its grammar alone (nats, log
    /// ratios, unweighted): the phrase, and right after a governing word
    /// its frame (after «могут» an infinitive). 0 where the grammar says
    /// nothing.
    pub fn grammar_fit(&self, ctx: &Context, word: &str) -> f32 {
        let readings = self.readings(word);
        // The previous word's frame: a governing word's (its preposition's
        // by what it hangs on), or any word's own («он» — hardly an
        // infinitive after it).
        let frame = ctx
            .next
            .or(ctx.prev_frame)
            .map_or(0.0, |f| f.next(&readings));
        self.phrase_nats(ctx, &readings).0 + frame
    }
}
