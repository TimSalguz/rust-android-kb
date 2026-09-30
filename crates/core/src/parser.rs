//! The student parser (tools/graph_parser.py, tools/graph_parser_export.py):
//! the chances of the links of a sentence's graph — which word each word
//! hangs on — on the keyboard's own structures: each word's lemma vector
//! ([`crate::lemmas`]), its grammar class, the grammemes of all its readings
//! together. It weighs the links; it throws none away (the grammar's graph
//! keeps them all).
//!
//! Causal (as the keyboard reads a sentence): each word sees the words
//! before it only, and hangs on one of them, on the root, or on a word
//! still to come ("later"). Pre-norm transformer layers over the root and
//! the words; a word's chance of each head is its dependent projection ·
//! the head's projection, softmaxed over the heads it may have.
//!
//! The blob: see tools/graph_parser_export.py.

use crate::gram;
use crate::lemmas::Lemmas;

const MAGIC: &[u8; 4] = b"KBGP";

/// A word as the parser reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    /// Its lemma's row in the lemma vectors (UNK for any other word).
    pub lemma: u32,
    /// Its grammar class (0: none known).
    pub class: u32,
    /// The grammemes of all its readings ([`crate::gram::bit`]).
    pub grammemes: u64,
}

struct Layer {
    norm1: (usize, usize),
    in_proj: (usize, usize),
    out_proj: (usize, usize),
    norm2: (usize, usize),
    ff1: (usize, usize),
    ff2: (usize, usize),
}

pub struct Parser<D: AsRef<[u8]>> {
    data: D,
    body: usize,
    d: usize,
    heads: usize,
    ff: usize,
    dv: usize,
    classes: usize,
    places: usize,
    causal: bool,
    /// Each grammeme input's bit.
    grams: Vec<u64>,
    relations: Vec<String>,
    lemma: (usize, usize),
    class: usize,
    gram: (usize, usize),
    pos: usize,
    root: usize,
    layers: Vec<Layer>,
    norm: (usize, usize),
    dep: [(usize, usize); 2],
    hd: [(usize, usize); 2],
    later: usize,
    owned: Option<Vec<f32>>,
    /// Sentences parsed lately (a sentence doesn't change while a word of it
    /// is typed).
    cache: std::sync::Mutex<Vec<(Vec<Word>, Graph)>>,
}

/// A sentence's graph: each word's chances of its heads.
pub type Graph = std::sync::Arc<Vec<Vec<f32>>>;

/// Sentences kept parsed.
const CACHED: usize = 8;

impl<D: AsRef<[u8]>> Parser<D> {
    /// Read a blob's layout (None: not one, or cut short).
    pub fn new(data: D) -> Option<Self> {
        let b = data.as_ref();
        if b.len() < 48 || &b[..4] != MAGIC || u32_at(b, 4) != 1 {
            return None;
        }
        let h: Vec<usize> = (0..10).map(|i| u32_at(b, 8 + 4 * i) as usize).collect();
        let (d, n_layers, heads, ff, dv, classes, places, n_gram, n_rel, causal) = (
            h[0],
            h[1],
            h[2],
            h[3],
            h[4],
            h[5],
            h[6],
            h[7],
            h[8],
            h[9] == 1,
        );
        if heads == 0 || d % heads != 0 {
            return None;
        }
        let mut at = 48;
        let mut names = Vec::new();
        for _ in 0..n_gram + n_rel {
            let n = u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize;
            names.push(
                std::str::from_utf8(b.get(at + 2..at + 2 + n)?)
                    .ok()?
                    .to_string(),
            );
            at += 2 + n;
        }
        let body = at + (4 - at % 4) % 4;
        let grams = names[..n_gram].iter().map(|g| gram::parse(g)).collect();
        let relations = names[n_gram..].to_vec();
        let mut off = 0usize;
        let mut take = |n: usize| {
            let o = off;
            off += n;
            o
        };
        let lemma = (take(d * dv), take(d));
        let class = take(classes * d);
        let gram_w = (take(d * n_gram), take(d));
        let pos = take(places * d);
        let root = take(d);
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
        let dep = [(take(d * d), take(d)), (take(d * d), take(d))];
        let hd = [(take(d * d), take(d)), (take(d * d), take(d))];
        let later = take(d);
        let _rel = [(take(d * 2 * d), take(d)), (take(n_rel * d), take(n_rel))];
        if b.len() != body + 4 * off {
            return None;
        }
        let floats = &b[body..];
        let in_place =
            cfg!(target_endian = "little") && (floats.as_ptr() as usize).is_multiple_of(4);
        let owned = (!in_place).then(|| {
            floats
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect()
        });
        Some(Parser {
            data,
            body,
            d,
            heads,
            ff,
            dv,
            classes,
            places,
            causal,
            grams,
            relations,
            lemma,
            class,
            gram: gram_w,
            pos,
            root,
            layers,
            norm,
            dep,
            hd,
            later,
            owned,
            cache: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Words it reads at most (the most recent).
    pub fn places(&self) -> usize {
        self.places - 1
    }

    /// Whether a word sees only the words before it.
    pub fn causal(&self) -> bool {
        self.causal
    }

    /// The relations' names (UD), as the parser numbers them.
    pub fn relations(&self) -> &[String] {
        &self.relations
    }

    fn ws(&self) -> &[f32] {
        if let Some(o) = &self.owned {
            return o;
        }
        let floats = &self.data.as_ref()[self.body..];
        // SAFETY: checked in `new`: little-endian, 4-aligned, a whole number
        // of floats; every bit pattern is a valid f32; the bytes live as long
        // as `self`.
        unsafe { std::slice::from_raw_parts(floats.as_ptr() as *const f32, floats.len() / 4) }
    }

    fn linear(&self, (w, b): (usize, usize), x: &[f32], out: &mut [f32]) {
        let n = x.len();
        let ws = self.ws();
        for (o, y) in out.iter_mut().enumerate() {
            let row = &ws[w + o * n..][..n];
            *y = ws[b + o] + row.iter().zip(x).map(|(a, b)| a * b).sum::<f32>();
        }
    }

    fn layer_norm(&self, (g, b): (usize, usize), x: &[f32]) -> Vec<f32> {
        let ws = self.ws();
        let n = x.len() as f32;
        let mean = x.iter().sum::<f32>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + 1e-5).sqrt();
        x.iter()
            .enumerate()
            .map(|(i, v)| (v - mean) * inv * ws[g + i] + ws[b + i])
            .collect()
    }

    fn mlp(&self, l: &[(usize, usize); 2], x: &[f32]) -> Vec<f32> {
        let mut a = vec![0f32; self.d];
        self.linear(l[0], x, &mut a);
        a.iter_mut().for_each(|v| *v = v.max(0.0));
        let mut o = vec![0f32; self.d];
        self.linear(l[1], &a, &mut o);
        o
    }

    /// [`Parser::parse`], kept for the sentences parsed lately.
    pub fn parse_cached<L: AsRef<[u8]>>(
        &self,
        lemmas: &Lemmas<L>,
        words: &[Word],
    ) -> Graph {
        let words = &words[words.len().saturating_sub(self.places())..];
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(i) = cache.iter().position(|r| r.0 == words) {
                let r = cache.remove(i);
                let out = r.1.clone();
                cache.insert(0, r);
                return out;
            }
        }
        let g = std::sync::Arc::new(self.parse(lemmas, words));
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(0, (words.to_vec(), g.clone()));
            cache.truncate(CACHED);
        }
        g
    }

    /// Each word's chances of its heads: for word i (in `words` order), a
    /// vector over the root (0), the words (1..=n, the word itself 0) and —
    /// causal — "a word still to come" (n + 1); a causal parser gives the
    /// words after i nothing. Only the last [`Parser::places`] words are read.
    pub fn parse<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, words: &[Word]) -> Vec<Vec<f32>> {
        let words = &words[words.len().saturating_sub(self.places())..];
        let (d, n) = (self.d, words.len());
        let ws = self.ws();
        let mut v = vec![0f32; self.dv];
        let mut bag = vec![0f32; self.grams.len()];
        let mut xs: Vec<Vec<f32>> = Vec::with_capacity(n + 1);
        xs.push(
            (0..d)
                .map(|i| ws[self.root + i] + ws[self.pos + i])
                .collect(),
        );
        for (j, w) in words.iter().enumerate() {
            lemmas.ctx_vector(w.lemma, &mut v);
            let mut x = vec![0f32; d];
            self.linear(self.lemma, &v, &mut x);
            for (k, b) in bag.iter_mut().enumerate() {
                *b = if w.grammemes & self.grams[k] != 0 {
                    1.0
                } else {
                    0.0
                };
            }
            let mut gx = vec![0f32; d];
            self.linear(self.gram, &bag, &mut gx);
            let c = (w.class as usize).min(self.classes - 1);
            for (i, xi) in x.iter_mut().enumerate() {
                *xi += gx[i] + ws[self.class + c * d + i] + ws[self.pos + (j + 1) * d + i];
            }
            xs.push(x);
        }
        let hd_n = d / self.heads;
        let scale = 1.0 / (hd_n as f32).sqrt();
        for layer in &self.layers {
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
                // Causal: a token reads the tokens up to itself.
                let seen = if self.causal { t + 1 } else { xs.len() };
                let mut att = vec![0f32; d];
                for h in 0..self.heads {
                    let q = &qkv[t][h * hd_n..(h + 1) * hd_n];
                    let s: Vec<f32> = qkv[..seen]
                        .iter()
                        .map(|r| {
                            let k = &r[d + h * hd_n..d + (h + 1) * hd_n];
                            q.iter().zip(k).map(|(a, b)| a * b).sum::<f32>() * scale
                        })
                        .collect();
                    let m = s.iter().copied().fold(f32::MIN, f32::max);
                    let e: Vec<f32> = s.iter().map(|v| (v - m).exp()).collect();
                    let z: f32 = e.iter().sum();
                    for (r, w) in qkv[..seen].iter().zip(&e) {
                        let val = &r[2 * d + h * hd_n..2 * d + (h + 1) * hd_n];
                        for (i, vv) in val.iter().enumerate() {
                            att[h * hd_n + i] += w / z * vv;
                        }
                    }
                }
                let mut o = vec![0f32; d];
                self.linear(layer.out_proj, &att, &mut o);
                let mut y: Vec<f32> = x.iter().zip(&o).map(|(a, b)| a + b).collect();
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
        let hs: Vec<Vec<f32>> = xs.iter().map(|x| self.layer_norm(self.norm, x)).collect();
        let deps: Vec<Vec<f32>> = hs.iter().map(|h| self.mlp(&self.dep, h)).collect();
        let heads: Vec<Vec<f32>> = hs.iter().map(|h| self.mlp(&self.hd, h)).collect();
        let sd = (d as f32).sqrt();
        let later = &ws[self.later..self.later + d];
        (1..=n)
            .map(|i| {
                let dot =
                    |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / sd;
                let mut s: Vec<f32> = (0..=n)
                    .map(|j| {
                        let allowed = j != i && (!self.causal || j < i);
                        if allowed {
                            dot(&deps[i], &heads[j])
                        } else {
                            f32::NEG_INFINITY
                        }
                    })
                    .collect();
                if self.causal {
                    s.push(dot(&deps[i], later));
                }
                let m = s.iter().copied().fold(f32::MIN, f32::max);
                let e: Vec<f32> = s
                    .iter()
                    .map(|v| if v.is_finite() { (v - m).exp() } else { 0.0 })
                    .collect();
                let z: f32 = e.iter().sum();
                e.iter().map(|v| v / z).collect()
            })
            .collect()
    }
}

impl Parser<memmap2::Mmap> {
    /// Map a blob from a file (read-only, like the FSTs).
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the blob is a read-only, prebuilt asset; we don't mutate it.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Parser::new(mmap).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "not a parser blob")
        })
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
