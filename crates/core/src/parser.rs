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
    /// The marks before it, a bit per kind ([`MARK_KINDS`]): read by a
    /// parser that learned them.
    pub marks: u16,
}

/// The kinds of marks a word may have before it, in the parser's order
/// (tools/graph_parser.py).
pub const MARK_KINDS: [char; 13] = [',', '—', ':', ';', '(', ')', '«', '»', '"', '.', '?', '!', '…'];

/// The marks in `text` (the text between the word before and this one), as
/// a word's `marks`: «, —» → comma and dash.
pub fn marks_of(text: &str) -> u16 {
    text.chars()
        .map(|c| match c {
            '–' | '-' => '—',
            '„' => '«',
            '“' | '”' => '»',
            c => c,
        })
        .filter_map(|c| MARK_KINDS.iter().position(|&k| k == c))
        .fold(0, |a, k| a | 1 << k)
}

/// A weight matrix in the blob: `rows × cols` f32 (version 1) or i8 with a
/// scale per row (version 2); byte offsets from the blob's start.
#[derive(Clone, Copy)]
struct Mat {
    at: usize,
    scales: Option<usize>,
    cols: usize,
}

/// A linear layer: its matrix and the byte offset of its bias.
type Linear = (Mat, usize);

struct Layer {
    norm1: (usize, usize),
    in_proj: Linear,
    out_proj: Linear,
    norm2: (usize, usize),
    ff1: Linear,
    ff2: Linear,
}

pub struct Parser<D: AsRef<[u8]>> {
    data: D,
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
    lemma: Linear,
    class: Mat,
    gram: Linear,
    /// The marks' projection (a parser that reads them).
    marks: Option<Mat>,
    pos: Mat,
    root: usize,
    layers: Vec<Layer>,
    norm: (usize, usize),
    dep: [Linear; 2],
    hd: [Linear; 2],
    later: usize,
    /// How a word waiting hangs on the head still to come (causal field 2).
    wait_rel: Option<[Linear; 2]>,
    /// Whether a comma stands before a word, from the text without its
    /// commas (a whole sentence's parser, field + 8).
    comma: Option<[Linear; 2]>,
    /// A word's relation to its head, from the two: [word, head] → relation.
    rel: [Linear; 2],
    /// The blob copied to 4-aligned memory, when it doesn't lie so.
    owned: Option<Vec<u32>>,
    /// Sentences read lately, word by word (a sentence doesn't change while
    /// a word of it is typed; the next one starts with it).
    cache: std::sync::Mutex<Vec<Read>>,
}

/// A sentence read so far by the causal parser: what each word after it
/// needs of it — every token's keys and values, layer by layer, and head
/// projection — and each word's chances.
#[derive(Clone, Default)]
struct Read {
    words: Vec<Word>,
    /// Per token (the root first): layer by layer, its key, then its value.
    kv: Vec<Vec<f32>>,
    /// Per token: its projection as a head.
    heads: Vec<Vec<f32>>,
    /// Per word: its chances of the root, of each word before it, and of a
    /// word still to come.
    rows: Vec<Vec<f32>>,
    /// Per word: were its head still to come, the chances of each relation.
    waiting: Vec<Vec<f32>>,
}

/// A sentence's graph: each word's chances of its heads, and how a word
/// waiting would hang on the head still to come.
#[derive(Clone, Debug, Default)]
pub struct Sentence {
    /// Word i's row: the root (0), the words (1..=n; nothing for the word
    /// itself and — causal — the words after it) and, causal, a word still to
    /// come (n + 1).
    pub heads: Vec<Vec<f32>>,
    /// Word i's chances of each relation ([`Parser::relations`]) to a head
    /// still to come (empty: the parser doesn't tell).
    pub waiting: Vec<Vec<f32>>,
    /// Word i's relation to its likeliest head, and how sure (a whole
    /// sentence's graph; empty else).
    pub relations: Vec<(usize, f32)>,
    /// Word i's chances of every relation to its likeliest head (a whole
    /// sentence's graph; empty else).
    pub relation_chances: Vec<Vec<f32>>,
    /// The chance of a comma before word i (a parser with the comma head;
    /// empty else).
    pub commas: Vec<f32>,
}

/// A sentence's graph, shared.
pub type Graph = std::sync::Arc<Sentence>;

/// Sentences kept parsed.
const CACHED: usize = 8;

impl<D: AsRef<[u8]>> Parser<D> {
    /// Read a blob's layout (None: not one, or cut short).
    pub fn new(data: D) -> Option<Self> {
        let b = data.as_ref();
        let version = if b.len() >= 48 && &b[..4] == MAGIC {
            u32_at(b, 4)
        } else {
            0
        };
        if !matches!(version, 1 | 2) || !cfg!(target_endian = "little") {
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
            h[9],
        );
        // The causal field: 0 whole sentence, 1 causal, 2 causal and the
        // waiting word's relation; + 4: the marks read; + 8: the comma head.
        let (causal, waiting, reads_marks, commas) =
            (causal & 3 >= 1, causal & 3 == 2, causal & 4 != 0, causal & 8 != 0);
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
        // Byte offsets, in the blob's order: f32 vectors; matrices f32
        // (version 1) or a scale per row and i8 rows, padded to 4 bytes.
        let mut off = body;
        let vector = |n: usize, off: &mut usize| {
            let o = *off;
            *off += 4 * n;
            o
        };
        let matrix = |rows: usize, cols: usize, off: &mut usize| {
            if version == 1 {
                let at = *off;
                *off += 4 * rows * cols;
                Mat { at, scales: None, cols }
            } else {
                let scales = *off;
                let at = scales + 4 * rows;
                *off = at + rows * cols;
                *off += (4 - *off % 4) % 4;
                Mat { at, scales: Some(scales), cols }
            }
        };
        let linear = |rows: usize, cols: usize, off: &mut usize| {
            let m = matrix(rows, cols, off);
            (m, vector(rows, off))
        };
        let lemma = linear(d, dv, &mut off);
        let class = matrix(classes, d, &mut off);
        let gram_w = linear(d, n_gram, &mut off);
        let marks = reads_marks.then(|| matrix(d, MARK_KINDS.len(), &mut off));
        let pos = matrix(places, d, &mut off);
        let root = vector(d, &mut off);
        let mut layers = Vec::new();
        for _ in 0..n_layers {
            layers.push(Layer {
                norm1: (vector(d, &mut off), vector(d, &mut off)),
                in_proj: linear(3 * d, d, &mut off),
                out_proj: linear(d, d, &mut off),
                norm2: (vector(d, &mut off), vector(d, &mut off)),
                ff1: linear(ff, d, &mut off),
                ff2: linear(d, ff, &mut off),
            });
        }
        let norm = (vector(d, &mut off), vector(d, &mut off));
        let dep = [linear(d, d, &mut off), linear(d, d, &mut off)];
        let hd = [linear(d, d, &mut off), linear(d, d, &mut off)];
        let later = vector(d, &mut off);
        let rel = [linear(d, 2 * d, &mut off), linear(n_rel, d, &mut off)];
        let wait_rel = waiting.then(|| [linear(d, d, &mut off), linear(n_rel, d, &mut off)]);
        let comma = commas.then(|| [linear(d, d, &mut off), linear(1, d, &mut off)]);
        if b.len() != off {
            return None;
        }
        let owned = (!(b.as_ptr() as usize).is_multiple_of(4)).then(|| {
            let mut v = vec![0u32; b.len().div_ceil(4)];
            // SAFETY: `v` holds at least `b.len()` bytes; u32 has no invalid
            // bit patterns.
            unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, b.len()) }
                .copy_from_slice(b);
            v
        });
        Some(Parser {
            data,
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
            wait_rel,
            comma,
            rel,
            marks,
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

    /// A relation's number (None: the parser doesn't know it).
    pub fn relation(&self, name: &str) -> Option<usize> {
        self.relations.iter().position(|r| r == name)
    }

    fn bytes(&self) -> &[u8] {
        match &self.owned {
            // SAFETY: the copy holds the blob's bytes (and up to 3 more).
            Some(o) => unsafe { std::slice::from_raw_parts(o.as_ptr() as *const u8, o.len() * 4) },
            None => self.data.as_ref(),
        }
    }

    /// `n` floats at byte offset `at`.
    fn floats(&self, at: usize, n: usize) -> &[f32] {
        let b = &self.bytes()[at..at + 4 * n];
        // SAFETY: checked in `new`: little-endian, the blob 4-aligned (or
        // copied so), every offset a multiple of 4 and within it; every bit
        // pattern is a valid f32; the bytes live as long as `self`.
        unsafe { std::slice::from_raw_parts(b.as_ptr() as *const f32, n) }
    }

    /// Row `r` of a matrix, as f32.
    fn row(&self, m: &Mat, r: usize, out: &mut [f32]) {
        match m.scales {
            None => out.copy_from_slice(self.floats(m.at + 4 * r * m.cols, m.cols)),
            Some(s) => {
                let scale = self.floats(s + 4 * r, 1)[0];
                let q = &self.bytes()[m.at + r * m.cols..][..m.cols];
                for (o, &v) in out.iter_mut().zip(q) {
                    *o = scale * v as i8 as f32;
                }
            }
        }
    }

    /// Column `k` of a matrix of `out.len()` rows, as f32.
    fn column(&self, m: &Mat, k: usize, out: &mut [f32]) {
        for (r, o) in out.iter_mut().enumerate() {
            *o = match m.scales {
                None => self.floats(m.at + 4 * (r * m.cols + k), 1)[0],
                Some(s) => {
                    let q = self.bytes()[m.at + r * m.cols + k] as i8 as f32;
                    self.floats(s + 4 * r, 1)[0] * q
                }
            };
        }
    }

    fn linear(&self, (m, b): &Linear, x: &[f32], out: &mut [f32]) {
        let n = x.len();
        let bias = self.floats(*b, out.len());
        match m.scales {
            None => {
                let ws = self.floats(m.at, out.len() * n);
                for (o, y) in out.iter_mut().enumerate() {
                    let row = &ws[o * n..][..n];
                    *y = bias[o] + row.iter().zip(x).map(|(a, b)| a * b).sum::<f32>();
                }
            }
            Some(s) => {
                let scales = self.floats(s, out.len());
                let q = &self.bytes()[m.at..][..out.len() * n];
                for (o, y) in out.iter_mut().enumerate() {
                    let row = &q[o * n..][..n];
                    let dot = row.iter().zip(x).map(|(&a, b)| a as i8 as f32 * b).sum::<f32>();
                    *y = bias[o] + scales[o] * dot;
                }
            }
        }
    }

    fn layer_norm(&self, (g, b): (usize, usize), x: &[f32]) -> Vec<f32> {
        let (g, b) = (self.floats(g, x.len()), self.floats(b, x.len()));
        let n = x.len() as f32;
        let mean = x.iter().sum::<f32>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + 1e-5).sqrt();
        x.iter()
            .enumerate()
            .map(|(i, v)| (v - mean) * inv * g[i] + b[i])
            .collect()
    }

    fn mlp(&self, l: &[Linear; 2], x: &[f32]) -> Vec<f32> {
        let mut a = vec![0f32; self.d];
        self.linear(&l[0], x, &mut a);
        a.iter_mut().for_each(|v| *v = v.max(0.0));
        let mut o = vec![0f32; self.d];
        self.linear(&l[1], &a, &mut o);
        o
    }

    /// [`Parser::parse`], for a causal parser from the sentences read lately:
    /// only the words after the longest start in common are read (their
    /// graph rows are the same — a word sees only the words before it).
    pub fn parse_cached<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, words: &[Word]) -> Graph {
        let words = &words[words.len().saturating_sub(self.places())..];
        if !self.causal {
            return std::sync::Arc::new(self.parse_full(lemmas, words));
        }
        let common = |r: &Read| r.words.iter().zip(words).take_while(|(a, b)| a == b).count();
        let mut read = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            match (0..cache.len()).max_by_key(|&i| common(&cache[i])) {
                Some(i) if common(&cache[i]) == words.len() && cache[i].words.len() == words.len() => {
                    let r = cache.remove(i);
                    let g = std::sync::Arc::new(self.graph_of(&r));
                    cache.insert(0, r);
                    return g;
                }
                Some(i) => {
                    let n = common(&cache[i]);
                    let r = &cache[i];
                    Read {
                        words: r.words[..n].to_vec(),
                        kv: r.kv[..n + 1].to_vec(),
                        heads: r.heads[..n + 1].to_vec(),
                        rows: r.rows[..n].to_vec(),
                        waiting: r.waiting[..r.waiting.len().min(n)].to_vec(),
                    }
                }
                None => Read::default(),
            }
        };
        if read.kv.is_empty() {
            let x = self.root_input();
            self.step(&mut read, x);
        }
        for w in &words[read.words.len()..] {
            let x = self.word_input(lemmas, w, read.words.len() + 1);
            read.words.push(*w);
            self.step(&mut read, x);
        }
        let g = std::sync::Arc::new(self.graph_of(&read));
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(0, read);
        cache.truncate(CACHED);
        g
    }

    /// The root's input.
    fn root_input(&self) -> Vec<f32> {
        let mut x = vec![0f32; self.d];
        self.row(&self.pos, 0, &mut x);
        x.iter_mut()
            .zip(self.floats(self.root, self.d))
            .for_each(|(a, b)| *a += b);
        x
    }

    /// A word's input at `place` (the root's is 0): its lemma's vector, its
    /// class, its grammemes, its place.
    fn word_input<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, w: &Word, place: usize) -> Vec<f32> {
        let d = self.d;
        let mut v = vec![0f32; self.dv];
        lemmas.ctx_vector(w.lemma, &mut v);
        let mut x = vec![0f32; d];
        self.linear(&self.lemma, &v, &mut x);
        let bag: Vec<f32> = self
            .grams
            .iter()
            .map(|&g| if w.grammemes & g != 0 { 1.0 } else { 0.0 })
            .collect();
        let mut gx = vec![0f32; d];
        self.linear(&self.gram, &bag, &mut gx);
        if let Some(m) = &self.marks {
            // The marks' projection has no bias: a column per kind.
            let mut col = vec![0f32; d];
            for k in (0..MARK_KINDS.len()).filter(|k| w.marks & 1 << k != 0) {
                self.column(m, k, &mut col);
                x.iter_mut().zip(&col).for_each(|(a, b)| *a += b);
            }
        }
        let c = (w.class as usize).min(self.classes - 1);
        let (mut cx, mut px) = (vec![0f32; d], vec![0f32; d]);
        self.row(&self.class, c, &mut cx);
        self.row(&self.pos, place, &mut px);
        for (i, xi) in x.iter_mut().enumerate() {
            *xi += gx[i] + cx[i] + px[i];
        }
        x
    }

    /// Read one more token (causal) with input `x`: through the layers,
    /// seeing the tokens read before; a word gets its row of chances.
    fn step(&self, read: &mut Read, mut x: Vec<f32>) {
        let d = self.d;
        let hd_n = d / self.heads;
        let scale = 1.0 / (hd_n as f32).sqrt();
        let t = read.kv.len();
        let mut kv = vec![0f32; self.layers.len() * 2 * d];
        for (l, layer) in self.layers.iter().enumerate() {
            let normed = self.layer_norm(layer.norm1, &x);
            let mut qkv = vec![0f32; 3 * d];
            self.linear(&layer.in_proj, &normed, &mut qkv);
            kv[l * 2 * d..(l + 1) * 2 * d].copy_from_slice(&qkv[d..]);
            let key_value = |j: usize| -> &[f32] {
                let row = if j == t { &kv } else { &read.kv[j] };
                &row[l * 2 * d..(l + 1) * 2 * d]
            };
            let mut att = vec![0f32; d];
            for h in 0..self.heads {
                let q = &qkv[h * hd_n..(h + 1) * hd_n];
                let s: Vec<f32> = (0..=t)
                    .map(|j| {
                        let k = &key_value(j)[h * hd_n..(h + 1) * hd_n];
                        q.iter().zip(k).map(|(a, b)| a * b).sum::<f32>() * scale
                    })
                    .collect();
                let m = s.iter().copied().fold(f32::MIN, f32::max);
                let e: Vec<f32> = s.iter().map(|v| (v - m).exp()).collect();
                let z: f32 = e.iter().sum();
                for (j, w) in e.iter().enumerate() {
                    let val = &key_value(j)[d + h * hd_n..d + (h + 1) * hd_n];
                    for (i, vv) in val.iter().enumerate() {
                        att[h * hd_n + i] += w / z * vv;
                    }
                }
            }
            let mut o = vec![0f32; d];
            self.linear(&layer.out_proj, &att, &mut o);
            x.iter_mut().zip(&o).for_each(|(a, b)| *a += b);
            let n2 = self.layer_norm(layer.norm2, &x);
            let mut hid = vec![0f32; self.ff];
            self.linear(&layer.ff1, &n2, &mut hid);
            hid.iter_mut().for_each(|v| *v = v.max(0.0));
            let mut o2 = vec![0f32; d];
            self.linear(&layer.ff2, &hid, &mut o2);
            x.iter_mut().zip(&o2).for_each(|(a, b)| *a += b);
        }
        read.kv.push(kv);
        let h = self.layer_norm(self.norm, &x);
        read.heads.push(self.mlp(&self.hd, &h));
        if t == 0 {
            return;
        }
        if let Some(w) = &self.wait_rel {
            let mut a = vec![0f32; d];
            self.linear(&w[0], &h, &mut a);
            a.iter_mut().for_each(|v| *v = v.max(0.0));
            let mut logits = vec![0f32; self.relations.len()];
            self.linear(&w[1], &a, &mut logits);
            read.waiting.push(softmax(&logits));
        }
        let dep = self.mlp(&self.dep, &h);
        let sd = (d as f32).sqrt();
        let dot = |b: &[f32]| dep.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / sd;
        let mut s: Vec<f32> = read.heads[..t].iter().map(|b| dot(b)).collect();
        s.push(dot(self.floats(self.later, d)));
        read.rows.push(softmax(&s));
    }

    /// A sentence read's graph, as [`Parser::parse`] gives it: each word's
    /// row over the root, all the words (nothing for itself and the words
    /// after it) and a word still to come.
    fn graph_of(&self, read: &Read) -> Sentence {
        let n = read.rows.len();
        let heads = read
            .rows
            .iter()
            .map(|r| {
                let (before, later) = r.split_at(r.len() - 1);
                let mut row = before.to_vec();
                row.resize(n + 1, 0.0);
                row.extend_from_slice(later);
                row
            })
            .collect();
        Sentence {
            heads,
            waiting: read.waiting.clone(),
            relations: Vec::new(),
            relation_chances: Vec::new(),
            commas: Vec::new(),
        }
    }

    /// Each word's chances of its heads: for word i (in `words` order), a
    /// vector over the root (0), the words (1..=n, the word itself 0) and —
    /// causal — "a word still to come" (n + 1); a causal parser gives the
    /// words after i nothing. Only the last [`Parser::places`] words are read.
    pub fn parse<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, words: &[Word]) -> Vec<Vec<f32>> {
        let words = &words[words.len().saturating_sub(self.places())..];
        let hs = self.encode(lemmas, words);
        self.heads_of(&hs)
    }

    /// The whole sentence's graph (a parser of whole sentences): each word's
    /// chances of its heads, and its relation to the likeliest one with that
    /// relation's chance.
    pub fn parse_full<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, words: &[Word]) -> Sentence {
        let words = &words[words.len().saturating_sub(self.places())..];
        let hs = self.encode(lemmas, words);
        let heads = self.heads_of(&hs);
        let relation_chances = heads
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let h = (0..row.len())
                    .max_by(|&a, &b| row[a].total_cmp(&row[b]))
                    .unwrap_or(0)
                    .min(hs.len() - 1);
                let x: Vec<f32> = hs[i + 1].iter().chain(&hs[h]).copied().collect();
                let mut a = vec![0f32; self.d];
                self.linear(&self.rel[0], &x, &mut a);
                a.iter_mut().for_each(|v| *v = v.max(0.0));
                let mut logits = vec![0f32; self.relations.len()];
                self.linear(&self.rel[1], &a, &mut logits);
                softmax(&logits)
            })
            .collect::<Vec<_>>();
        let relations = relation_chances
            .iter()
            .map(|p| {
                let k = (0..p.len()).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(0);
                (k, p[k])
            })
            .collect();
        let commas = match &self.comma {
            Some(c) => (1..hs.len())
                .map(|i| {
                    let mut a = vec![0f32; self.d];
                    self.linear(&c[0], &hs[i], &mut a);
                    a.iter_mut().for_each(|v| *v = v.max(0.0));
                    let mut o = [0f32; 1];
                    self.linear(&c[1], &a, &mut o);
                    1.0 / (1.0 + (-o[0]).exp())
                })
                .collect(),
            None => Vec::new(),
        };
        Sentence {
            heads,
            waiting: Vec::new(),
            relations,
            relation_chances,
            commas,
        }
    }

    /// The tokens' states after the layers, normed: the root, then the words.
    fn encode<L: AsRef<[u8]>>(&self, lemmas: &Lemmas<L>, words: &[Word]) -> Vec<Vec<f32>> {
        let (d, n) = (self.d, words.len());
        let mut xs: Vec<Vec<f32>> = Vec::with_capacity(n + 1);
        xs.push(self.root_input());
        for (j, w) in words.iter().enumerate() {
            xs.push(self.word_input(lemmas, w, j + 1));
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
                    self.linear(&layer.in_proj, x, &mut o);
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
                self.linear(&layer.out_proj, &att, &mut o);
                let mut y: Vec<f32> = x.iter().zip(&o).map(|(a, b)| a + b).collect();
                let n2 = self.layer_norm(layer.norm2, &y);
                let mut hid = vec![0f32; self.ff];
                self.linear(&layer.ff1, &n2, &mut hid);
                hid.iter_mut().for_each(|v| *v = v.max(0.0));
                let mut o2 = vec![0f32; d];
                self.linear(&layer.ff2, &hid, &mut o2);
                y.iter_mut().zip(&o2).for_each(|(a, b)| *a += b);
                next.push(y);
            }
            xs = next;
        }
        xs.iter().map(|x| self.layer_norm(self.norm, x)).collect()
    }

    /// Each word's chances of its heads, from the tokens' states.
    fn heads_of(&self, hs: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let (d, n) = (self.d, hs.len() - 1);
        let deps: Vec<Vec<f32>> = hs.iter().map(|h| self.mlp(&self.dep, h)).collect();
        let heads: Vec<Vec<f32>> = hs.iter().map(|h| self.mlp(&self.hd, h)).collect();
        let sd = (d as f32).sqrt();
        let later = self.floats(self.later, d);
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

fn softmax(s: &[f32]) -> Vec<f32> {
    let m = s.iter().copied().fold(f32::MIN, f32::max);
    let e: Vec<f32> = s.iter().map(|v| (v - m).exp()).collect();
    let z: f32 = e.iter().sum();
    e.iter().map(|v| v / z).collect()
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
