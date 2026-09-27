//! What the simulation examples share: a seeded random source and a finger
//! drawing a word across the keys.

/// xorshift64* — deterministic, no dependencies.
pub struct Rng(pub u64);

impl Rng {
    pub fn unit(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn gauss(&mut self) -> f32 {
        let (u, v) = (self.unit().max(1e-7), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

/// How the finger's pace is drawn: not at all, stopping only where the way
/// turns, or on every letter.
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Pace {
    None,
    Corners,
    Letters,
}

/// A word drawn as a finger would (`keys`: each letter's key center): the
/// corners missed by `sigma` key widths (Gaussian), points every 8 px, cut
/// by a moving average — and the time of each point (ms from the landing),
/// unless `pace` is none. None when a letter has no key.
#[allow(clippy::type_complexity)]
pub fn draw(
    word: &str,
    keys: &[(char, f32, f32)],
    key_w: f32,
    sigma: f32,
    pace: Pace,
    rng: &mut Rng,
) -> Option<(Vec<(f32, f32)>, Option<Vec<f32>>)> {
    let corners: Vec<(f32, f32)> = word
        .chars()
        .map(|c| keys.iter().find(|k| k.0 == c))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .map(|&(_, x, y)| {
            (
                x + rng.gauss() * sigma * key_w,
                y + rng.gauss() * sigma * key_w,
            )
        })
        .collect();
    let mut pts = vec![*corners.first()?];
    let mut at_corner = vec![0usize];
    for w in corners.windows(2) {
        let d = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        let steps = (d / 8.0).ceil().max(1.0) as usize;
        for s in 1..=steps {
            let f = s as f32 / steps as f32;
            pts.push((
                w[0].0 + (w[1].0 - w[0].0) * f,
                w[0].1 + (w[1].1 - w[0].1) * f,
            ));
        }
        at_corner.push(pts.len() - 1);
    }
    let times = match pace {
        Pace::None => None,
        _ => Some(stroke_times(
            &pts,
            &at_corner,
            pace == Pace::Letters,
            key_w,
            rng,
        )),
    };
    let smooth = (0..pts.len())
        .map(|i| {
            let lo = i.saturating_sub(3);
            let hi = (i + 4).min(pts.len());
            let k = (hi - lo) as f32;
            pts[lo..hi]
                .iter()
                .fold((0.0, 0.0), |a, p| (a.0 + p.0 / k, a.1 + p.1 / k))
        })
        .collect();
    Some((smooth, times))
}

/// The times of a drawn path (ms): strokes between stops — every letter, or
/// only where the way turns (and the ends) — each taking a Fitts-like time
/// with a minimum-jerk profile, and a short rest at each stop.
fn stroke_times(
    pts: &[(f32, f32)],
    corners: &[usize],
    every: bool,
    key_w: f32,
    rng: &mut Rng,
) -> Vec<f32> {
    let turn = |i: usize| {
        let (a, b, c) = (pts[corners[i - 1]], pts[corners[i]], pts[corners[i + 1]]);
        let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
        let (nu, nv) = (u.0.hypot(u.1), v.0.hypot(v.1));
        if nu < 1.0 || nv < 1.0 {
            return true; // a doubled letter: the finger stays
        }
        (u.0 * v.0 + u.1 * v.1) / (nu * nv) < 0.87 // over 30°
    };
    let mut stops = vec![corners[0]];
    for (i, &c) in corners
        .iter()
        .enumerate()
        .take(corners.len().saturating_sub(1))
        .skip(1)
    {
        if every || turn(i) {
            stops.push(c);
        }
    }
    stops.push(*corners.last().unwrap());
    stops.dedup();
    let mut times = vec![0.0f32; pts.len()];
    let mut t = 40.0 + 30.0 * rng.unit(); // the finger lands and settles
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len: Vec<f32> = (a..b)
            .map(|i| (pts[i + 1].0 - pts[i].0).hypot(pts[i + 1].1 - pts[i].1))
            .collect();
        let total: f32 = len.iter().sum();
        let dur = 60.0 + 70.0 * (1.0 + total / key_w).log2();
        let mut done = 0.0;
        times[a] = t;
        for (k, l) in len.iter().enumerate() {
            done += l;
            // Minimum jerk: s(τ) = 10τ³ − 15τ⁴ + 6τ⁵; τ for this share of the way.
            let f = if total > 0.0 { done / total } else { 1.0 };
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..24 {
                let m = (lo + hi) / 2.0;
                if m * m * m * (10.0 - 15.0 * m + 6.0 * m * m) < f {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            times[a + k + 1] = t + dur * lo;
        }
        t += dur + 15.0 + 20.0 * rng.unit(); // a short rest at the stop
    }
    times
}
