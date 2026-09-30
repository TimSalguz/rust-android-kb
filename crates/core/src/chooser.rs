//! The chooser (tools/chooser.py): a small transformer that reads the
//! sentence so far and re-weighs the candidates for the word being typed.
//! It adds to the keyboard's own cost what the sentence says beyond the word
//! right before — `logit = −τ·cost + f(sentence, candidate)`, learned so that
//! f = 0 is the keyboard as it is — and chooses among what the search found:
//! calibrated, it writes no words of its own.
//!
//! Each word before (at most `ctx`, oldest first) is its lemma's context
//! vector ([`crate::lemmas`], the same the pairs use) projected to `d`, plus
//! its grammar class's and its place's embedding; a query token after them
//! reads them through `layers` pre-norm transformer layers. The candidate is
//! its lemma's target vector projected, plus its class's embedding; f is the
//! two's scaled product plus a linear term on the candidate's features (the
//! edit, its odds, whether it was typed, its cost over the best, no lemma of
//! its own).
//!
//! The blob (little-endian, mmap'd): `b"KBCH"`, u32 version 1, d, layers,
//! heads, ff, lemma dims, classes, ctx, features; then f32 weights in the
//! order tools/chooser.py writes them.

use crate::lemmas::Lemmas;

const MAGIC: &[u8; 4] = b"KBCH";
const HEADER: usize = 4 + 9 * 4;
/// An edit counts up to this much in the features (as tools/chooser.py's
/// EDIT_CAP): the search's bound on f stays finite.
pub const EDIT_CAP: f32 = 10.0;
/// Each feature's range: the edit, the prior nats / 10, no lemma of its own,
/// expected before any letter (a prediction).
const FEATURE_RANGE: [(f32, f32); 4] = [(0.0, EDIT_CAP), (0.0, 3.0), (0.0, 1.0), (0.0, 1.0)];

/// Where one layer's weights start.
struct Layer {
    norm1: (usize, usize),
    in_proj: (usize, usize),
    out_proj: (usize, usize),
    norm2: (usize, usize),
    ff1: (usize, usize),
    ff2: (usize, usize),
}

pub struct Chooser<D: AsRef<[u8]>> {
    data: D,
    d: usize,
    heads: usize,
    ff: usize,
    dv: usize,
    classes: usize,
    ctx: usize,
    feats: usize,
    ctx_proj: (usize, usize),
    ctx_class: usize,
    pos: usize,
    query: usize,
    layers: Vec<Layer>,
    norm: (usize, usize),
    head: (usize, usize),
    tgt_proj: (usize, usize),
    tgt_class: usize,
    feat: (usize, usize),
    tau: usize,
    /// The weights decoded, where the mapped bytes can't be read as floats
    /// in place (big-endian, or not aligned); None: read in place.
    owned: Option<Vec<f32>>,
    /// Sentences read lately, with their `h` and bound: the text before a
    /// word doesn't change while it is typed, nor do the places the
    /// two-word window tries.
    cache: std::sync::Mutex<Vec<Cached>>,
    /// The same by the sentence's words as written: a hit costs no lookup
    /// of their lemmas and classes.
    by_words: std::sync::Mutex<Vec<(Vec<String>, std::sync::Arc<Reading>)>>,
}

/// What the chooser read of a sentence, ready to score candidates: with h
/// its output, `u = W_projᵀ·h` (a lemma's part is `v·u`), `b·h`, each
/// grammar class's `e·h` — a candidate costs a 16-number product — and the
/// most f can be for any candidate there.
#[derive(Debug)]
pub struct Reading {
    u: Vec<f32>,
    bh: f32,
    class_dot: Vec<f32>,
    /// The most the sentence gives any lemma and class: the part of f the
    /// features don't bound.
    sense: f32,
    pub most: f32,
}

/// A sentence's words (lemma rows, classes) and their reading.
type Cached = (Vec<(u32, u32)>, std::sync::Arc<Reading>);

/// Sentences kept read.
const CACHED: usize = 16;

impl<D: AsRef<[u8]>> Chooser<D> {
    /// Read a blob's layout (None: not one, or cut short).
    pub fn new(data: D) -> Option<Self> {
        let b = data.as_ref();
        if b.len() < HEADER || &b[..4] != MAGIC || u32_at(b, 4) != 1 {
            return None;
        }
        let h: Vec<usize> = (1..9).map(|i| u32_at(b, 4 + 4 * i) as usize).collect();
        let (d, n_layers, heads, ff, dv, classes, ctx, feats) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        if heads == 0 || d % heads != 0 {
            return None;
        }
        // Offsets in floats from the end of the header, in the writer's order.
        let mut at = 0usize;
        let mut take = |n: usize| {
            let o = at;
            at += n;
            o
        };
        let ctx_proj = (take(d * dv), take(d));
        let ctx_class = take(classes * d);
        let pos = take((ctx + 1) * d);
        let query = take(d);
        let mut layers = Vec::new();
        for _ in 0..n_layers {
            layers.push(Layer {
                norm1: (take(d), take(d)),
                in_proj: (take(3 * d * d), take(3 * d)),
                out_proj: (take(d * d), take(d)),
                norm2: (take(d), take(d)),
                ff1: (take(ff * d), take(ff)),
                ff2: (take(d * ff), take(d)),
            });
        }
        let norm = (take(d), take(d));
        let head = (take(d * d), take(d));
        let tgt_proj = (take(d * dv), take(d));
        let tgt_class = take(classes * d);
        let feat = (take(feats), take(1));
        let tau = take(1);
        if b.len() != HEADER + 4 * at {
            return None;
        }
        let body = &b[HEADER..];
        let in_place = cfg!(target_endian = "little") && (body.as_ptr() as usize).is_multiple_of(4);
        let owned = (!in_place).then(|| {
            body.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect()
        });
        Some(Chooser {
            data,
            d,
            heads,
            ff,
            dv,
            classes,
            ctx,
            feats,
            ctx_proj,
            ctx_class,
            pos,
            query,
            layers,
            norm,
            head,
            tgt_proj,
            tgt_class,
            feat,
            tau,
            owned,
            cache: std::sync::Mutex::new(Vec::new()),
            by_words: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Whether it learned to choose among words expected before any letter
    /// (its fourth feature says so).
    pub fn predicts(&self) -> bool {
        self.feats >= 4
    }

    /// Words before it the chooser reads (the most recent).
    pub fn context_len(&self) -> usize {
        self.ctx
    }

    /// The weight on the keyboard's own cost: `cost' = cost − f/τ` orders
    /// the candidates as the chooser's logits do.
    pub fn tau(&self) -> f32 {
        self.w(self.tau)
    }

    /// The weights as floats.
    fn ws(&self) -> &[f32] {
        if let Some(o) = &self.owned {
            return o;
        }
        let body = &self.data.as_ref()[HEADER..];
        // SAFETY: checked in `new`: little-endian, 4-aligned, a whole number
        // of floats; every bit pattern is a valid f32; the bytes live as long
        // as `self` (a map or a heap buffer, which moving `self` doesn't move).
        unsafe { std::slice::from_raw_parts(body.as_ptr() as *const f32, body.len() / 4) }
    }

    fn w(&self, i: usize) -> f32 {
        self.ws()[i]
    }

    /// `out = W·x + b` for a torch Linear (W stored out × in, row-major).
    fn linear(&self, (w, b): (usize, usize), x: &[f32], out: &mut [f32]) {
        let n = x.len();
        let ws = self.ws();
        for (o, y) in out.iter_mut().enumerate() {
            let row = &ws[w + o * n..][..n];
            *y = ws[b + o] + row.iter().zip(x).map(|(a, b)| a * b).sum::<f32>();
        }
    }

    /// The reading of a sentence (`words`, as [`Chooser::read`] takes them),
    /// kept for the sentences read lately.
    pub fn reading<L: AsRef<[u8]>>(
        &self,
        lemmas: &Lemmas<L>,
        words: &[(u32, u32)],
    ) -> std::sync::Arc<Reading> {
        let words = &words[words.len().saturating_sub(self.ctx)..];
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(i) = cache.iter().position(|r| r.0 == words) {
                let r = cache.remove(i);
                let out = r.1.clone();
                cache.insert(0, r);
                return out;
            }
        }
        let r = std::sync::Arc::new(self.prepare(lemmas, &self.read(lemmas, words)));
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(0, (words.to_vec(), r.clone()));
            cache.truncate(CACHED);
        }
        r
    }

    /// The reading of a sentence given as its words (lowercase; `tokens`
    /// looks up their lemma rows and classes — only when it isn't kept).
    pub fn reading_of<L: AsRef<[u8]>>(
        &self,
        lemmas: &Lemmas<L>,
        sentence: &[String],
        tokens: impl FnOnce(&[String]) -> Vec<(u32, u32)>,
    ) -> std::sync::Arc<Reading> {
        let sentence = &sentence[sentence.len().saturating_sub(self.ctx)..];
        if let Ok(mut cache) = self.by_words.lock() {
            if let Some(i) = cache.iter().position(|r| r.0 == sentence) {
                let r = cache.remove(i);
                let out = r.1.clone();
                cache.insert(0, r);
                return out;
            }
        }
        let r = self.reading(lemmas, &tokens(sentence));
        if let Ok(mut cache) = self.by_words.lock() {
            cache.insert(0, (sentence.to_vec(), r.clone()));
            cache.truncate(CACHED);
        }
        r
    }

    /// Ready `h` to score candidates, and bound f: the best lemma, the best
    /// class and the features at their best — what a search allows for
    /// before it knows the word.
    pub fn prepare<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, h: &[f32]) -> Reading {
        let (d, dv) = (self.d, self.dv);
        let ws = self.ws();
        let mut u = vec![0f32; dv];
        for (o, hi) in h.iter().enumerate() {
            let row = &ws[self.tgt_proj.0 + o * dv..][..dv];
            u.iter_mut().zip(row).for_each(|(uk, w)| *uk += w * hi);
        }
        let bh: f32 = (0..d).map(|o| ws[self.tgt_proj.1 + o] * h[o]).sum();
        let class_dot: Vec<f32> = (0..self.classes)
            .map(|k| {
                let e = &ws[self.tgt_class + k * d..][..d];
                e.iter().zip(h).map(|(a, b)| a * b).sum()
            })
            .collect();
        let lemma = (0..=lemmas.unk())
            .map(|l| lemmas.tgt_dot(l, &u))
            .fold(f32::MIN, f32::max);
        let class = class_dot.iter().copied().fold(f32::MIN, f32::max);
        let lin: f32 = FEATURE_RANGE
            .iter()
            .enumerate()
            .take(self.feats)
            .map(|(i, &(lo, hi))| {
                let w = ws[self.feat.0 + i];
                (w * lo).max(w * hi)
            })
            .sum::<f32>()
            + ws[self.feat.1];
        let sense = (lemma + bh + class) / (d as f32).sqrt();
        Reading {
            u,
            bh,
            class_dot,
            sense,
            most: sense + lin,
        }
    }

    /// The most f can be for a word whose prior is at least `least` nats:
    /// the features' part bounded by it — the chooser holds rare words back
    /// (its weight on the prior is negative), so where a search's words are
    /// all rare it allows for much less. Never less than f of such a word.
    pub fn bound(&self, r: &Reading, least: f32) -> f32 {
        let ws = self.ws();
        let prior_lo = (least / 10.0).clamp(0.0, FEATURE_RANGE[1].1);
        let lin: f32 = FEATURE_RANGE
            .iter()
            .enumerate()
            .take(self.feats)
            .map(|(i, &(lo, hi))| {
                let w = ws[self.feat.0 + i];
                let lo = if i == 1 { prior_lo.max(lo) } else { lo };
                (w * lo).max(w * hi)
            })
            .sum::<f32>()
            + ws[self.feat.1];
        r.sense + lin
    }

    /// What a reading ([`Chooser::reading`]) adds to a candidate's logit:
    /// its lemma row, its grammar class, its features (the edit up to
    /// [`EDIT_CAP`], its prior nats / 10, no lemma of its own).
    pub fn score<L: AsRef<[u8]>>(
        &self,
        lemmas: &Lemmas<L>,
        r: &Reading,
        lemma: u32,
        class: u32,
        feats: &[f32],
    ) -> f32 {
        let ws = self.ws();
        let k = (class as usize).min(self.classes - 1);
        let lin: f32 = (0..self.feats.min(feats.len()))
            .map(|i| ws[self.feat.0 + i] * feats[i])
            .sum::<f32>()
            + ws[self.feat.1];
        (lemmas.tgt_dot(lemma, &r.u) + r.bh + r.class_dot[k]) / (self.d as f32).sqrt() + lin
    }

    fn layer_norm(&self, (g, b): (usize, usize), x: &[f32]) -> Vec<f32> {
        let n = x.len() as f32;
        let mean = x.iter().sum::<f32>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + 1e-5).sqrt();
        x.iter()
            .enumerate()
            .map(|(i, v)| (v - mean) * inv * self.w(g + i) + self.w(b + i))
            .collect()
    }

    /// What the sentence so far says: the query token's reading of `words`
    /// — each `(lemma row, grammar class)`, oldest first; at most
    /// [`Chooser::context_len`] are read, the most recent — as `d` numbers.
    /// No words: a sentence's start (`lemmas.start()`).
    pub fn read<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, words: &[(u32, u32)]) -> Vec<f32> {
        let d = self.d;
        let start = [(lemmas.start(), 0)];
        let words = if words.is_empty() { &start[..] } else { words };
        let words = &words[words.len().saturating_sub(self.ctx)..];
        let n = words.len();
        let mut v = vec![0f32; self.dv];
        // The tokens: the words, then the query; places right-aligned.
        let mut xs: Vec<Vec<f32>> = Vec::with_capacity(n + 1);
        for (j, &(l, c)) in words.iter().enumerate() {
            lemmas.ctx_vector(l, &mut v);
            let mut x = vec![0f32; d];
            self.linear(self.ctx_proj, &v, &mut x);
            let c = (c as usize).min(self.classes - 1);
            let p = self.ctx - n + j;
            for (i, xi) in x.iter_mut().enumerate() {
                *xi += self.w(self.ctx_class + c * d + i) + self.w(self.pos + p * d + i);
            }
            xs.push(x);
        }
        xs.push(
            (0..d)
                .map(|i| self.w(self.query + i) + self.w(self.pos + self.ctx * d + i))
                .collect(),
        );
        let hd = d / self.heads;
        let scale = 1.0 / (hd as f32).sqrt();
        for layer in &self.layers {
            // Attention: every token reads every token (no padding here).
            let normed: Vec<Vec<f32>> =
                xs.iter().map(|x| self.layer_norm(layer.norm1, x)).collect();
            let qkv: Vec<Vec<f32>> = normed
                .iter()
                .map(|x| {
                    let mut o = vec![0f32; 3 * d];
                    self.linear(layer.in_proj, x, &mut o);
                    o
                })
                .collect();
            let mut next = Vec::with_capacity(xs.len());
            for (t, x) in xs.iter().enumerate() {
                let mut att = vec![0f32; d];
                for h in 0..self.heads {
                    let q = &qkv[t][h * hd..(h + 1) * hd];
                    let s: Vec<f32> = qkv
                        .iter()
                        .map(|r| {
                            let k = &r[d + h * hd..d + (h + 1) * hd];
                            q.iter().zip(k).map(|(a, b)| a * b).sum::<f32>() * scale
                        })
                        .collect();
                    let m = s.iter().copied().fold(f32::MIN, f32::max);
                    let e: Vec<f32> = s.iter().map(|v| (v - m).exp()).collect();
                    let z: f32 = e.iter().sum();
                    for (r, w) in qkv.iter().zip(&e) {
                        let val = &r[2 * d + h * hd..2 * d + (h + 1) * hd];
                        for (i, vv) in val.iter().enumerate() {
                            att[h * hd + i] += w / z * vv;
                        }
                    }
                }
                let mut o = vec![0f32; d];
                self.linear(layer.out_proj, &att, &mut o);
                let mut y: Vec<f32> = x.iter().zip(&o).map(|(a, b)| a + b).collect();
                // Feed-forward.
                let n2 = self.layer_norm(layer.norm2, &y);
                let mut hid = vec![0f32; self.ff];
                self.linear(layer.ff1, &n2, &mut hid);
                hid.iter_mut().for_each(|v| *v = v.max(0.0));
                let mut o2 = vec![0f32; d];
                self.linear(layer.ff2, &hid, &mut o2);
                y.iter_mut().zip(&o2).for_each(|(a, b)| *a += b);
                next.push(y);
            }
            xs = next;
        }
        let last = self.layer_norm(self.norm, xs.last().unwrap());
        let mut h = vec![0f32; d];
        self.linear(self.head, &last, &mut h);
        h
    }
}

impl Chooser<memmap2::Mmap> {
    /// Map a blob from a file (read-only, like the FSTs).
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the blob is a read-only, prebuilt asset; we don't mutate it.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Chooser::new(mmap).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "not a chooser blob")
        })
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
pub(crate) mod tests {
    /// A chooser blob of the given sizes with pseudo-random weights.
    pub(crate) fn blob(d: usize, layers: usize, dv: usize, classes: usize, ctx: usize) -> Vec<u8> {
        let (heads, ff, feats) = (2, 2 * d, 3);
        let per_layer = 2 * d + 3 * d * d + 3 * d + d * d + d + 2 * d + ff * d + ff + d * ff + d;
        let n = d * dv
            + d
            + classes * d
            + (ctx + 1) * d
            + d
            + layers * per_layer
            + 2 * d
            + d * d
            + d
            + d * dv
            + d
            + classes * d
            + feats
            + 1
            + 1;
        let mut out = b"KBCH".to_vec();
        for x in [1, d, layers, heads, ff, dv, classes, ctx, feats] {
            out.extend((x as u32).to_le_bytes());
        }
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        for i in 0..n {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = (s % 2000) as f32 / 1000.0 - 1.0;
            // τ (the last number) positive.
            let x = if i == n - 1 { 1.0 } else { x };
            out.extend(x.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_blob_reads_back() {
        let c = super::Chooser::new(blob(8, 1, 4, 5, 3)).unwrap();
        assert_eq!((c.context_len(), c.tau()), (3, 1.0));
        assert!(super::Chooser::new(blob(8, 1, 4, 5, 3)[..100].to_vec()).is_none());
    }
}
