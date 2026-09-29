//! Lemma vectors (tools/lemma_vectors.py, tools/lemma_export.py): what the
//! previous word's lemma says about the next word's — a low-rank model of
//! which lemma follows which, `P(l | c) = softmax(u_c · v_l + b_l)`, learned
//! with its probabilities calibrated.
//!
//! The keyboard uses it as a PMI on the form's own odds: `P(form | previous)
//! = P(form)·e^PMI(c, l)`, `PMI(c, l) = log P(l | c) − log P(l)` — the form
//! the grammar picks, the lemma the sense. Lemmas never seen together still
//! get a fair chance from what their neighbours share: «пить чай» teaches
//! «пить кофе».
//!
//! The blob (little-endian, mmap'd like the FSTs): `b"KBLM"`, u32 version 1,
//! u32 V (lemmas; UNK — every rarer lemma — is V), u32 d; then V + 2 context
//! rows (the lemmas, UNK, START: a sentence's first word): i8 vectors, f32
//! scales, f32 log Z, f32 the most PMI after it; V + 1 target rows: i8
//! vectors, f32 scales, f32 bias less `log P(l)`, f32 `log P(l)`; then each
//! lemma's stem, 8 letters (alphabet ids, 0 after the end): where its forms
//! lie among the context model's `0, 25, word` keys, which hold each word's
//! lemma id.

const MAGIC: &[u8; 4] = b"KBLM";
/// Letters of a lemma's stem kept.
const STEM: usize = 8;

pub struct Lemmas<D: AsRef<[u8]>> {
    data: D,
    /// Lemmas with vectors (UNK is `v`, START `v + 1`), numbers per vector.
    v: usize,
    d: usize,
    ctx_vec: usize,
    ctx_scale: usize,
    ctx_logz: usize,
    ctx_most: usize,
    tgt_vec: usize,
    tgt_scale: usize,
    tgt_bias: usize,
    tgt_logp: usize,
    stems: usize,
}

impl<D: AsRef<[u8]>> Lemmas<D> {
    /// Read a blob's layout (None: not one, or cut short).
    pub fn new(data: D) -> Option<Self> {
        let b = data.as_ref();
        if b.len() < 16 || &b[..4] != MAGIC || u32_at(b, 4) != 1 {
            return None;
        }
        let (v, d) = (u32_at(b, 8) as usize, u32_at(b, 12) as usize);
        if d == 0 || d % 4 != 0 {
            return None;
        }
        let (nc, nt) = (v + 2, v + 1);
        let ctx_vec = 16;
        let ctx_scale = ctx_vec + nc * d;
        let ctx_logz = ctx_scale + nc * 4;
        let ctx_most = ctx_logz + nc * 4;
        let tgt_vec = ctx_most + nc * 4;
        let tgt_scale = tgt_vec + nt * d;
        let tgt_bias = tgt_scale + nt * 4;
        let tgt_logp = tgt_bias + nt * 4;
        let stems = tgt_logp + nt * 4;
        if b.len() != stems + v * STEM {
            return None;
        }
        Some(Lemmas {
            data,
            v,
            d,
            ctx_vec,
            ctx_scale,
            ctx_logz,
            ctx_most,
            tgt_vec,
            tgt_scale,
            tgt_bias,
            tgt_logp,
            stems,
        })
    }

    /// The id every lemma without vectors of its own shares.
    pub fn unk(&self) -> u32 {
        self.v as u32
    }

    /// The context of a sentence's first word.
    pub fn start(&self) -> u32 {
        self.v as u32 + 1
    }

    /// How much likelier lemma `l` comes after `c` than on its own:
    /// `log P(l | c) − log P(l)`, nats. `c` a lemma id, UNK or START; `l` a
    /// lemma id or UNK.
    pub fn pmi(&self, c: u32, l: u32) -> f32 {
        let b = self.data.as_ref();
        let (c, l) = ((c as usize).min(self.v + 1), (l as usize).min(self.v));
        let u = &b[self.ctx_vec + c * self.d..][..self.d];
        let w = &b[self.tgt_vec + l * self.d..][..self.d];
        let dot: i32 = u
            .iter()
            .zip(w)
            .map(|(&x, &y)| x as i8 as i32 * y as i8 as i32)
            .sum();
        dot as f32 * f32_at(b, self.ctx_scale + 4 * c) * f32_at(b, self.tgt_scale + 4 * l)
            + f32_at(b, self.tgt_bias + 4 * l)
            - f32_at(b, self.ctx_logz + 4 * c)
    }

    /// The lemmas likeliest after `c` (`log P(l | c)`, best first): what
    /// the previous word's lemma predicts, whatever words it was seen with.
    pub fn top(&self, c: u32, n: usize) -> Vec<(u32, f32)> {
        let b = self.data.as_ref();
        let mut best: Vec<(u32, f32)> = Vec::with_capacity(n + 1);
        for l in 0..self.v as u32 {
            let lp = self.pmi(c, l) + f32_at(b, self.tgt_logp + 4 * l as usize);
            if best.len() == n && best[n - 1].1 >= lp {
                continue;
            }
            let at = best.partition_point(|x| x.1 >= lp);
            best.insert(at, (l, lp));
            best.truncate(n);
        }
        best
    }

    /// The letters a lemma's forms start with (alphabet ids).
    pub fn stem(&self, l: u32) -> &[u8] {
        let l = (l as usize).min(self.v.saturating_sub(1));
        let s = &self.data.as_ref()[self.stems + l * STEM..][..STEM];
        &s[..s.iter().position(|&x| x == 0).unwrap_or(STEM)]
    }

    /// The most PMI any lemma gets after `c`: what a search allows for
    /// before it knows the word.
    pub fn most(&self, c: u32) -> f32 {
        let c = (c as usize).min(self.v + 1);
        f32_at(self.data.as_ref(), self.ctx_most + 4 * c)
    }
}

impl Lemmas<memmap2::Mmap> {
    /// Map a blob from a file (read-only, like the FSTs).
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the blob is a read-only, prebuilt asset; we don't mutate it.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Lemmas::new(mmap)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a lemma blob"))
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A blob of `v` lemmas, `d` numbers each, from float rows.
    pub(crate) fn blob(
        ctx: &[Vec<f32>],
        tgt: &[Vec<f32>],
        bias: &[f32],
        stems: &[&[u8]],
    ) -> Vec<u8> {
        let (v, d) = (tgt.len() - 1, tgt[0].len());
        let q = |rows: &[Vec<f32>]| -> (Vec<u8>, Vec<f32>) {
            let mut bytes = Vec::new();
            let mut scales = Vec::new();
            for r in rows {
                let s = r.iter().fold(0f32, |m, x| m.max(x.abs())) / 127.0 + 1e-12;
                bytes.extend(r.iter().map(|x| (x / s).round() as i8 as u8));
                scales.push(s);
            }
            (bytes, scales)
        };
        let (cb, cs) = q(ctx);
        let (tb, ts) = q(tgt);
        // log Z over the targets (bias here is b − log P, P uniform).
        let lp = -((v + 1) as f32).ln();
        let score = |c: usize, l: usize| -> f32 {
            let dot: i32 = (0..d)
                .map(|k| cb[c * d + k] as i8 as i32 * tb[l * d + k] as i8 as i32)
                .sum();
            dot as f32 * cs[c] * ts[l] + bias[l] + lp
        };
        let logz: Vec<f32> = (0..ctx.len())
            .map(|c| (0..=v).map(|l| score(c, l).exp()).sum::<f32>().ln())
            .collect();
        let most: Vec<f32> = (0..ctx.len())
            .map(|c| {
                (0..=v)
                    .map(|l| score(c, l) - lp - logz[c])
                    .fold(f32::MIN, f32::max)
            })
            .collect();
        let mut out = MAGIC.to_vec();
        for x in [1u32, v as u32, d as u32] {
            out.extend(x.to_le_bytes());
        }
        out.extend(&cb);
        for a in [&cs, &logz, &most] {
            out.extend(a.iter().flat_map(|x| x.to_le_bytes()));
        }
        out.extend(&tb);
        let logp = vec![lp; v + 1];
        for a in [&ts, &bias.to_vec(), &logp] {
            out.extend(a.iter().flat_map(|x| x.to_le_bytes()));
        }
        for i in 0..v {
            let mut s = stems.get(i).copied().unwrap_or_default().to_vec();
            s.resize(STEM, 0);
            out.extend(s);
        }
        out
    }

    #[test]
    fn the_odds_after_a_lemma_sum_to_one() {
        // Two lemmas and UNK; «пить» (0) goes with «чай» (1).
        let ctx = vec![
            vec![1.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.0; 4],
            vec![0.0; 4],
        ];
        let tgt = vec![
            vec![0.0, 1.0, 0.0, 0.0],
            vec![2.0, 0.0, 0.0, 0.0],
            vec![0.0; 4],
        ];
        let lemmas = Lemmas::new(blob(&ctx, &tgt, &[0.0; 3], &[&[5, 6], &[7]])).unwrap();
        assert_eq!((lemmas.unk(), lemmas.start()), (2, 3));
        assert!(lemmas.pmi(0, 1) > 0.5 && lemmas.pmi(0, 0) < 0.0);
        // P(l | c) = P(l)·e^PMI sums to 1 (P uniform here).
        let sum: f32 = (0..3).map(|l| lemmas.pmi(0, l).exp() / 3.0).sum();
        assert!((sum - 1.0).abs() < 1e-3, "{sum}");
        assert!((0..3).all(|l| lemmas.pmi(0, l) <= lemmas.most(0) + 1e-6));
        // START knows nothing here: every lemma as likely as on its own.
        assert!(lemmas.pmi(3, 1).abs() < 1e-3);
        // What «пить» predicts first: «чай».
        assert_eq!(lemmas.top(0, 1)[0].0, 1);
        assert_eq!(lemmas.stem(0), &[5, 6]);
        assert!(Lemmas::new(vec![0u8; 16]).is_none());
    }
}
