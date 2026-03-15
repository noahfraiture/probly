use bitvec::prelude::*;
use std::hash::{Hash, Hasher};
use xxhash_rust::xxh3;

pub struct Bloom {
    hash_seeds: Vec<u64>,
    bits: BitVec,
}

impl Bloom {
    /// Creates an empty Bloom filter with `m` bits and `k` deterministic hash seeds.
    ///
    /// A zero-sized filter is allowed, but inserts become no-ops and lookups always return `false`.
    pub fn new(m: usize, k: usize) -> Self {
        Self {
            hash_seeds: (0..k).map(|seed| seed as u64).collect(),
            bits: bitvec![0; m],
        }
    }

    /// Hashes a raw byte slice with each configured seed and sets the corresponding bits.
    pub fn add_bytes(&mut self, value: &[u8]) {
        let bit_count = self.bits.len() as u64;
        if bit_count == 0 {
            return;
        }

        for &seed in &self.hash_seeds {
            let bit = xxh3::xxh3_64_with_seed(value, seed) % bit_count;
            self.bits.set(bit as usize, true);
        }
    }

    /// Hashes a typed value with each configured seed and sets the corresponding bits.
    pub fn add<T: Hash>(&mut self, value: &T) {
        let bit_count = self.bits.len() as u64;
        if bit_count == 0 {
            return;
        }

        for &seed in &self.hash_seeds {
            let mut hasher = xxh3::Xxh3::with_seed(seed);
            value.hash(&mut hasher);
            let bit = hasher.finish() % bit_count;
            self.bits.set(bit as usize, true);
        }
    }

    /// Unions another Bloom filter into this one.
    ///
    /// The filters must use the same seeded hash family; otherwise a precision mismatch error is returned.
    pub fn merge(&mut self, other: &Self) -> crate::Result<()> {
        let different_hash = self
            .hash_seeds
            .iter()
            .zip(other.hash_seeds.iter())
            .any(|(a, b)| a != b);
        if different_hash {
            return Err(crate::error::Error::PrecisionMismatch {
                left: self.hash_seeds.len() as u8,
                right: other.hash_seeds.len() as u8,
            });
        }

        for bit in other.bits.iter_ones() {
            self.bits.set(bit as usize, true);
        }

        Ok(())
    }

    /// Checks whether a raw byte slice may be present in the filter.
    ///
    /// `true` means "possibly present" and `false` means "definitely not present".
    pub fn contains_bytes(&self, value: &[u8]) -> bool {
        self.hash_seeds.iter().all(|seed| {
            let mut hasher = xxh3::Xxh3::with_seed(*seed);
            hasher.update(value);
            let bit = hasher.finish() % self.bits.len() as u64;
            self.bits.get(bit as usize).map(|bit| *bit).unwrap_or(false)
        })
    }

    /// Checks whether a typed value may be present in the filter.
    ///
    /// `true` means "possibly present" and `false` means "definitely not present".
    pub fn contains<T: Hash>(&self, value: &T) -> bool {
        self.hash_seeds.iter().all(|seed| {
            let mut hasher = xxh3::Xxh3::with_seed(*seed);
            value.hash(&mut hasher);
            let bit = hasher.finish() % self.bits.len() as u64;
            self.bits.get(bit as usize).map(|bit| *bit).unwrap_or(false)
        })
    }
}
