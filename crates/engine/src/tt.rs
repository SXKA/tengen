//! Lock-free transposition table shared by all search threads.
//!
//! Each entry is two 64-bit atomics: the packed data, and the position key
//! XORed with that data. A torn write (key from one store, data from another)
//! fails the key check on probe, so threads never need a lock.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

use crate::board::Pos;
use crate::eval::Value;

/// How a stored value relates to the true score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Bound {
    None = 0,
    /// The true score is at most the value (fail low).
    Upper = 1,
    /// The true score is at least the value (fail high).
    Lower = 2,
    Exact = 3,
}

/// A decoded table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TtEntry {
    pub best_move: Option<Pos>,
    pub value: Value,
    pub eval: Value,
    pub depth: i32,
    pub bound: Bound,
    pub is_pv: bool,
}

impl TtEntry {
    fn pack(&self, generation: u8) -> u64 {
        let mv = self.best_move.map_or(u16::MAX, Pos::raw) as u64;
        let value = self.value as i16 as u16 as u64;
        let eval = self.eval as i16 as u16 as u64;
        let depth = (self.depth.clamp(-100, 127) as i8) as u8 as u64;
        let flags = self.bound as u64 | (self.is_pv as u64) << 2 | ((generation & 0x1f) as u64) << 3;
        mv | value << 16 | eval << 32 | depth << 48 | flags << 56
    }

    fn unpack(data: u64) -> TtEntry {
        let mv = data as u16;
        let flags = (data >> 56) as u8;
        TtEntry {
            best_move: (mv != u16::MAX).then(|| Pos::from_raw(mv)),
            value: (data >> 16) as u16 as i16 as Value,
            eval: (data >> 32) as u16 as i16 as Value,
            depth: (data >> 48) as u8 as i8 as i32,
            bound: match flags & 3 {
                1 => Bound::Upper,
                2 => Bound::Lower,
                3 => Bound::Exact,
                _ => Bound::None,
            },
            is_pv: flags & 4 != 0,
        }
    }
}

fn generation_of(data: u64) -> u8 {
    (data >> 59) as u8 & 0x1f
}

#[derive(Default)]
struct Slot {
    check: AtomicU64,
    data: AtomicU64,
}

const BUCKET: usize = 4;

/// One cache line of slots.
#[repr(align(64))]
#[derive(Default)]
struct Bucket {
    slots: [Slot; BUCKET],
}

/// The table itself.
pub struct TranspositionTable {
    buckets: Box<[Bucket]>,
    generation: AtomicU8,
}

impl TranspositionTable {
    /// A table using about `megabytes` of memory.
    pub fn new(megabytes: usize) -> TranspositionTable {
        let count = (megabytes.max(1) << 20) / std::mem::size_of::<Bucket>();
        TranspositionTable {
            buckets: (0..count).map(|_| Bucket::default()).collect(),
            generation: AtomicU8::new(0),
        }
    }

    /// Size in megabytes.
    pub fn megabytes(&self) -> usize {
        (self.buckets.len() * std::mem::size_of::<Bucket>()) >> 20
    }

    /// Forget everything.
    pub fn clear(&self) {
        for bucket in self.buckets.iter() {
            for slot in &bucket.slots {
                slot.check.store(0, Relaxed);
                slot.data.store(0, Relaxed);
            }
        }
    }

    /// Start a new search: older entries become preferred victims.
    pub fn new_search(&self) {
        self.generation.fetch_add(1, Relaxed);
    }

    fn generation(&self) -> u8 {
        self.generation.load(Relaxed) & 0x1f
    }

    #[inline]
    fn bucket(&self, key: u64) -> &Bucket {
        let index = ((key as u128 * self.buckets.len() as u128) >> 64) as usize;
        &self.buckets[index]
    }

    /// Start loading the bucket of `key` into the cache, ahead of a probe.
    #[inline]
    pub fn prefetch(&self, key: u64) {
        #[cfg(target_arch = "x86_64")]
        {
            let bucket: *const Bucket = self.bucket(key | 1);
            // SAFETY: a prefetch is only a hint and never faults; the pointer
            // is to a live bucket anyway.
            unsafe { std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(bucket.cast()) };
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = key;
    }

    /// Look up a position.
    #[inline]
    pub fn probe(&self, key: u64) -> Option<TtEntry> {
        let key = key | 1; // zero marks an empty slot
        for slot in &self.bucket(key).slots {
            let data = slot.data.load(Relaxed);
            if slot.check.load(Relaxed) ^ data == key {
                return Some(TtEntry::unpack(data));
            }
        }
        None
    }

    /// Store a result. Keeps an existing deeper result for the same position
    /// unless the new one is exact; otherwise evicts the shallowest, oldest
    /// entry of the bucket.
    pub fn store(&self, key: u64, mut entry: TtEntry) {
        let key = key | 1;
        let generation = self.generation();
        let bucket = self.bucket(key);
        let mut victim = &bucket.slots[0];
        let mut victim_worth = i32::MAX;
        for slot in &bucket.slots {
            let data = slot.data.load(Relaxed);
            let check = slot.check.load(Relaxed);
            if check ^ data == key {
                let old = TtEntry::unpack(data);
                if entry.best_move.is_none() {
                    entry.best_move = old.best_move;
                }
                if entry.bound != Bound::Exact && old.depth > entry.depth + 2 && generation_of(data) == generation {
                    return;
                }
                victim = slot;
                break;
            }
            let age = (generation.wrapping_sub(generation_of(data)) & 0x1f) as i32;
            let worth = if check == 0 && data == 0 {
                i32::MIN
            } else {
                TtEntry::unpack(data).depth - 4 * age
            };
            if worth < victim_worth {
                victim_worth = worth;
                victim = slot;
            }
        }
        let data = entry.pack(generation);
        victim.data.store(data, Relaxed);
        victim.check.store(key ^ data, Relaxed);
    }

    /// Permille of sampled slots written during the current search.
    pub fn hashfull(&self) -> u32 {
        let generation = self.generation();
        let sample = self.buckets.len().min(1000);
        let used = self.buckets[..sample]
            .iter()
            .flat_map(|b| &b.slots)
            .filter(|s| {
                let data = s.data.load(Relaxed);
                data != 0 && generation_of(data) == generation
            })
            .count();
        (used * 1000 / (sample * BUCKET).max(1)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip() {
        let tt = TranspositionTable::new(1);
        let entry = TtEntry {
            best_move: Some(Pos::new(3, 17)),
            value: -29_950,
            eval: 123,
            depth: -3,
            bound: Bound::Lower,
            is_pv: true,
        };
        tt.store(0xdead_beef_0000_1234, entry);
        assert_eq!(tt.probe(0xdead_beef_0000_1234), Some(entry));
        assert_eq!(tt.probe(0xdead_beef_0000_1236), None);
        let empty = TtEntry {
            best_move: None,
            ..entry
        };
        tt.store(42, empty);
        assert_eq!(tt.probe(42).unwrap().best_move, None);
        tt.clear();
        assert_eq!(tt.probe(0xdead_beef_0000_1234), None);
    }

    #[test]
    fn keeps_moves_and_deeper_results() {
        let tt = TranspositionTable::new(1);
        let deep = TtEntry {
            best_move: Some(Pos::new(1, 1)),
            value: 5,
            eval: 0,
            depth: 12,
            bound: Bound::Lower,
            is_pv: false,
        };
        tt.store(7, deep);
        tt.store(
            7,
            TtEntry {
                depth: 2,
                best_move: None,
                ..deep
            },
        );
        assert_eq!(tt.probe(7), Some(deep));
        tt.store(
            7,
            TtEntry {
                depth: 2,
                best_move: None,
                bound: Bound::Exact,
                ..deep
            },
        );
        let got = tt.probe(7).unwrap();
        assert_eq!((got.depth, got.best_move), (2, Some(Pos::new(1, 1))));
    }
}
