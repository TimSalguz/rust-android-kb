//! The words' priors as an array by the word's number — for a dictionary
//! whose automaton gives each word's rank (its place in alphabetical
//! order) instead of its prior ([`crate::DictFormat::ranked`]).
//!
//! A prior differs from form to form, so kept in the automaton's outputs it
//! keeps the endings of different words from merging: the words alone are
//! 3.2 MB, with the grammar 4.6, with the priors too 9.1. Here the automaton
//! keeps the words and their grammar, and the priors sit apart: most words
//! were never seen in the counts and share the floor, the rest are packed a
//! block at a time.
//!
//! The words below any node of the automaton are a run of ranks, so the
//! least prior below it — what the search prunes on — is the least of a
//! range: minima of 16, 256, 4096, 65536 words make it a few dozen steps.
//!
//! The blob (little-endian): `b"KBWS"`, u32 version 1, u32 words, u32
//! quantum (thousandths of a nat a step), u32 floor (the step most words
//! share — the rarest, seen once or never — not stored); per block of 64 words a u64 bitmap of the seen ones, a u32 bit
//! offset into the packed steps, a u16 least step and a u16 width; the
//! packed steps (u64 words, least significant bit first); then each level
//! of minima (a byte each: half the least step, rounded down — a lower
//! bound).

const MAGIC: &[u8; 4] = b"KBWS";
const BLOCK: usize = 64;
/// Words under each minimum of the first level, and how many of a level
/// make one of the next.
const FAN: usize = 16;
const LEVELS: usize = 4;

pub struct Store<D: AsRef<[u8]>> {
    data: D,
    n: usize,
    quantum: u64,
    floor: u64,
    blocks: usize,
    packed: usize,
    /// Where each level of minima starts, and its length.
    levels: [(usize, usize); LEVELS],
}

impl<D: AsRef<[u8]>> Store<D> {
    /// Read a blob's layout (None: not one, or cut short).
    pub fn new(data: D) -> Option<Self> {
        let b = data.as_ref();
        if b.len() < 20 || &b[..4] != MAGIC || u32_at(b, 4) != 1 {
            return None;
        }
        let n = u32_at(b, 8) as usize;
        let quantum = u32_at(b, 12) as u64;
        let floor = u32_at(b, 16) as u64;
        let nb = n.div_ceil(BLOCK);
        let blocks = 20;
        let head_end = blocks + nb * 16;
        if b.len() < head_end {
            return None;
        }
        // The packed steps end where the last block's do.
        let packed_bits = (0..nb)
            .map(|i| {
                let h = blocks + i * 16;
                let seen = u64_at(b, h).count_ones() as usize;
                u32_at(b, h + 8) as usize + seen * u16_at(b, h + 14) as usize
            })
            .max()
            .unwrap_or(0);
        let packed = head_end;
        let mut at = packed + packed_bits.div_ceil(64) * 8;
        let mut levels = [(0, 0); LEVELS];
        let mut len = n.div_ceil(FAN);
        for level in &mut levels {
            *level = (at, len);
            at += len;
            len = len.div_ceil(FAN);
        }
        if b.len() != at {
            return None;
        }
        Some(Store {
            data,
            n,
            quantum,
            floor,
            blocks,
            packed,
            levels,
        })
    }

    /// Words in the store.
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The prior of the word of rank `r`, thousandths of a nat.
    pub fn prior(&self, r: u64) -> u64 {
        self.step(r as usize) * self.quantum
    }

    fn step(&self, r: usize) -> u64 {
        let b = self.data.as_ref();
        if r >= self.n {
            return self.floor;
        }
        let h = self.blocks + (r / BLOCK) * 16;
        let seen = u64_at(b, h);
        let bit = r % BLOCK;
        if seen >> bit & 1 == 0 {
            return self.floor;
        }
        let i = (seen & ((1u64 << bit) - 1)).count_ones() as usize;
        let (offset, least, width) = (
            u32_at(b, h + 8) as usize,
            u16_at(b, h + 12) as u64,
            u16_at(b, h + 14) as usize,
        );
        least + self.bits(offset + i * width, width)
    }

    /// The least step of ranks `lo..hi`, all in one block: its header read
    /// once.
    fn least_in_block(&self, lo: usize, hi: usize) -> u64 {
        let b = self.data.as_ref();
        let h = self.blocks + (lo / BLOCK) * 16;
        let seen = u64_at(b, h);
        let (a, z) = (lo % BLOCK, (hi - 1) % BLOCK + 1);
        let range = if z - a == 64 {
            u64::MAX
        } else {
            ((1u64 << (z - a)) - 1) << a
        };
        let mut m = if seen & range == range {
            u64::MAX
        } else {
            self.floor
        };
        let (offset, least, width) = (
            u32_at(b, h + 8) as usize,
            u16_at(b, h + 12) as u64,
            u16_at(b, h + 14) as usize,
        );
        let mut i = (seen & ((1u64 << a) - 1)).count_ones() as usize;
        let mut bits = seen & range;
        while bits != 0 {
            m = m.min(least + self.bits(offset + i * width, width));
            i += 1;
            bits &= bits - 1;
        }
        m
    }

    /// `width` bits at bit `at` of the packed steps.
    fn bits(&self, at: usize, width: usize) -> u64 {
        if width == 0 {
            return 0;
        }
        let b = self.data.as_ref();
        let word = |i: usize| u64_at(b, self.packed + i * 8);
        let (i, shift) = (at / 64, at % 64);
        let mut v = word(i) >> shift;
        if shift + width > 64 {
            v |= word(i + 1) << (64 - shift);
        }
        v & ((1u64 << width) - 1)
    }

    /// A lower bound of the priors of ranks `lo..hi` (thousandths of a
    /// nat): exact within two blocks, else the least of the minima of the
    /// 16-word groups it touches.
    pub fn least(&self, lo: u64, hi: u64) -> u64 {
        let (lo, hi) = (lo as usize, (hi as usize).min(self.n));
        if lo >= hi {
            return self.floor * self.quantum;
        }
        // Within a block or two: exact, from the blocks' own steps (few are
        // stored — most words share the floor).
        let (first, last) = (lo / BLOCK, (hi - 1) / BLOCK);
        if first == last {
            return self.least_in_block(lo, hi) * self.quantum;
        }
        if first + 1 == last {
            let cut = last * BLOCK;
            let m = self
                .least_in_block(lo, cut)
                .min(self.least_in_block(cut, hi));
            return m * self.quantum;
        }
        self.bound(lo as u64, hi as u64)
    }

    /// A quick lower bound of the priors of ranks `lo..hi` (thousandths of
    /// a nat), from the minima alone: a few bytes read. At most a step
    /// below [`Store::least`] for a run within a 16-word group; looser for a
    /// short run in a group of commoner words.
    pub fn bound(&self, lo: u64, hi: u64) -> u64 {
        let (lo, hi) = (lo as usize, (hi as usize).min(self.n));
        if lo >= hi {
            return self.floor * self.quantum;
        }
        let b = self.data.as_ref();
        let mut m = u8::MAX;
        let (mut a, mut z) = (lo / FAN, (hi - 1) / FAN + 1);
        for (level, &(start, len)) in self.levels.iter().enumerate() {
            let at = |i: usize| b[start + i.min(len - 1)];
            if level + 1 == LEVELS {
                for i in a..z {
                    m = m.min(at(i));
                }
                break;
            }
            while a < z && a % FAN != 0 {
                m = m.min(at(a));
                a += 1;
            }
            while a < z && z % FAN != 0 {
                z -= 1;
                m = m.min(at(z));
            }
            if a >= z {
                break;
            }
            a /= FAN;
            z /= FAN;
        }
        (m as u64 * 2).min(self.floor) * self.quantum
    }
}

/// A store's blob from each word's prior step (by rank), `quantum`
/// thousandths of a nat a step, `floor` the step most words share (not
/// stored).
pub fn build(steps: &[u16], quantum: u32, floor: u16) -> Vec<u8> {
    let n = steps.len();
    let mut out = MAGIC.to_vec();
    for x in [1u32, n as u32, quantum, floor as u32] {
        out.extend(x.to_le_bytes());
    }
    let mut packed: Vec<u64> = Vec::new();
    let mut bit = 0usize;
    let mut push = |v: u64, width: usize, bit: &mut usize| {
        if width == 0 {
            return;
        }
        let (i, shift) = (*bit / 64, *bit % 64);
        if packed.len() <= i + 1 {
            packed.resize(i + 2, 0);
        }
        packed[i] |= v << shift;
        if shift + width > 64 {
            packed[i + 1] |= v >> (64 - shift);
        }
        *bit += width;
    };
    for block in steps.chunks(BLOCK) {
        let mut seen = 0u64;
        let known: Vec<u16> = block
            .iter()
            .enumerate()
            .filter(|(_, &s)| s != floor)
            .map(|(i, &s)| {
                seen |= 1 << i;
                s
            })
            .collect();
        let least = known.iter().copied().min().unwrap_or(0);
        let most = known.iter().copied().max().unwrap_or(0);
        let width = (16 - (most - least).leading_zeros()) as usize;
        out.extend(seen.to_le_bytes());
        out.extend((bit as u32).to_le_bytes());
        out.extend(least.to_le_bytes());
        out.extend((width as u16).to_le_bytes());
        for s in known {
            push((s - least) as u64, width, &mut bit);
        }
    }
    packed.truncate(bit.div_ceil(64));
    for w in &packed {
        out.extend(w.to_le_bytes());
    }
    // Minima: half the least step (a lower bound), a byte each.
    let mut level: Vec<u16> = steps
        .chunks(FAN)
        .map(|g| g.iter().copied().min().unwrap_or(floor))
        .collect();
    for _ in 0..LEVELS {
        out.extend(level.iter().map(|&s| (s / 2).min(255) as u8));
        level = level
            .chunks(FAN)
            .map(|g| g.iter().copied().min().unwrap_or(floor))
            .collect();
    }
    out
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

impl Store<memmap2::Mmap> {
    /// Map a blob from a file (read-only, like the FSTs).
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the blob is a read-only, prebuilt asset; we don't mutate it.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Store::new(mmap)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a word store"))
    }
}

/// A ranked dictionary of `words` (prior in thousandths of a nat, a
/// multiple of 50) and its store, for tests.
#[cfg(test)]
pub(crate) fn ranked(words: &[(&str, u64)]) -> (fst::Map<Vec<u8>>, Store<Vec<u8>>) {
    let format = crate::DictFormat {
        quantum: 50,
        shift: 0,
        floor: 0,
        ranked: true,
    };
    let mut kv: Vec<(Vec<u8>, u64)> = words
        .iter()
        .map(|(w, p)| {
            let k = w.chars().map(|c| crate::alphabet::char_to_id(c).unwrap());
            (k.collect(), *p)
        })
        .collect();
    kv.sort();
    kv.dedup_by(|a, b| a.0 == b.0);
    let steps: Vec<u16> = kv.iter().map(|(_, p)| (p / 50) as u16).collect();
    let store = Store::new(build(&steps, 50, 500)).unwrap();
    let mut b = fst::raw::Builder::new_type(Vec::new(), format.fst_type()).unwrap();
    for (r, (k, _)) in kv.iter().enumerate() {
        b.insert(k, format.ranked_value(r as u64, 0)).unwrap();
    }
    (fst::Map::new(b.into_inner().unwrap()).unwrap(), store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priors_by_rank_and_the_least_of_a_run() {
        // 10 000 words: most unseen (the floor, 400), some seen.
        let floor = 400u16;
        let steps: Vec<u16> = (0..10_000u32)
            .map(|i| match i % 7 {
                0 => (i % 301) as u16,
                3 => 150 + (i % 50) as u16,
                _ => floor,
            })
            .collect();
        let store = Store::new(build(&steps, 50, floor)).unwrap();
        assert_eq!(store.len(), 10_000);
        for (r, &s) in steps.iter().enumerate() {
            assert_eq!(store.prior(r as u64), s as u64 * 50, "rank {r}");
        }
        // The least of any run is a lower bound, exact for short ones.
        for (lo, hi) in [
            (0, 1),
            (5, 9),
            (3, 19),
            (17, 5000),
            (100, 10_000),
            (9_990, 10_000),
        ] {
            let exact = steps[lo..hi].iter().copied().min().unwrap() as u64 * 50;
            let got = store.least(lo as u64, hi as u64);
            assert!(got <= exact, "{lo}..{hi}: {got} > {exact}");
            if (hi - 1) / BLOCK <= lo / BLOCK + 1 {
                assert_eq!(got, exact);
            }
        }
        // A run of unseen words only: the floor.
        let all_unseen = Store::new(build(&[floor; 100], 50, floor)).unwrap();
        assert_eq!(all_unseen.least(0, 100), floor as u64 * 50);
        assert!(Store::new(vec![0u8; 20]).is_none());
    }
}
