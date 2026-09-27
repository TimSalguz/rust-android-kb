//! How the phone is held — left thumb, right thumb or both — and where that
//! grip's taps land.
//!
//! Calibrated once, on request (settings → calibration): the farthest arc
//! each thumb reaches, then a phrase typed with each grip, while the tilt of
//! the phone is recorded. While typing, the grip is told apart by the tilt (a
//! thumb reaching across tips the phone its own way) and by the taps (two
//! fingers overlap, and alternate sides fast), and taps are shifted by where
//! that grip usually lands. Nothing is learned outside the calibration; the
//! profile holds tilts and offsets, never text.
//!
//! Coordinates are in keyboard widths from the top-left of the keys area
//! (`u` across, `v` down), so they survive a change of keyboard size.

use std::collections::HashMap;

use crate::layout::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Left,
    Right,
    Both,
    /// One finger (an index finger, the phone on a table): taps come mostly
    /// to the middle, the edges fall short.
    Finger,
}

/// How many grips there are.
pub const GRIPS: usize = 4;

impl Grip {
    pub const ALL: [Grip; GRIPS] = [Grip::Left, Grip::Right, Grip::Both, Grip::Finger];

    pub fn index(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        match self {
            Grip::Left => "left",
            Grip::Right => "right",
            Grip::Both => "both",
            Grip::Finger => "finger",
        }
    }

    /// The indicator: where the thumbs are.
    pub fn label(self) -> &'static str {
        match self {
            Grip::Left => "[I ]",
            Grip::Right => "[ I]",
            Grip::Both => "[II]",
            Grip::Finger => "[•]",
        }
    }

    /// Which hand(s), for the calibration prompts: a key of [`crate::i18n`].
    pub fn hands(self) -> &'static str {
        match self {
            Grip::Left => "grip.left",
            Grip::Right => "grip.right",
            Grip::Both => "grip.both",
            Grip::Finger => "grip.finger",
        }
    }
}

/// The largest shift applied to a tap, in keyboard widths (~ ⅔ of a key).
const MAX_SHIFT: f32 = 0.06;
/// Past this fraction of a thumb's reach, its taps spread outward.
const REACH_EDGE: f32 = 0.75;
/// How much they spread per unit of reach past the edge, when the
/// calibration has too few far taps to say; and the most it may say.
const STRETCH_PRIOR: f32 = 1.0;
const STRETCH_MAX: f32 = 3.0;
/// Aimed past the edge of the reach, a tap falls short toward the pivot, the
/// more the farther: this many keyboard widths per unit of reach past
/// [`REACH_EDGE`] at most, and never more than [`MAX_PULL`] in all.
const PULL_MAX_RATE: f32 = 0.6;
const MAX_PULL: f32 = 0.16;
/// In the calibration's alignment, a tap short of a key past the edge counts
/// as nearer by up to this factor (a miss sideways is still a miss).
const SLACK_RATE: f32 = 4.0;
const MAX_SLACK: f32 = 4.0;

/// Gravity from the accelerometer as a unit vector (screen x right, y up,
/// z out of the screen).
pub type Tilt = [f32; 3];

fn unit(v: Tilt) -> Option<Tilt> {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (n > 1e-3).then(|| [v[0] / n, v[1] / n, v[2] / n])
}

/// Sideways tilt, radians: how far the screen is turned toward the left or
/// right hand.
fn roll(t: Tilt) -> f32 {
    t[0].clamp(-1.0, 1.0).asin()
}

/// One grip as calibrated.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Mean gravity direction and how far single samples stray from it.
    tilt: Tilt,
    tilt_sd: f32,
    /// Roll against where the finger is: `roll ≈ a + b·u`.
    roll_a: f32,
    roll_b: f32,
    roll_sd: f32,
    /// Offset of taps from the intended key center over the keyboard, an
    /// affine field: `du = du[0] + du[1]·u + du[2]·v`, `dv` alike (rows sit
    /// lower the higher they are; far keys are undershot).
    du: [f32; 3],
    dv: [f32; 3],
    /// One thumb's reach: pivot `(u, v)` and radius, from its arc.
    reach: Option<(f32, f32, f32)>,
    /// Near the edge of the reach, a tap aimed farther lands short along
    /// the line from the pivot: the spread outward grows by this much per
    /// unit of reach past [`REACH_EDGE`]…
    stretch: f32,
    /// …and it lands this much short on average (keyboard widths per unit
    /// of reach past the edge).
    pull: f32,
    /// Each key row's stretch the thumb reaches, `(u_from, u_to)`, from the
    /// painted area (empty: an older calibration, or none).
    rows: Vec<(f32, f32)>,
}

impl Profile {
    /// How well a tilt (and, with a tap, where it was) fits this grip, as a
    /// log-likelihood.
    fn fit(&self, t: Tilt, u: Option<f32>) -> f32 {
        let d2: f32 = (0..3).map(|i| (t[i] - self.tilt[i]).powi(2)).sum();
        let mut ll =
            -(d2 / (2.0 * self.tilt_sd * self.tilt_sd)).min(12.0) - 2.0 * self.tilt_sd.ln();
        if let Some(u) = u {
            let r = roll(t) - (self.roll_a + self.roll_b * u);
            ll += -(r * r / (2.0 * self.roll_sd * self.roll_sd)).min(12.0) - self.roll_sd.ln();
        }
        ll
    }

    /// How a tap at `(u, v)` spreads: the outward direction from the thumb's
    /// pivot and the factor along it (1: round).
    fn spread(&self, u: f32, v: f32) -> Option<((f32, f32), f32)> {
        let (pu, pv, r) = self.reach?;
        let (du, dv) = (u - pu, v - pv);
        let d = (du * du + dv * dv).sqrt();
        if d < 1e-4 || r < 1e-3 {
            return None;
        }
        let factor = 1.0 + self.stretch * (d / r - REACH_EDGE).max(0.0);
        Some(((du / d, dv / d), factor.min(1.0 + STRETCH_MAX)))
    }

    /// Where this grip's taps land relative to the key under `(u, v)`: the
    /// affine field, plus the fall short toward the pivot past the reach.
    fn offset(&self, u: f32, v: f32) -> (f32, f32) {
        let at = |c: &[f32; 3]| (c[0] + c[1] * u + c[2] * v).clamp(-MAX_SHIFT, MAX_SHIFT);
        let (du, dv) = (at(&self.du), at(&self.dv));
        // The tap lies short of the key meant: where it was aimed is found
        // in two steps.
        let (s1u, s1v) = self.short(u, v);
        let (su, sv) = self.short(u - s1u, v - s1v);
        (du + su, dv + sv)
    }

    /// The fall short of a tap aimed at `(u, v)` (toward the pivot).
    fn short(&self, u: f32, v: f32) -> (f32, f32) {
        let Some((pu, pv, r)) = self.reach else {
            return (0.0, 0.0);
        };
        let (eu, ev) = (pu - u, pv - v);
        let d = (eu * eu + ev * ev).sqrt();
        let past = d / r.max(1e-3) - REACH_EDGE;
        if past <= 0.0 || d < 1e-4 {
            return (0.0, 0.0);
        }
        let s = (self.pull * past).min(MAX_PULL);
        (s * eu / d, s * ev / d)
    }
}

/// The calibrated grips (none until calibrated).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grips {
    profiles: [Option<Profile>; GRIPS],
    /// Each grip's calibration before the last one (what "undo" restores).
    previous: [Option<Profile>; GRIPS],
    /// When each grip was calibrated, and the undo request last applied to
    /// it (settings stamps, ms).
    made: [u64; GRIPS],
    undone: [u64; GRIPS],
    /// The "reset all" request last applied.
    pub stamp: u64,
}

impl Grips {
    /// Enough grips calibrated to tell them apart (two at least).
    pub fn calibrated(&self) -> bool {
        self.profiles.iter().filter(|p| p.is_some()).count() >= 2
    }

    pub fn has(&self, g: Grip) -> bool {
        self.get(g).is_some()
    }

    /// A new calibration of one grip; the one it replaces can be restored.
    pub fn set(&mut self, g: Grip, profile: Profile, made: u64) {
        let i = g.index();
        self.previous[i] = self.profiles[i].take();
        self.profiles[i] = Some(profile);
        self.made[i] = made;
    }

    /// Undo a grip's last calibration: the one before it again, or none.
    pub fn undo(&mut self, g: Grip, stamp: u64) {
        let i = g.index();
        self.profiles[i] = self.previous[i].take();
        self.undone[i] = stamp;
    }

    /// When the grip was calibrated / its last undo applied (stamps).
    pub fn made(&self, g: Grip) -> u64 {
        self.made[g.index()]
    }

    pub fn undone(&self, g: Grip) -> u64 {
        self.undone[g.index()]
    }

    /// A thumb's calibrated reach: pivot `(u, v)` and radius, keyboard widths.
    pub fn reach(&self, g: Grip) -> Option<(f32, f32, f32)> {
        self.get(g)?.reach
    }

    /// Set a thumb's rows (tests).
    #[cfg(test)]
    pub fn set_rows(&mut self, g: Grip, rows: Vec<(f32, f32)>) {
        if let Some(p) = self.profiles[g.index()].as_mut() {
            p.rows = rows;
        }
    }

    /// The stretch of each key row a thumb reaches, from its painted area.
    pub fn rows(&self, g: Grip) -> Option<&[(f32, f32)]> {
        let rows = &self.get(g)?.rows;
        (!rows.is_empty()).then_some(rows.as_slice())
    }

    fn get(&self, g: Grip) -> Option<&Profile> {
        self.profiles[g.index()].as_ref()
    }

    /// The tap offset expected at `(u, v)` for a grip mix `probs`.
    pub fn shift(&self, probs: [f32; GRIPS], u: f32, v: f32) -> (f32, f32) {
        let (mut du, mut dv) = (0.0, 0.0);
        for g in Grip::ALL {
            if let Some(p) = self.get(g) {
                let (a, b) = p.offset(u, v);
                du += probs[g.index()] * a;
                dv += probs[g.index()] * b;
            }
        }
        (du, dv)
    }

    /// [`Grips::shift`] without the fall short past the reach — for the
    /// one-handed rows, which keep every key within it.
    pub fn shift_near(&self, probs: [f32; GRIPS], u: f32, v: f32) -> (f32, f32) {
        let (mut du, mut dv) = (0.0, 0.0);
        for g in Grip::ALL {
            if let Some(p) = self.get(g) {
                let at = |c: &[f32; 3]| (c[0] + c[1] * u + c[2] * v).clamp(-MAX_SHIFT, MAX_SHIFT);
                du += probs[g.index()] * at(&p.du);
                dv += probs[g.index()] * at(&p.dv);
            }
        }
        (du, dv)
    }

    /// How a tap at `(u, v)` spreads for a grip mix: the outward direction
    /// and the factor along it, when it is stretched at all (near the edge
    /// of a thumb's reach — the key meant may lie farther out).
    pub fn spread(&self, probs: [f32; GRIPS], u: f32, v: f32) -> Option<((f32, f32), f32)> {
        let (mut factor, mut dx, mut dy) = (0.0, 0.0, 0.0);
        for g in Grip::ALL {
            let p = probs[g.index()];
            match self.get(g).and_then(|prof| prof.spread(u, v)) {
                Some(((ex, ey), f)) => {
                    factor += p * f;
                    dx += p * (f - 1.0) * ex;
                    dy += p * (f - 1.0) * ey;
                }
                None => factor += p,
            }
        }
        let n = (dx * dx + dy * dy).sqrt();
        (factor > 1.05 && n > 1e-4).then(|| ((dx / n, dy / n), factor))
    }

    /// `grip.conf`: plain lines of numbers — each grip's profile, the one
    /// before it (`was-` lines) and its stamps.
    pub fn to_text(&self) -> String {
        let mut out = format!("stamp {}\n", self.stamp);
        for g in Grip::ALL {
            let (i, name) = (g.index(), g.name());
            out += &format!("{name} stamps {} {}\n", self.made[i], self.undone[i]);
            if let Some(p) = &self.profiles[i] {
                write_profile(&mut out, name, p);
            }
            if let Some(p) = &self.previous[i] {
                write_profile(&mut out, &format!("was-{name}"), p);
            }
        }
        out
    }

    pub fn from_text(text: &str) -> Grips {
        let mut grips = Grips::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| f.get(i).and_then(|s| s.parse::<f32>().ok());
            if f.first() == Some(&"stamp") {
                grips.stamp = f.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                continue;
            }
            let first = f.first().copied().unwrap_or("");
            let (was, name) = match first.strip_prefix("was-") {
                Some(n) => (true, n),
                None => (false, first),
            };
            let Some(g) = Grip::ALL.into_iter().find(|g| g.name() == name) else {
                continue;
            };
            if f.get(1) == Some(&"stamps") {
                let stamp = |i: usize| f.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
                grips.made[g.index()] = stamp(2);
                grips.undone[g.index()] = stamp(3);
                continue;
            }
            let slot = if was {
                &mut grips.previous[g.index()]
            } else {
                &mut grips.profiles[g.index()]
            };
            let p = slot.get_or_insert(Profile {
                tilt: [0.0, 0.0, 1.0],
                tilt_sd: 0.1,
                roll_a: 0.0,
                roll_b: 0.0,
                roll_sd: 0.1,
                du: [0.0; 3],
                dv: [0.0; 3],
                reach: None,
                stretch: STRETCH_PRIOR,
                pull: 0.0,
                rows: Vec::new(),
            });
            match (f.get(1).copied(), f.len()) {
                (Some("tilt"), 10) => {
                    if let (Some(x), Some(y), Some(z), Some(sd), Some(a), Some(b), Some(rs)) =
                        (num(2), num(3), num(4), num(5), num(7), num(8), num(9))
                    {
                        p.tilt = [x, y, z];
                        p.tilt_sd = sd.max(0.02);
                        (p.roll_a, p.roll_b, p.roll_sd) = (a, b, rs.max(0.02));
                    }
                }
                (Some("reach"), 5..=7) => {
                    if let (Some(a), Some(b), Some(r)) = (num(2), num(3), num(4)) {
                        p.reach = Some((a, b, r));
                        p.stretch = num(5).map_or(STRETCH_PRIOR, |k| k.clamp(0.0, STRETCH_MAX));
                        p.pull = num(6).map_or(0.0, |k| k.clamp(0.0, PULL_MAX_RATE));
                    }
                }
                (Some("rows"), 10) => {
                    if let Some(c) = (2..10).map(num).collect::<Option<Vec<f32>>>() {
                        p.rows = c.chunks(2).map(|x| (x[0], x[1])).collect();
                    }
                }
                (Some("shift"), 8) => {
                    if let Some(c) = (2..8).map(num).collect::<Option<Vec<f32>>>() {
                        p.du = [c[0], c[1], c[2]];
                        p.dv = [c[3], c[4], c[5]];
                    }
                }
                _ => {}
            }
        }
        grips
    }
}

/// The running guess of the grip, from the tilt and the taps.
#[derive(Clone, Debug, Default)]
pub struct Sense {
    tilt: Option<Tilt>,
    /// Log-evidence per grip, fading with every new observation.
    score: [f32; GRIPS],
    last_tap: Option<(f32, i64)>,
    tilts_seen: u32,
    shown: Option<Grip>,
}

impl Sense {
    pub fn tilt(&self) -> Option<Tilt> {
        self.tilt
    }

    pub fn shown(&self) -> Option<Grip> {
        self.shown
    }

    /// Grip probabilities (even when nothing is known).
    pub fn probs(&self) -> [f32; GRIPS] {
        let m = self.score.iter().copied().fold(f32::MIN, f32::max);
        let e = self.score.map(|s| (s - m).exp());
        let sum: f32 = e.iter().sum();
        e.map(|x| x / sum)
    }

    /// A new accelerometer sample (m/s², any scale). Returns whether the
    /// shown grip changed: the tilt alone moves the guess, slowly.
    pub fn on_tilt(&mut self, grips: &Grips, raw: Tilt) -> bool {
        let Some(t) = unit(raw) else {
            return false;
        };
        let smooth = match self.tilt {
            Some(p) => unit([
                0.8 * p[0] + 0.2 * t[0],
                0.8 * p[1] + 0.2 * t[1],
                0.8 * p[2] + 0.2 * t[2],
            ]),
            None => Some(t),
        };
        self.tilt = smooth;
        self.tilts_seen += 1;
        if !grips.calibrated() || !self.tilts_seen.is_multiple_of(8) {
            return false;
        }
        self.observe(grips, None, 0.25);
        self.update_shown()
    }

    /// A letter tap at `u` (keyboard widths across) at `ms`; `overlap`: it
    /// landed while another finger was still down. Returns whether the shown
    /// grip changed.
    pub fn on_tap(&mut self, grips: &Grips, u: f32, ms: i64, overlap: bool) -> bool {
        let mut touch = [0.0f32; GRIPS];
        if overlap {
            touch[Grip::Both.index()] += 2.0; // one thumb can't overlap itself
        }
        if let Some((pu, pms)) = self.last_tap {
            let (dt, du) = (ms - pms, (u - pu).abs());
            if dt < 150 && du > 0.35 {
                touch[Grip::Both.index()] += 0.8; // across the keyboard too fast for one thumb
            } else if dt > 280 && du > 0.5 {
                touch[Grip::Left.index()] += 0.2;
                touch[Grip::Right.index()] += 0.2;
                touch[Grip::Finger.index()] += 0.2;
            }
        }
        self.last_tap = Some((u, ms));
        if !grips.calibrated() {
            return false;
        }
        self.observe(grips, Some(u), 0.5);
        for (s, t) in self.score.iter_mut().zip(touch) {
            *s += t;
        }
        self.update_shown()
    }

    fn observe(&mut self, grips: &Grips, u: Option<f32>, weight: f32) {
        let Some(t) = self.tilt else { return };
        for g in Grip::ALL {
            let fit = grips.get(g).map_or(-12.0, |p| p.fit(t, u));
            let s = &mut self.score[g.index()];
            *s = 0.85 * *s + weight * fit;
        }
    }

    fn update_shown(&mut self) -> bool {
        let p = self.probs();
        let best = Grip::ALL
            .into_iter()
            .max_by(|a, b| p[a.index()].total_cmp(&p[b.index()]))
            .unwrap_or(Grip::Both);
        // Hysteresis: a new grip must be clearly likelier.
        let sure = if self.shown.is_none() { 0.6 } else { 0.75 };
        if Some(best) != self.shown && p[best.index()] >= sure {
            self.shown = Some(best);
            return true;
        }
        false
    }

    /// Forget the guess (calibration changed).
    pub fn reset(&mut self) {
        *self = Sense {
            tilt: self.tilt,
            ..Sense::default()
        };
    }
}

/// The phrase typed with each grip: every Russian letter but ё and ъ (they
/// need a long press), 51 taps with the spaces. Slips are fine: the taps are
/// aligned to the phrase (a missed, extra or swapped letter shifts nothing),
/// and a tap far from its letter is left out.
pub const PHRASE: &str = "в чащах юга жил бы цитрус да но фальшивый экземпляр";

/// The phrase for a language, typed on its layout (pangrams, or nearly:
/// letters behind a long press left out).
pub fn phrase_for(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru => PHRASE,
        Lang::En => "the quick brown fox jumps over the lazy dog",
        Lang::De => "zwölf boxkämpfer jagen viktor quer über den sylter deich",
        Lang::Fr => "portez ce vieux whisky au juge blond qui fume",
        Lang::Es => "jovencillo emponzoñado de whisky que figurota exhibe",
        Lang::Pt => "zebras caolhas de java querem mandar fax para moça gigante de new york",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Paint where the thumb comfortably reaches (strokes, then "done").
    Area(Grip),
    Type(Grip),
}

/// A grip's calibration: a thumb paints its area, then types the phrase;
/// two hands and one finger type it.
pub fn steps_for(g: Grip) -> Vec<Step> {
    match g {
        Grip::Left | Grip::Right => vec![Step::Area(g), Step::Type(g)],
        Grip::Both | Grip::Finger => vec![Step::Type(g)],
    }
}

/// What one grip showed during the calibration.
#[derive(Clone, Debug, Default)]
struct Samples {
    tilts: Vec<Tilt>,
    /// `(u, roll)`: where the finger was and how the phone was turned.
    rolls: Vec<(f32, f32)>,
    /// `(u, v, du, dv)`: intended key center and the tap's offset from it.
    offsets: Vec<(f32, f32, f32, f32)>,
    /// Where the thumb reaches: the painted points, `(u, v)`.
    area: Vec<(f32, f32)>,
}

/// Painted points it takes to call an area an area.
const AREA_MIN_POINTS: usize = 12;

/// A calibration in progress.
#[derive(Clone, Debug, Default)]
pub struct Calibration {
    /// The grip calibrated, and its steps.
    grip: Option<Grip>,
    steps: Vec<Step>,
    step: usize,
    /// The thumbs' reaches from earlier calibrations (for two hands typing).
    known: [Option<(f32, f32, f32)>; GRIPS],
    /// The current stroke, or the current phrase's taps: `(u, v, tilt)`.
    points: Vec<(f32, f32, Option<Tilt>)>,
    /// The strokes painted so far in this step (to show them).
    strokes: Vec<Vec<(f32, f32)>>,
    samples: [Samples; GRIPS],
    /// How far into the phrase the taps are, by the alignment.
    progress: usize,
    /// The phrase typed (empty: the Russian one).
    phrase: &'static str,
    /// "Done" came too early: too little painted.
    pub retry: bool,
    /// The current touch began in the suggestion strip (where "cancel" is),
    /// not on the keys.
    pub strip_touch: bool,
}

impl Calibration {
    /// A calibration of one grip, typing `phrase` (see [`phrase_for`]);
    /// `known`: the thumbs' reaches calibrated before.
    pub fn new(
        phrase: &'static str,
        grip: Grip,
        known: [Option<(f32, f32, f32)>; GRIPS],
    ) -> Calibration {
        Calibration {
            phrase,
            grip: Some(grip),
            steps: steps_for(grip),
            known,
            ..Calibration::default()
        }
    }

    pub fn grip(&self) -> Option<Grip> {
        self.grip
    }

    /// A thumb's rows as painted so far (to type the phrase on them).
    pub fn rows_now(&self, row_v: f32) -> Option<Vec<(f32, f32)>> {
        let g = self.grip?;
        let rows = row_spans(&self.samples[g.index()].area, row_v);
        (!rows.is_empty()).then_some(rows)
    }

    pub fn phrase(&self) -> &'static str {
        if self.phrase.is_empty() {
            PHRASE
        } else {
            self.phrase
        }
    }

    pub fn step(&self) -> Option<Step> {
        self.steps.get(self.step).copied()
    }

    /// Chars of the phrase typed so far (typing steps), by the alignment: a
    /// missed or extra letter doesn't shift the rest.
    pub fn typed(&self) -> usize {
        self.progress
    }

    /// Points of the stroke being drawn.
    pub fn trail(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.points.iter().map(|&(u, v, _)| (u, v))
    }

    /// The strokes painted in this step so far, the current one too.
    pub fn painted(&self) -> Vec<Vec<(f32, f32)>> {
        let mut all = self.strokes.clone();
        all.push(self.trail().collect());
        all
    }

    pub fn arc_start(&mut self) {
        self.points.clear();
    }

    pub fn arc_point(&mut self, u: f32, v: f32, tilt: Option<Tilt>) {
        self.points.push((u, v, tilt));
    }

    /// The finger lifted: the stroke joins the area painted in this step.
    pub fn arc_end(&mut self) {
        let Some(Step::Area(g)) = self.step() else {
            return;
        };
        let s = &mut self.samples[g.index()];
        for &(u, v, t) in &self.points {
            s.area.push((u, v));
            if let Some(t) = t {
                s.tilts.push(t);
                s.rolls.push((u, roll(t)));
            }
        }
        self.strokes.push(self.trail().collect());
        self.points.clear();
        self.retry = false;
    }

    /// "Done" with the area: on to the next step, if enough was painted —
    /// a stroke long enough in any direction (a left thumb's reach may run
    /// mostly up the middle of the keyboard).
    pub fn area_done(&mut self) {
        let Some(Step::Area(g)) = self.step() else {
            return;
        };
        let area = &self.samples[g.index()].area;
        let (mut lo_u, mut hi_u, mut lo_v, mut hi_v) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for &(u, v) in area {
            (lo_u, hi_u, lo_v, hi_v) = (lo_u.min(u), hi_u.max(u), lo_v.min(v), hi_v.max(v));
        }
        let extent = (hi_u - lo_u).max(0.0).hypot((hi_v - lo_v).max(0.0));
        self.retry = area.len() < AREA_MIN_POINTS || extent < 0.3;
        if !self.retry {
            self.strokes.clear();
            self.step += 1;
        }
    }

    /// A tap on a letter or the space bar while typing the phrase
    /// (`centers`: key center per char). Returns true when the phrase is
    /// done — the taps reach its last letter (then call
    /// [`Calibration::phrase_done`]).
    pub fn tap(
        &mut self,
        u: f32,
        v: f32,
        tilt: Option<Tilt>,
        centers: &HashMap<char, (f32, f32)>,
        key_w: f32,
    ) -> bool {
        self.points.push((u, v, tilt));
        let m = self.phrase().chars().count();
        self.update_progress(centers, key_w);
        self.progress >= m || self.points.len() >= m + 12
    }

    /// ⌫ while typing the phrase: take the last tap back.
    pub fn back(&mut self, centers: &HashMap<char, (f32, f32)>, key_w: f32) {
        self.points.pop();
        self.update_progress(centers, key_w);
    }

    fn update_progress(&mut self, centers: &HashMap<char, (f32, f32)>, key_w: f32) {
        let target: Vec<char> = self.phrase().chars().collect();
        let taps: Vec<(f32, f32)> = self.points.iter().map(|&(u, v, _)| (u, v)).collect();
        let pull = self.pull();
        self.progress = prefix_end(&taps, &target, centers, key_w, &pull);
    }

    /// Where the current grip's taps fall short, from the arcs drawn.
    fn pull(&self) -> Pull {
        let reach = |g: Grip| {
            fit_circle(&outer_edge(&self.samples[g.index()].area, g)).or(self.known[g.index()])
        };
        match self.step() {
            Some(Step::Type(Grip::Left)) => reach(Grip::Left).map_or(Pull::None, Pull::Thumb),
            Some(Step::Type(Grip::Right)) => reach(Grip::Right).map_or(Pull::None, Pull::Thumb),
            Some(Step::Type(Grip::Both)) => match (reach(Grip::Left), reach(Grip::Right)) {
                (Some(l), Some(r)) => Pull::Both(l, r),
                _ => Pull::None,
            },
            Some(Step::Type(Grip::Finger)) => Pull::Finger,
            _ => Pull::None,
        }
    }

    /// The phrase is typed: match the taps to its letters (`centers`: key
    /// center per char) and keep the offsets and tilts.
    pub fn phrase_done(&mut self, centers: &HashMap<char, (f32, f32)>, key_w: f32) {
        let Some(Step::Type(g)) = self.step() else {
            return;
        };
        let target: Vec<char> = self.phrase().chars().collect();
        let taps: Vec<(f32, f32)> = self.points.iter().map(|&(u, v, _)| (u, v)).collect();
        let pull = self.pull();
        let s = &mut self.samples[g.index()];
        for (i, j) in align(&taps, &target, centers, key_w, &pull) {
            let (u, v, t) = self.points[i];
            let c = target[j];
            if let Some(t) = t {
                s.tilts.push(t);
                s.rolls.push((u, roll(t)));
            }
            if c != ' ' {
                let (cu, cv) = centers[&c];
                s.offsets.push((cu, cv, u - cu, v - cv));
            }
        }
        self.points.clear();
        self.progress = 0;
        self.step += 1;
    }

    pub fn done(&self) -> bool {
        self.step >= self.steps.len()
    }

    /// The grip's profile, once every step is done; `row_v`: a key row's
    /// height in keyboard widths (to tell what each row of keys can reach).
    pub fn finish(&self, row_v: f32) -> Option<Profile> {
        let g = self.grip?;
        let s = &self.samples[g.index()];
        if s.tilts.is_empty() && s.offsets.is_empty() {
            return None;
        }
        let (tilt, tilt_sd) = mean_tilt(&s.tilts);
        let (roll_a, roll_b, roll_sd) = regress(&s.rolls);
        let reach = fit_circle(&outer_edge(&s.area, g)).or(self.known[g.index()]);
        let rows = row_spans(&s.area, row_v);
        let (du, dv, pull) = fit_shift(&s.offsets, reach);
        let stretch = reach.map_or(STRETCH_PRIOR, |r| fit_stretch(&s.offsets, du, dv, r, pull));
        Some(Profile {
            tilt,
            tilt_sd,
            roll_a,
            roll_b,
            roll_sd,
            du,
            dv,
            reach,
            stretch,
            pull,
            rows,
        })
    }
}

fn write_profile(out: &mut String, name: &str, p: &Profile) {
    *out += &format!(
        "{name} tilt {} {} {} {} roll {} {} {}\n",
        p.tilt[0], p.tilt[1], p.tilt[2], p.tilt_sd, p.roll_a, p.roll_b, p.roll_sd
    );
    let (a, b) = (p.du, p.dv);
    *out += &format!(
        "{name} shift {} {} {} {} {} {}\n",
        a[0], a[1], a[2], b[0], b[1], b[2]
    );
    if let Some((a, b, r)) = p.reach {
        *out += &format!("{name} reach {a} {b} {r} {} {}\n", p.stretch, p.pull);
    }
    if !p.rows.is_empty() {
        let spans: Vec<String> = p.rows.iter().map(|(a, b)| format!("{a} {b}")).collect();
        *out += &format!("{name} rows {}\n", spans.join(" "));
    }
}

/// The far edge of a painted area, as the thumb sees it: seen from the
/// area's corner on the thumb's side (bottom left for the left thumb), the
/// farthest point in each 5° direction — the circle through them is the
/// thumb's reach.
fn outer_edge(area: &[(f32, f32)], g: Grip) -> Vec<(f32, f32)> {
    let Some(&(first_u, _)) = area.first() else {
        return Vec::new();
    };
    let (mut cu, mut cv) = (first_u, f32::MIN);
    for &(u, v) in area {
        cu = match g {
            Grip::Right => cu.max(u),
            _ => cu.min(u),
        };
        cv = cv.max(v);
    }
    let mut bins: std::collections::BTreeMap<i32, (f32, (f32, f32))> =
        std::collections::BTreeMap::new();
    for &(u, v) in area {
        let (du, dv) = ((u - cu).abs(), cv - v);
        let key = (dv.atan2(du).to_degrees() / 5.0).floor() as i32;
        let d = du.hypot(dv);
        let e = bins.entry(key).or_insert((d, (u, v)));
        if d > e.0 {
            *e = (d, (u, v));
        }
    }
    bins.into_values().map(|(_, p)| p).collect()
}

/// For each of the 4 key rows (`row_v` tall, from the top of the keys), the
/// stretch of it the painted area covers, `(u_from, u_to)` — a few stray
/// points at either end left out. A row with too little paint takes its
/// nearest painted neighbor's. Empty when nothing was painted.
fn row_spans(area: &[(f32, f32)], row_v: f32) -> Vec<(f32, f32)> {
    if area.len() < AREA_MIN_POINTS || row_v <= 0.0 {
        return Vec::new();
    }
    let spans: Vec<Option<(f32, f32)>> = (0..4)
        .map(|r| {
            let (top, bottom) = (r as f32 * row_v, (r + 1) as f32 * row_v);
            let mut us: Vec<f32> = area
                .iter()
                .filter(|&&(_, v)| v >= top && v < bottom)
                .map(|&(u, _)| u)
                .collect();
            if us.len() < 4 {
                return None;
            }
            us.sort_by(f32::total_cmp);
            let at = |q: f32| us[((us.len() - 1) as f32 * q).round() as usize];
            Some((at(0.03).clamp(0.0, 1.0), at(0.97).clamp(0.0, 1.0)))
        })
        .collect();
    if spans.iter().all(Option::is_none) {
        return Vec::new();
    }
    (0..4usize)
        .map(|r| {
            (0..4usize)
                .flat_map(|d| [r.checked_sub(d), Some(r + d)])
                .flatten()
                .filter(|&i| i < 4)
                .find_map(|i| spans[i])
                .unwrap_or((0.0, 1.0))
        })
        .collect()
}

fn mean_tilt(ts: &[Tilt]) -> (Tilt, f32) {
    let sum = ts
        .iter()
        .fold([0.0f32; 3], |a, t| [a[0] + t[0], a[1] + t[1], a[2] + t[2]]);
    let Some(mean) = unit(sum) else {
        return ([0.0, 0.0, 1.0], 0.5);
    };
    let var = ts
        .iter()
        .map(|t| (0..3).map(|i| (t[i] - mean[i]).powi(2)).sum::<f32>())
        .sum::<f32>()
        / ts.len().max(1) as f32;
    (mean, var.sqrt().max(0.03))
}

/// Least squares `y ≈ a + b·x`, and the residual spread.
fn regress(pts: &[(f32, f32)]) -> (f32, f32, f32) {
    let n = pts.len() as f32;
    if pts.len() < 3 {
        return (0.0, 0.0, 0.5);
    }
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let sxx: f32 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f32 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let b = if sxx > 1e-6 { sxy / sxx } else { 0.0 };
    let a = my - b * mx;
    let res = (pts.iter().map(|p| (p.1 - a - b * p.0).powi(2)).sum::<f32>() / n).sqrt();
    (a, b, res.max(0.02))
}

/// Least squares for 3 unknowns: `Σ row·rowᵀ · x = Σ row·rhs`, plus
/// `ridge` on the diagonal (pulls poorly determined terms toward 0).
fn solve3(rows: impl Iterator<Item = ([f64; 3], f64)>, ridge: f64) -> Option<[f64; 3]> {
    let mut m = [[0.0f64; 4]; 3];
    for (row, rhs) in rows {
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += row[i] * row[j];
            }
            m[i][3] += row[i] * rhs;
        }
    }
    for (i, r) in m.iter_mut().enumerate() {
        r[i] += ridge;
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..3 {
        let piv = (col..3).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        m.swap(col, piv);
        if m[col][col].abs() < 1e-12 {
            return None;
        }
        let pivot = m[col];
        for (r, row) in m.iter_mut().enumerate() {
            if r != col {
                let k = row[col] / pivot[col];
                for (x, p) in row.iter_mut().zip(pivot).skip(col) {
                    *x -= k * p;
                }
            }
        }
    }
    Some([m[0][3] / m[0][0], m[1][3] / m[1][1], m[2][3] / m[2][2]])
}

/// Circle through the points (algebraic least squares): the thumb's pivot
/// and reach.
fn fit_circle(pts: &[(f32, f32)]) -> Option<(f32, f32, f32)> {
    if pts.len() < 5 {
        return None;
    }
    // x² + y² + a·x + b·y + c = 0.
    let rows = pts.iter().map(|&(x, y)| {
        let (x, y) = (x as f64, y as f64);
        ([x, y, 1.0], -(x * x + y * y))
    });
    let [a, b, c] = solve3(rows, 0.0)?;
    let (cx, cy) = (-a / 2.0, -b / 2.0);
    let r2 = cx * cx + cy * cy - c;
    (r2 > 0.0).then(|| (cx as f32, cy as f32, r2.sqrt() as f32))
}

/// `d ≈ c₀ + c₁·u + c₂·v` over `(u, v, d)`, slopes held back a little (a
/// phrase is ~50 taps).
fn fit_affine(pts: &[(f32, f32, f32)]) -> [f32; 3] {
    if pts.len() < 6 {
        return [0.0; 3];
    }
    let rows = pts
        .iter()
        .map(|&(u, v, d)| ([1.0, u as f64, v as f64], d as f64));
    solve3(rows, 0.5).map_or([0.0; 3], |c| c.map(|x| x as f32))
}

/// Where a grip's taps land: the affine field `(du, dv)` and, with a thumb's
/// reach, how far taps aimed past it fall short toward the pivot (per unit
/// of reach past [`REACH_EDGE`]). Fitted together — alone, the field would
/// take a long reach's fall short for a shift of every tap.
fn fit_shift(
    offsets: &[(f32, f32, f32, f32)],
    reach: Option<(f32, f32, f32)>,
) -> ([f32; 3], [f32; 3], f32) {
    let affine = |k: f32| {
        let short = |u: f32, v: f32| reach.map_or((0.0, 0.0), |r| short_dir(r, u, v));
        let du: Vec<(f32, f32, f32)> = offsets
            .iter()
            .map(|&(u, v, ou, _)| (u, v, ou - k * short(u, v).0))
            .collect();
        let dv: Vec<(f32, f32, f32)> = offsets
            .iter()
            .map(|&(u, v, _, ov)| (u, v, ov - k * short(u, v).1))
            .collect();
        (fit_affine(&du), fit_affine(&dv))
    };
    let Some(r) = reach.filter(|_| offsets.len() >= 6) else {
        let (du, dv) = affine(0.0);
        return (du, dv, 0.0);
    };
    // Unknowns: du₀ du₁ du₂ dv₀ dv₁ dv₂ and the rate of falling short.
    let rows = offsets.iter().flat_map(|&(u, v, ou, ov)| {
        let (su, sv) = short_dir(r, u, v);
        let (u, v) = (u as f64, v as f64);
        [
            (vec![1.0, u, v, 0.0, 0.0, 0.0, su as f64], ou as f64),
            (vec![0.0, 0.0, 0.0, 1.0, u, v, sv as f64], ov as f64),
        ]
    });
    let ridge = [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.01];
    match solve(rows, &ridge) {
        Some(x) if (0.0..=PULL_MAX_RATE as f64).contains(&x[6]) => {
            let c = |i: usize| x[i] as f32;
            ([c(0), c(1), c(2)], [c(3), c(4), c(5)], c(6))
        }
        // Out of bounds: the rate held at the bound, the field fitted to the rest.
        Some(x) => {
            let k = (x[6] as f32).clamp(0.0, PULL_MAX_RATE);
            let (du, dv) = affine(k);
            (du, dv, k)
        }
        None => {
            let (du, dv) = affine(0.0);
            (du, dv, 0.0)
        }
    }
}

/// For a key at `(u, v)`: how far past the reach `(pivot, radius)` it lies
/// (0 within [`REACH_EDGE`]) times the unit direction toward the pivot.
fn short_dir((pu, pv, r): (f32, f32, f32), u: f32, v: f32) -> (f32, f32) {
    let (eu, ev) = (pu - u, pv - v);
    let d = (eu * eu + ev * ev).sqrt();
    let past = d / r.max(1e-3) - REACH_EDGE;
    if past <= 0.0 || d < 1e-4 {
        return (0.0, 0.0);
    }
    (past * eu / d, past * ev / d)
}

/// Least squares for `ridge.len()` unknowns: `Σ row·rowᵀ · x = Σ row·rhs`,
/// with `ridge[i]` added on the diagonal.
#[allow(clippy::needless_range_loop)]
fn solve(rows: impl Iterator<Item = (Vec<f64>, f64)>, ridge: &[f64]) -> Option<Vec<f64>> {
    let n = ridge.len();
    let mut m = vec![vec![0.0f64; n + 1]; n];
    for (row, rhs) in rows {
        for i in 0..n {
            for j in 0..n {
                m[i][j] += row[i] * row[j];
            }
            m[i][n] += row[i] * rhs;
        }
    }
    for i in 0..n {
        m[i][i] += ridge[i];
    }
    for col in 0..n {
        let piv = (col..n).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        m.swap(col, piv);
        if m[col][col].abs() < 1e-12 {
            return None;
        }
        let pivot = m[col].clone();
        for (r, row) in m.iter_mut().enumerate() {
            if r != col {
                let k = row[col] / pivot[col];
                for (x, p) in row.iter_mut().zip(&pivot).skip(col) {
                    *x -= k * p;
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

/// How much taps near the edge of the reach spread outward, from the
/// phrase: past the edge, the leftover error (after the affine shift) along
/// the line from the pivot against the error across it. Too few far taps:
/// the prior.
fn fit_stretch(
    offsets: &[(f32, f32, f32, f32)],
    du: [f32; 3],
    dv: [f32; 3],
    (pu, pv, r): (f32, f32, f32),
    pull: f32,
) -> f32 {
    let (mut along, mut across, mut past, mut n) = (0.0f32, 0.0f32, 0.0f32, 0usize);
    for &(u, v, ou, ov) in offsets {
        let (eu, ev) = (u - pu, v - pv);
        let d = (eu * eu + ev * ev).sqrt();
        let rho = d / r.max(1e-3);
        if rho <= REACH_EDGE || d < 1e-4 {
            continue;
        }
        let (ru, rv) = (
            ou - (du[0] + du[1] * u + du[2] * v),
            ov - (dv[0] + dv[1] * u + dv[2] * v),
        );
        let (a, c) = ((ru * eu + rv * ev) / d, (-ru * ev + rv * eu) / d);
        // Less the average fall short (a is outward).
        let a = a + (pull * (rho - REACH_EDGE)).min(MAX_PULL);
        along += a * a;
        across += c * c;
        past += rho - REACH_EDGE;
        n += 1;
    }
    if n < 6 || across <= 0.0 {
        return STRETCH_PRIOR;
    }
    let ratio = (along / across).sqrt();
    ((ratio - 1.0) / (past / n as f32).max(0.1)).clamp(0.0, STRETCH_MAX)
}

/// An extra or missed tap costs this much in the alignment (in key widths:
/// more than a sloppy tap, less than one on another key)…
const GAP: f32 = 1.5;
/// …and two letters typed the other way round this much besides their taps.
const SWAP: f32 = 0.3;

/// Alignment costs of taps against the phrase's chars, `d[i][j]`: the first
/// `i` taps typed as the first `j` chars — each tap costs its distance to the
/// char's key (in key widths, at most 3), a missed or extra tap [`GAP`], two
/// swapped letters their distances plus [`SWAP`].
fn alignment(
    taps: &[(f32, f32)],
    target: &[char],
    centers: &HashMap<char, (f32, f32)>,
    key_w: f32,
    pull: &Pull,
) -> Vec<Vec<f32>> {
    let (n, m) = (taps.len(), target.len());
    let dist = |i: usize, j: usize| -> f32 { tap_dist(taps, target, centers, key_w, pull, i, j) };
    let mut d = vec![vec![0.0f32; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i as f32 * GAP;
    }
    for (j, x) in d[0].iter_mut().enumerate() {
        *x = j as f32 * GAP;
    }
    for i in 1..=n {
        for j in 1..=m {
            let mut best = (d[i - 1][j - 1] + dist(i - 1, j - 1))
                .min(d[i - 1][j] + GAP)
                .min(d[i][j - 1] + GAP);
            if i >= 2 && j >= 2 {
                best = best.min(d[i - 2][j - 2] + dist(i - 2, j - 1) + dist(i - 1, j - 2) + SWAP);
            }
            d[i][j] = best;
        }
    }
    d
}

/// How far tap `i` is from char `j`'s key, in key widths (at most 3): a tap
/// short of a key past the reach, toward where the finger comes from, counts
/// as nearer.
fn tap_dist(
    taps: &[(f32, f32)],
    target: &[char],
    centers: &HashMap<char, (f32, f32)>,
    key_w: f32,
    pull: &Pull,
    i: usize,
    j: usize,
) -> f32 {
    centers.get(&target[j]).map_or(3.0, |&(cu, cv)| {
        let (u, v) = taps[i];
        let (ou, ov) = ((u - cu) / key_w, (v - cv) / key_w);
        let (along, across) = match pull.at(cu, cv) {
            Some(((eu, ev), slack)) => {
                let a = ou * eu + ov * ev;
                (if a > 0.0 { a / slack } else { a }, -ou * ev + ov * eu)
            }
            None => (ou, ov),
        };
        (along * along + across * across).sqrt().min(3.0)
    })
}

/// How many of the phrase's chars the taps so far stand for: where the
/// cheapest alignment of all the taps to a start of the phrase ends.
fn prefix_end(
    taps: &[(f32, f32)],
    target: &[char],
    centers: &HashMap<char, (f32, f32)>,
    key_w: f32,
    pull: &Pull,
) -> usize {
    if taps.is_empty() {
        return 0;
    }
    let d = alignment(taps, target, centers, key_w, pull);
    let last = &d[taps.len()];
    (1..last.len())
        .rev()
        .min_by(|&a, &b| last[a].total_cmp(&last[b]))
        .unwrap_or(0)
}

/// Where a grip's taps fall short while typing the phrase — toward a thumb's
/// pivot past its reach, toward the middle for one finger — so the alignment
/// forgives a long reach that fell short.
#[derive(Clone, Copy, Debug)]
enum Pull {
    None,
    /// One thumb: pivot and reach.
    Thumb((f32, f32, f32)),
    /// Two thumbs: the left one's on the left half, the right one's on the right.
    Both((f32, f32, f32), (f32, f32, f32)),
    /// One finger over the middle: the edges fall short toward it.
    Finger,
}

impl Pull {
    /// For a key at `(u, v)`: the direction taps fall short in and how much
    /// shorter than it is such a tap counts.
    fn at(self, u: f32, v: f32) -> Option<((f32, f32), f32)> {
        let thumb = |(pu, pv, r): (f32, f32, f32)| {
            let (eu, ev) = (pu - u, pv - v);
            let d = (eu * eu + ev * ev).sqrt();
            let past = d / r.max(1e-3) - REACH_EDGE;
            (past > 0.0 && d > 1e-4)
                .then(|| ((eu / d, ev / d), (1.0 + SLACK_RATE * past).min(MAX_SLACK)))
        };
        match self {
            Pull::None => None,
            Pull::Thumb(c) => thumb(c),
            Pull::Both(l, r) => thumb(if u < 0.5 { l } else { r }),
            Pull::Finger => {
                let past = (u - 0.5).abs() - 0.25;
                (past > 0.0).then(|| (((0.5 - u).signum(), 0.0), (1.0 + 8.0 * past).min(MAX_SLACK)))
            }
        }
    }
}

/// Match taps to the phrase's chars (a typo may add, drop or swap a tap):
/// pairs `(tap, char)` of the cheapest alignment, keeping only the pairs
/// where the tap landed within a key and a half of its char's key.
fn align(
    taps: &[(f32, f32)],
    target: &[char],
    centers: &HashMap<char, (f32, f32)>,
    key_w: f32,
    pull: &Pull,
) -> Vec<(usize, usize)> {
    let d = alignment(taps, target, centers, key_w, pull);
    let dist = |i: usize, j: usize| -> f32 { tap_dist(taps, target, centers, key_w, pull, i, j) };
    let near = |i: usize, j: usize| dist(i, j) <= 1.5;
    let same = |a: f32, b: f32| (a - b).abs() < 1e-4;
    let (mut i, mut j, mut out) = (taps.len(), target.len(), Vec::new());
    while i > 0 && j > 0 {
        if same(d[i][j], d[i - 1][j - 1] + dist(i - 1, j - 1)) {
            if near(i - 1, j - 1) {
                out.push((i - 1, j - 1));
            }
            i -= 1;
            j -= 1;
        } else if i >= 2
            && j >= 2
            && same(
                d[i][j],
                d[i - 2][j - 2] + dist(i - 2, j - 1) + dist(i - 1, j - 2) + SWAP,
            )
        {
            if near(i - 1, j - 2) {
                out.push((i - 1, j - 2));
            }
            if near(i - 2, j - 1) {
                out.push((i - 2, j - 1));
            }
            i -= 2;
            j -= 2;
        } else if same(d[i][j], d[i - 1][j] + GAP) {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    out.reverse();
    out
}

/// Grips for the keyboard's tests: every grip calibrated, its taps landing
/// `(du, dv)` keyboard widths off.
#[cfg(test)]
pub mod tests_support {
    use super::*;

    pub fn shifted_grips(du: f32, dv: f32) -> Grips {
        let mut grips = Grips {
            stamp: 1,
            ..Grips::default()
        };
        for g in Grip::ALL {
            grips.set(
                g,
                Profile {
                    tilt: unit([0.0, 0.7, 0.7]).unwrap(),
                    tilt_sd: 0.1,
                    roll_a: 0.0,
                    roll_b: 0.0,
                    roll_sd: 0.1,
                    du: [du, 0.0, 0.0],
                    dv: [dv, 0.0, 0.0],
                    reach: None,
                    stretch: STRETCH_PRIOR,
                    pull: 0.0,
                    rows: Vec::new(),
                },
                1,
            );
        }
        grips
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tilt_for(g: Grip) -> Tilt {
        match g {
            Grip::Left => unit([-0.3, 0.8, 0.5]).unwrap(),
            Grip::Right => unit([0.3, 0.8, 0.5]).unwrap(),
            Grip::Both => unit([0.0, 0.6, 0.8]).unwrap(),
            Grip::Finger => unit([0.0, 0.05, 1.0]).unwrap(),
        }
    }

    /// A key grid like the phone's: 11 keys across, `h` = 0.5 widths tall.
    fn centers() -> HashMap<char, (f32, f32)> {
        let rows = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"];
        let mut m = HashMap::new();
        for (r, row) in rows.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                m.insert(
                    c,
                    (
                        (i as f32 + 0.5 + r as f32 * 0.3) / 11.0,
                        (r as f32 + 0.5) * 0.125,
                    ),
                );
            }
        }
        m.insert(' ', (0.5, 3.5 * 0.125));
        m
    }

    /// Calibrate with each grip tapping `shift` away from the key centers.
    /// Paint the area a thumb pivoting at `(0 or 1, 0.6)` reaches within 0.5.
    fn paint(cal: &mut Calibration, g: Grip) {
        for ring in 1..=5 {
            let rad = 0.1 * ring as f32;
            cal.arc_start();
            for k in 0..20 {
                let a = k as f32 / 19.0 * std::f32::consts::FRAC_PI_2;
                let u = rad * a.cos();
                let u = if g == Grip::Left { u } else { 1.0 - u };
                cal.arc_point(u, 0.6 - rad * a.sin(), Some(tilt_for(g)));
            }
            cal.arc_end();
        }
    }

    /// Calibrate every grip, one at a time.
    fn calibrated(shift: [(f32, f32); GRIPS]) -> Grips {
        let c = centers();
        let mut grips = Grips::default();
        for g in Grip::ALL {
            let known = Grip::ALL.map(|h| grips.reach(h));
            let mut cal = Calibration::new(PHRASE, g, known);
            if matches!(g, Grip::Left | Grip::Right) {
                assert_eq!(cal.step(), Some(Step::Area(g)));
                paint(&mut cal, g);
                cal.area_done();
                assert!(!cal.retry);
            }
            assert_eq!(cal.step(), Some(Step::Type(g)));
            let (du, dv) = shift[g.index()];
            let mut done = false;
            for ch in PHRASE.chars() {
                let (u, v) = c[&ch];
                done = cal.tap(u + du, v + dv, Some(tilt_for(g)), &c, 1.0 / 11.0);
            }
            assert!(done);
            cal.phrase_done(&c, 1.0 / 11.0);
            assert!(cal.done());
            grips.set(g, cal.finish(0.125).unwrap(), 7);
        }
        grips
    }

    #[test]
    fn calibration_learns_where_each_grip_lands() {
        let grips = calibrated([(0.01, 0.02), (-0.01, 0.02), (0.0, 0.01), (0.0, 0.0)]);
        let left = grips.get(Grip::Left).unwrap();
        let (du, dv) = left.offset(0.3, 0.2);
        assert!(
            (du - 0.01).abs() < 0.002 && (dv - 0.02).abs() < 0.002,
            "{du} {dv}"
        );
        // The reach: the far edge of the painted quarter disc (radius 0.5
        // about (0, 0.6)).
        let (cu, cv, r) = left.reach.unwrap();
        assert!(
            cu.abs() < 0.05 && (cv - 0.6).abs() < 0.05 && (r - 0.5).abs() < 0.05,
            "{cu} {cv} {r}"
        );
        let right = grips.get(Grip::Right).unwrap();
        assert!((right.reach.unwrap().0 - 1.0).abs() < 0.05);
        // Each key row reaches as far as the paint went at its height.
        let rows = grips.rows(Grip::Left).unwrap();
        assert!(rows[0].1 < rows[3].1, "{rows:?}");
        assert!(
            rows.iter().all(|&(a, b)| a < 0.05 && b > 0.1 && b <= 0.51),
            "{rows:?}"
        );
        assert_eq!(Grips::from_text(&grips.to_text()), grips);
    }

    #[test]
    fn the_tilt_tells_the_grips_apart() {
        let grips = calibrated([(0.0, 0.0); GRIPS]);
        for g in Grip::ALL {
            let mut s = Sense::default();
            for k in 0..40 {
                s.on_tilt(&grips, tilt_for(g));
                s.on_tap(&grips, 0.5, k * 300, false);
            }
            assert_eq!(s.shown(), Some(g));
        }
    }

    #[test]
    fn overlapping_taps_mean_two_thumbs() {
        let grips = calibrated([(0.0, 0.0); GRIPS]);
        let mut s = Sense::default();
        // A tilt halfway between the one-handed ones, but fingers overlap.
        let mid = unit([0.0, 0.8, 0.5]).unwrap();
        for k in 0..20 {
            s.on_tilt(&grips, mid);
            s.on_tap(
                &grips,
                if k % 2 == 0 { 0.2 } else { 0.8 },
                k * 120,
                k % 3 == 0,
            );
        }
        assert_eq!(s.shown(), Some(Grip::Both));
    }

    #[test]
    fn a_stray_tap_does_not_spoil_the_alignment() {
        let c = centers();
        let target: Vec<char> = "мягких".chars().collect();
        let mut taps: Vec<(f32, f32)> = target.iter().map(|ch| c[ch]).collect();
        taps.insert(2, c[&'ж']); // an extra tap
        let pairs = align(&taps, &target, &c, 1.0 / 11.0, &Pull::None);
        assert_eq!(pairs, vec![(0, 0), (1, 1), (3, 2), (4, 3), (5, 4), (6, 5)]);
    }

    #[test]
    fn a_missed_or_swapped_letter_shifts_nothing() {
        let c = centers();
        let target: Vec<char> = PHRASE.chars().collect();
        let key_w = 1.0 / 11.0;
        // «в чащх» — а missed: the taps stand for «в чащах» all the same.
        let taps: Vec<(f32, f32)> = "в чащх".chars().map(|ch| c[&ch]).collect();
        assert_eq!(prefix_end(&taps, &target, &c, key_w, &Pull::None), 7);
        // «в чащха юга» — swapped: every tap still pairs with its own letter.
        let typed: Vec<char> = "в чащха юга".chars().collect();
        let taps: Vec<(f32, f32)> = typed.iter().map(|ch| c[ch]).collect();
        let pairs = align(&taps, &target[..11], &c, key_w, &Pull::None);
        assert_eq!(pairs.len(), 11);
        assert!(
            pairs.iter().all(|&(i, j)| typed[i] == target[j]),
            "{pairs:?}"
        );
        // The phrase ends at its last letter, one missed on the way.
        let mut cal = Calibration {
            step: 1,
            ..Calibration::new(PHRASE, Grip::Left, [None; GRIPS])
        };
        let mut done = false;
        for (k, ch) in PHRASE.chars().enumerate() {
            if k != 5 {
                done = cal.tap(c[&ch].0, c[&ch].1, None, &c, key_w);
            }
        }
        assert!(done);
    }

    #[test]
    fn a_long_reach_that_falls_short_is_learned_not_dropped() {
        let c = centers();
        let key_w = 1.0 / 11.0;
        let mut cal = Calibration::new(PHRASE, Grip::Left, [None; GRIPS]);
        // The left thumb: pivot (0, 0.6), reach 0.5.
        paint(&mut cal, Grip::Left);
        cal.area_done();
        // Keys past the reach are hit short toward the pivot — up to 2.5
        // keys for the farthest; near ones dead on.
        let (pu, pv, r) = (0.0, 0.6, 0.5);
        let mut done = false;
        for ch in PHRASE.chars() {
            let (u, v) = c[&ch];
            let (eu, ev) = (pu - u, pv - v);
            let d = (eu * eu + ev * ev).sqrt();
            let past = (d / r - REACH_EDGE).max(0.0);
            let s = (0.35 * past).min(2.5 * key_w);
            done = cal.tap(
                u + s * eu / d,
                v + s * ev / d,
                Some(tilt_for(Grip::Left)),
                &c,
                key_w,
            );
        }
        assert!(done, "the phrase ends at its last letter");
        cal.phrase_done(&c, key_w);
        let s = &cal.samples[Grip::Left.index()];
        let far = s
            .offsets
            .iter()
            .filter(|o| o.2.hypot(o.3) > 1.5 * key_w)
            .count();
        assert!(far >= 5, "the long reaches are kept: {far}");
        // Learned: aimed at a far key, a tap lands short toward the pivot.
        let (_, _, pull) = fit_shift(&s.offsets, Some((pu, pv, r)));
        assert!(pull > 0.1, "{pull}");
    }

    #[test]
    fn each_grip_is_calibrated_and_undone_on_its_own() {
        let mut grips = calibrated([(0.0, 0.0); GRIPS]);
        assert!(grips.calibrated() && grips.has(Grip::Finger));
        // A new left calibration keeps the old one for an undo.
        let old = grips.get(Grip::Left).cloned();
        let mut newer = old.clone().unwrap();
        newer.du = [0.02, 0.0, 0.0];
        grips.set(Grip::Left, newer.clone(), 9);
        assert_eq!(grips.get(Grip::Left), Some(&newer));
        // Saved and read back, the previous one too.
        let back = Grips::from_text(&grips.to_text());
        assert_eq!(back, grips);
        grips.undo(Grip::Left, 10);
        assert_eq!(grips.get(Grip::Left).cloned(), old);
        assert_eq!(grips.undone(Grip::Left), 10);
        // Undo again: nothing before it — the grip is not calibrated now.
        grips.undo(Grip::Left, 11);
        assert!(!grips.has(Grip::Left));
        assert!(grips.has(Grip::Right) && grips.calibrated());
    }

    #[test]
    fn taps_near_the_edge_of_reach_spread_outward() {
        let grips = calibrated([(0.0, 0.0); GRIPS]);
        let left_only = [1.0, 0.0, 0.0, 0.0];
        // The left thumb pivots at (0, 0.6) and reaches 0.5: well inside, round.
        assert!(grips.spread(left_only, 0.1, 0.5).is_none());
        // Past its reach, to the right: stretched outward (away from the pivot).
        let ((ex, _), f) = grips.spread(left_only, 0.7, 0.5).unwrap();
        assert!(ex > 0.9 && f > 1.2, "{ex} {f}");
        // Two thumbs: no reach to speak of.
        assert!(grips.spread([0.0, 0.0, 1.0, 0.0], 0.7, 0.5).is_none());
    }

    #[test]
    fn a_stroke_up_the_middle_counts() {
        let mut cal = Calibration::new(PHRASE, Grip::Left, [None; GRIPS]);
        for k in 0..20 {
            let t = k as f32 / 19.0;
            cal.arc_point(0.55 + 0.05 * (t * 3.0).sin(), 0.55 - 0.5 * t, None);
        }
        cal.arc_end();
        assert_eq!(
            cal.step(),
            Some(Step::Area(Grip::Left)),
            "strokes until done"
        );
        cal.area_done();
        assert!(!cal.retry);
        assert_eq!(cal.step(), Some(Step::Type(Grip::Left)));
    }

    #[test]
    fn a_dab_is_not_an_area() {
        let mut cal = Calibration::new(PHRASE, Grip::Left, [None; GRIPS]);
        for k in 0..5 {
            cal.arc_point(0.1 + k as f32 * 0.01, 0.3, None);
        }
        cal.arc_end();
        cal.area_done();
        assert!(cal.retry);
        assert_eq!(cal.step(), Some(Step::Area(Grip::Left)));
    }

    #[test]
    fn each_row_gets_the_stretch_the_thumb_reaches() {
        // A right thumb that reaches neither the far left nor its own corner.
        let mut area = Vec::new();
        for r in 0..4 {
            let v = (r as f32 + 0.5) * 0.125;
            let (from, to) = (0.3 + 0.08 * (3 - r) as f32, 0.9);
            for i in 0..=20 {
                area.push((from + (to - from) * i as f32 / 20.0, v));
            }
        }
        let rows = row_spans(&area, 0.125);
        assert_eq!(rows.len(), 4);
        assert!(
            (rows[0].0 - 0.54).abs() < 0.02 && (rows[0].1 - 0.9).abs() < 0.02,
            "{rows:?}"
        );
        assert!(rows[3].0 < rows[0].0, "lower rows reach farther");
        let mut grips = calibrated([(0.0, 0.0); GRIPS]);
        grips.profiles[Grip::Right.index()].as_mut().unwrap().rows = rows.clone();
        assert_eq!(
            Grips::from_text(&grips.to_text()).rows(Grip::Right),
            Some(rows.as_slice())
        );
    }
}
