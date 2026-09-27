//! Word-gesture decoding ("swipe typing"): a finger path drawn across the
//! keys, read as the word whose key-to-key polyline it follows most closely,
//! weighed with the word's prior.
//!
//! 1. The path is resampled to [`M`] points evenly spaced along its length.
//! 2. Candidates come from a walk of the dictionary FST: each letter's key
//!    must lie near the path at or after the previous letter's spot (a
//!    doubled letter stays put), the first near the start and the last near
//!    the end — a branch no word can finish is dropped at once.
//! 3. Each candidate's ideal path (straight lines between its key centers,
//!    resampled the same way) is compared with the drawn one: point by point
//!    where it lies ("location"), and after both are centered and scaled
//!    ("shape", which forgives a gesture drawn a little off or small — say
//!    with one thumb). With the prior, that is the candidate's cost; its
//!    `edit` is the geometric part, so [`Engine::rerank`] can add context.
//! 4. When the path comes with its times, the finger's pace is a hint too
//!    ([`Pace`]): where it stopped mid-way there is likely a letter (a word
//!    with none there pays), where it flew past a key in a straight line
//!    likely not (a letter placed there pays a little).

use fst::raw::{Fst, Node};

use crate::alphabet;
use crate::dict::DictFormat;
use crate::engine::{Candidate, Engine};
use crate::keyboard::N;

/// Points a path is resampled to.
const M: usize = 40;
/// A letter's key "is on the path" within this many key widths of a point.
const NEAR: f32 = 1.1;
/// The first letter must be near one of the first points, the last near one
/// of the last points…
const END_SLACK: usize = 4;
/// …and within this many key widths of either end, however few points that
/// is: on a short gesture 4 points are less than smoothing moves the ends
/// («мне» drawn with the corners 0.3 key off: right 170 times in 200, 145
/// before; whole sentences 94.5% of the words, 93.6% before).
const END_SLACK_KEYS: f32 = 1.0;
/// Candidates the walk keeps for the full comparison, and FST nodes it may
/// visit.
const KEEP: usize = 60;
const MAX_NODES: usize = 150_000;

type Pt = (f32, f32);

/// A stop counts when the finger lingered this many times longer than the
/// gesture's usual pace per unit of length; a pass counts as quick at this
/// fraction of it.
const PAUSE: f32 = 2.0;
const QUICK: f32 = 0.6;
/// A stop this many resampled points from a letter's is that letter's.
const PAUSE_REACH: usize = 2;
/// Most a single stop or a single quick pass can cost (nats).
const PACE_MAX: f32 = 1.5;

/// Points evenly spaced along the path (by length), `m` of them.
fn resample(path: &[Pt], m: usize) -> Vec<Pt> {
    if path.len() < 2 {
        return vec![path.first().copied().unwrap_or((0.0, 0.0)); m];
    }
    let seg: Vec<f32> = path
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .collect();
    let total: f32 = seg.iter().sum();
    if total <= f32::EPSILON {
        return vec![path[0]; m];
    }
    let step = total / (m - 1) as f32;
    let mut out = Vec::with_capacity(m);
    let (mut i, mut done) = (0usize, 0.0f32);
    for k in 0..m {
        let at = (k as f32 * step).min(total);
        while i + 1 < seg.len() && done + seg[i] < at {
            done += seg[i];
            i += 1;
        }
        let f = if seg[i] > 0.0 {
            ((at - done) / seg[i]).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = (path[i], path[i + 1]);
        out.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
    }
    out
}

fn mean_dist(a: &[Pt], b: &[Pt]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1))
        .sum::<f32>()
        / a.len().max(1) as f32
}

/// Centered on its centroid and scaled to unit size (the larger side of the
/// bounding box; a dot stays a dot).
fn normalized(pts: &[Pt]) -> Vec<Pt> {
    let n = pts.len().max(1) as f32;
    let (cx, cy) = pts
        .iter()
        .fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
    let (mut w, mut h) = (0.0f32, 0.0f32);
    for p in pts {
        w = w.max((p.0 - cx).abs());
        h = h.max((p.1 - cy).abs());
    }
    let s = w.max(h).max(1e-3);
    pts.iter()
        .map(|p| ((p.0 - cx) / s, (p.1 - cy) / s))
        .collect()
}

/// How slowly the finger went along the path: for each resampled point, the
/// time it spent on the stretch of path around it (half a step either side;
/// a still finger's time all at its spot) per unit of length, relative to
/// the gesture's median — from which what a stop no letter explains, and a
/// letter on a quick pass, cost.
struct Pace {
    /// `stops[k]`: the cost of the stops at points `< k` (prefix sums).
    stops: Vec<f32>,
    /// The cost of a letter at each point.
    quick: Vec<f32>,
}

impl Pace {
    fn new(path: &[Pt], times: &[f32], m: usize) -> Option<Self> {
        if times.len() != path.len() || path.len() < 2 || m < 3 {
            return None;
        }
        let seg: Vec<f32> = path
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .collect();
        let total: f32 = seg.iter().sum();
        if total <= f32::EPSILON || times[times.len() - 1] <= times[0] {
            return None;
        }
        // The time spent before length `a` along the path.
        let spent = |a: f32| {
            let (mut done, mut t) = (0.0f32, 0.0f32);
            for (i, &s) in seg.iter().enumerate() {
                if done >= a {
                    break;
                }
                let dt = (times[i + 1] - times[i]).max(0.0);
                t += if done + s <= a {
                    dt
                } else {
                    dt * (a - done) / s
                };
                done += s;
            }
            t
        };
        let step = total / (m - 1) as f32;
        let bounds: Vec<f32> = (0..=m)
            .map(|i| match i {
                0 => 0.0,
                i if i == m => f32::INFINITY,
                i => spent((i as f32 - 0.5) * step),
            })
            .collect();
        let rate: Vec<f32> = (0..m)
            .map(|k| {
                let width = if k == 0 || k == m - 1 {
                    step / 2.0
                } else {
                    step
                };
                (bounds[k + 1] - bounds[k]) / width
            })
            .collect();
        let mut inner: Vec<f32> = rate[1..m - 1].to_vec();
        inner.sort_by(f32::total_cmp);
        let median = inner[inner.len() / 2];
        if median <= 0.0 {
            return None;
        }
        let slow: Vec<f32> = rate.iter().map(|r| r / median).collect();
        let mut stops = vec![0.0f32; m + 1];
        for k in 0..m {
            // A stop: slower than both neighbours, well past the usual pace
            // (the ends, where every gesture starts and stops, don't count).
            let peak = k > 0 && k + 1 < m && slow[k] >= slow[k - 1] && slow[k] >= slow[k + 1];
            let cost = if peak {
                (slow[k] / PAUSE).ln().clamp(0.0, PACE_MAX)
            } else {
                0.0
            };
            stops[k + 1] = stops[k] + cost;
        }
        let quick = slow
            .iter()
            .map(|&s| (QUICK / s.max(1e-3)).ln().clamp(0.0, PACE_MAX))
            .collect();
        Some(Pace { stops, quick })
    }

    /// A letter at point `at` after one at `j`: its quick pass, and the
    /// stops between the two that neither explains.
    fn cost(&self, j: usize, at: usize) -> f32 {
        let (lo, hi) = (j + PAUSE_REACH + 1, at.saturating_sub(PAUSE_REACH));
        let stops = if lo < hi {
            self.stops[hi] - self.stops[lo]
        } else {
            0.0
        };
        stops + self.quick[at]
    }
}

/// A gesture being decoded: the resampled path and, per letter, the first
/// path point at or after each index where that letter's key is near.
struct Gesture {
    pts: Vec<Pt>,
    shape: Vec<Pt>,
    pos: [Option<Pt>; N],
    /// `next[c * (M + 1) + j]`: first `j' ≥ j` with key `c` near point `j'`
    /// (`M`: none).
    next: Vec<u8>,
    key_w: f32,
    pace: Option<Pace>,
    /// How many points at either end the first and last letter may lie
    /// within (see [`END_SLACK_KEYS`]).
    slack: usize,
}

impl Gesture {
    fn new(path: &[Pt], times: Option<&[f32]>, keys: &[(char, f32, f32)], key_w: f32) -> Self {
        let pts = resample(path, M);
        let mut pos = [None; N];
        for &(c, x, y) in keys {
            if let Some(id) = alphabet::char_to_id(c.to_lowercase().next().unwrap_or(c)) {
                pos[id as usize] = Some((x, y));
            }
        }
        let mut next = vec![M as u8; N * (M + 1)];
        let r = NEAR * key_w;
        for (c, p) in pos.iter().enumerate() {
            let Some(&(kx, ky)) = p.as_ref() else {
                continue;
            };
            for j in (0..M).rev() {
                let (x, y) = pts[j];
                next[c * (M + 1) + j] = if (x - kx).hypot(y - ky) <= r {
                    j as u8
                } else {
                    next[c * (M + 1) + j + 1]
                };
            }
        }
        let len: f32 = pts
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        let step = (len / (M - 1) as f32).max(1e-3);
        let slack = ((END_SLACK_KEYS * key_w / step).ceil() as usize).clamp(END_SLACK, M / 3);
        Gesture {
            shape: normalized(&pts),
            pts,
            pos,
            next,
            key_w,
            pace: times.and_then(|t| Pace::new(path, t, M)),
            slack,
        }
    }

    /// First path point at or after `j` near letter `c`'s key.
    fn near_from(&self, c: u8, j: usize) -> Option<usize> {
        let k = self.next[c as usize * (M + 1) + j.min(M)] as usize;
        (k < M).then_some(k)
    }

    /// The word's ideal path, resampled: straight lines between its keys.
    fn ideal(&self, ids: &[u8]) -> Option<Vec<Pt>> {
        let mut corners: Vec<Pt> = Vec::with_capacity(ids.len());
        for &c in ids {
            let p = self.pos[c as usize]?;
            if corners.last() != Some(&p) {
                corners.push(p);
            }
        }
        Some(resample(&corners, M))
    }

    /// How far the drawn path is from the word's: location (squared mean
    /// distance in key widths) and shape (in units of the gesture's size),
    /// weighted.
    fn distance(&self, ids: &[u8], w_location: f32, w_shape: f32) -> Option<f32> {
        let ideal = self.ideal(ids)?;
        let loc = mean_dist(&self.pts, &ideal) / self.key_w;
        let shape = mean_dist(&self.shape, &normalized(&ideal));
        Some(w_location * loc * loc + w_shape * shape * shape)
    }
}

/// The walk's best candidates so far, cheapest first: (cheap cost, word
/// ids, prior, pace cost); the k-th cost bounds the search.
struct Walk {
    k: usize,
    best: Vec<(f32, Vec<u8>, u64, f32)>,
    nodes: usize,
    w_walk: f32,
    w_prior: f32,
    w_pace: f32,
    /// How the FST walked now reads its values.
    format: DictFormat,
}

impl Walk {
    fn bound(&self) -> f32 {
        if self.best.len() < self.k {
            f32::INFINITY
        } else {
            self.best[self.k - 1].0
        }
    }

    fn offer(&mut self, cost: f32, ids: &[u8], prior: u64, pace: f32) {
        if cost >= self.bound() || self.best.iter().any(|b| b.1 == ids) {
            return;
        }
        let i = self.best.partition_point(|b| b.0 <= cost);
        self.best.insert(i, (cost, ids.to_vec(), prior, pace));
        self.best.truncate(self.k);
    }
}

impl<D: AsRef<[u8]>> Engine<D> {
    /// Decode a gesture: `path` is where the finger went (any units),
    /// `keys` each letter's key center in the same units, `key_w` a key's
    /// width in them. Candidates cheapest first (at most `top_k`); `edit` is
    /// the geometric part of the cost.
    pub fn gesture(&self, path: &[Pt], keys: &[(char, f32, f32)], key_w: f32) -> Vec<Candidate> {
        self.gesture_timed(path, None, keys, key_w)
    }

    /// [`Engine::gesture`] with the time of each path point (ms): where the
    /// finger stopped or flew weighs the letters too (see [`Pace`]).
    pub fn gesture_timed(
        &self,
        path: &[Pt],
        times: Option<&[f32]>,
        keys: &[(char, f32, f32)],
        key_w: f32,
    ) -> Vec<Candidate> {
        if path.len() < 2 || key_w <= 0.0 {
            return Vec::new();
        }
        let cfg = &self.cfg;
        let times = times.filter(|_| cfg.gesture_pace > 0.0);
        let g = Gesture::new(path, times, keys, key_w);
        let mut w = Walk {
            k: KEEP,
            best: Vec::with_capacity(KEEP + 1),
            nodes: 0,
            w_walk: cfg.gesture_walk,
            w_prior: cfg.w_lm * cfg.prior_scale,
            w_pace: cfg.gesture_pace,
            format: self.format,
        };
        let mut ids = Vec::with_capacity(24);
        let fst = self.map.as_fst();
        walk(
            &g,
            fst,
            fst.root(),
            0,
            0,
            0,
            g.pts[0],
            0,
            (0.0, 0.0),
            &mut ids,
            &mut w,
        );
        if let Some(user) = &self.user {
            let fst = user.as_fst();
            w.format = DictFormat::PLAIN;
            walk(
                &g,
                fst,
                fst.root(),
                0,
                0,
                0,
                g.pts[0],
                0,
                (0.0, 0.0),
                &mut ids,
                &mut w,
            );
        }
        let mut out: Vec<Candidate> = w
            .best
            .iter()
            .filter_map(|(_, ids, prior, pace)| {
                let geo = g.distance(ids, cfg.gesture_location, cfg.gesture_shape)?
                    + cfg.gesture_pace * pace;
                Some(Candidate {
                    word: alphabet::ids_to_string(ids),
                    cost: geo + cfg.w_lm * *prior as f32 * cfg.prior_scale,
                    edit: geo,
                })
            })
            .collect();
        out.sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.word.cmp(&b.word)));
        out.truncate(cfg.top_k);
        out
    }
}

/// A letter the walk may take next: its (geometric, pace) cost so far, its
/// point, id and key, the FST outputs so far and its node.
type Kid = ((f32, f32), usize, u8, Pt, u64, fst::raw::CompiledAddr);

/// Distance from `p` to the segment `a`–`b`.
fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// Branch and bound over the words whose letters lie along the path in order
/// (see the module docs). Each letter sits at the path point nearest its key
/// (after the previous letter's); its cost is that distance², plus how far
/// the path strays from the straight line on the way there (a skipped
/// corner is a missing letter), all in key widths. `j`, `last`, `from`: the
/// previous letter's point, id and key; `acc`: FST outputs so far (a lower
/// bound of the prior below); `cost`: the geometric and the pace cost so
/// far (both only grow, so the bound holds).
#[allow(clippy::too_many_arguments)]
fn walk<F: AsRef<[u8]>>(
    g: &Gesture,
    fst: &Fst<F>,
    node: Node,
    depth: usize,
    j: usize,
    last: u8,
    from: Pt,
    acc: u64,
    cost: (f32, f32),
    ids: &mut Vec<u8>,
    w: &mut Walk,
) {
    if w.nodes >= MAX_NODES {
        return;
    }
    w.nodes += 1;
    let d = |p: Pt, q: Pt| (p.0 - q.0).hypot(p.1 - q.1) / g.key_w;
    let mut kids: Vec<Kid> = Vec::new();
    for t in node.transitions() {
        let c = t.inp;
        let Some(key) = g.pos[c as usize] else {
            continue;
        };
        let (at, step, paced) = if depth > 0 && c == last {
            (j, 0.0, 0.0) // a doubled letter: the finger doesn't move
        } else {
            let Some(mut at) = g.near_from(c, if depth == 0 { 0 } else { j }) else {
                continue;
            };
            if depth == 0 && at > g.slack {
                continue; // the word must start where the gesture starts
            }
            while at + 1 < M && d(g.pts[at + 1], key) < d(g.pts[at], key) {
                at += 1;
            }
            let lo = if depth == 0 { 0 } else { j };
            let stray = (lo..at)
                .map(|k| {
                    if depth == 0 {
                        d(g.pts[k], key)
                    } else {
                        seg_dist(g.pts[k], from, key) / g.key_w
                    }
                })
                .fold(0.0f32, f32::max);
            let near = d(g.pts[at], key);
            let paced = match &g.pace {
                Some(p) if depth > 0 => p.cost(j, at),
                _ => 0.0,
            };
            (at, near * near + stray * stray, paced)
        };
        let acc = acc + t.out.value();
        let c_cost = (cost.0 + step, cost.1 + paced);
        let bound = w.w_prior * w.format.prior(acc) as f32;
        if w.w_walk * c_cost.0 + w.w_pace * c_cost.1 + bound >= w.bound() {
            continue;
        }
        kids.push((c_cost, at, c, key, acc, t.addr));
    }
    kids.sort_by(|a, b| {
        (w.w_walk * a.0 .0 + w.w_pace * a.0 .1).total_cmp(&(w.w_walk * b.0 .0 + w.w_pace * b.0 .1))
    });
    for (c_cost, at, c, key, acc, addr) in kids {
        ids.push(c);
        let child = fst.node(addr);
        if child.is_final() && depth >= 1 && at + g.slack >= M - 1 {
            // The path after the last letter should stay on its key.
            let tail = (at..M).map(|k| d(g.pts[k], key)).fold(0.0f32, f32::max);
            let prior = w.format.prior(acc + child.final_output().value());
            let total = w.w_walk * (c_cost.0 + tail * tail)
                + w.w_pace * c_cost.1
                + w.w_prior * prior as f32;
            w.offer(total, ids, prior, c_cost.1);
        }
        walk(g, fst, child, depth + 1, at, c, key, acc, c_cost, ids, w);
        ids.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use fst::Map;

    /// A ЙЦУКЕН grid: 11 keys a row, rows 1.35 keys apart, 100 units a key.
    fn keys() -> Vec<(char, f32, f32)> {
        let rows = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"];
        let mut out = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let shift = [0.0, 0.0, 100.0][r];
            for (i, c) in row.chars().enumerate() {
                out.push((c, shift + i as f32 * 100.0 + 50.0, r as f32 * 135.0 + 67.0));
            }
        }
        out
    }

    fn center(c: char) -> Pt {
        let k = keys().into_iter().find(|k| k.0 == c).unwrap();
        (k.1, k.2)
    }

    /// A drawn gesture: through the word's keys with the corners off by
    /// `(dx, dy)` alternately, points every 10 units.
    fn draw(word: &str, wobble: f32) -> Vec<Pt> {
        let corners: Vec<Pt> = word
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let (x, y) = center(c);
                let s = if i % 2 == 0 { wobble } else { -wobble };
                (x + s, y - s * 0.5)
            })
            .collect();
        let mut out = vec![corners[0]];
        for w in corners.windows(2) {
            let n = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / 10.0)
                .ceil()
                .max(1.0) as usize;
            for k in 1..=n {
                let f = k as f32 / n as f32;
                out.push((
                    w[0].0 + (w[1].0 - w[0].0) * f,
                    w[0].1 + (w[1].1 - w[0].1) * f,
                ));
            }
        }
        out
    }

    fn engine(words: &[(&str, u64)]) -> Engine<Vec<u8>> {
        let mut kv: Vec<(Vec<u8>, u64)> = words
            .iter()
            .map(|(w, v)| {
                (
                    w.chars()
                        .map(|c| alphabet::char_to_id(c).unwrap())
                        .collect(),
                    *v,
                )
            })
            .collect();
        kv.sort();
        Engine::new(Map::from_iter(kv).unwrap(), Config::default())
    }

    #[test]
    fn a_gesture_reads_as_its_word() {
        let e = engine(&[
            ("привет", 5000),
            ("приват", 9000),
            ("пет", 6000),
            ("пора", 5000),
        ]);
        let got = e.gesture(&draw("привет", 25.0), &keys(), 100.0);
        assert_eq!(
            got.first().map(|c| c.word.as_str()),
            Some("привет"),
            "{got:?}"
        );
    }

    #[test]
    fn doubled_letters_and_short_words() {
        let e = engine(&[("касса", 7000), ("каса", 9000), ("да", 3000), ("до", 4000)]);
        let got = e.gesture(&draw("каса", 10.0), &keys(), 100.0);
        assert!(got.iter().take(2).any(|c| c.word == "касса"), "{got:?}");
        let got = e.gesture(&draw("да", 10.0), &keys(), 100.0);
        assert_eq!(got.first().map(|c| c.word.as_str()), Some("да"), "{got:?}");
    }

    #[test]
    fn the_start_and_end_must_fit() {
        let e = engine(&[("привет", 5000), ("ветер", 3000)]);
        // «ветер» lies along «привет» but starts elsewhere.
        let got = e.gesture(&draw("привет", 0.0), &keys(), 100.0);
        assert!(got.iter().all(|c| c.word != "ветер"), "{got:?}");
    }

    /// The times of a drawn path: an even pace, with the finger resting
    /// `rest` ms at the point nearest `stop`.
    fn timed(path: &[Pt], stop: Option<Pt>, rest: f32) -> Vec<f32> {
        let at = stop.map(|s| {
            (0..path.len())
                .min_by(|&a, &b| {
                    let d = |p: Pt| (p.0 - s.0).hypot(p.1 - s.1);
                    d(path[a]).total_cmp(&d(path[b]))
                })
                .unwrap()
        });
        let mut t = 0.0;
        path.iter()
            .enumerate()
            .map(|(i, _)| {
                t += 8.0;
                if Some(i) == at {
                    t += rest;
                }
                t
            })
            .collect()
    }

    #[test]
    fn a_stop_on_the_way_is_a_letter() {
        // «пол» and «пл» draw the same line (о lies between п and л).
        let e = engine(&[("пол", 6000), ("пл", 4000)]);
        let path = draw("пл", 0.0);
        let plain = e.gesture(&path, &keys(), 100.0);
        assert_eq!(
            plain.first().map(|c| c.word.as_str()),
            Some("пл"),
            "{plain:?}"
        );
        // Resting on о: the word with о in it.
        let t = timed(&path, Some(center('о')), 300.0);
        let got = e.gesture_timed(&path, Some(&t), &keys(), 100.0);
        assert_eq!(got.first().map(|c| c.word.as_str()), Some("пол"), "{got:?}");
        // An even pace changes nothing.
        let t = timed(&path, None, 0.0);
        let even = e.gesture_timed(&path, Some(&t), &keys(), 100.0);
        assert_eq!(
            even.first().map(|c| c.word.as_str()),
            Some("пл"),
            "{even:?}"
        );
    }

    #[test]
    fn resampling_is_even() {
        let pts = resample(&[(0.0, 0.0), (10.0, 0.0), (10.0, 30.0)], 5);
        assert_eq!(pts[0], (0.0, 0.0));
        assert_eq!(pts[4], (10.0, 30.0));
        assert!((pts[1].0 - 10.0).abs() < 1e-4 && pts[1].1.abs() < 1e-4);
        assert!((pts[2].1 - 10.0).abs() < 1e-4);
    }
}
