use crate::distinct_count::{address, estimate_cardinality, rho};
use crate::error::Result;
use std::{
    cmp::max,
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3;

pub struct UltraLogLog {
    precision: u8,
    state: Vec<u8>,
}

impl UltraLogLog {
    /// Creates an empty UltraLogLog sketch with `2^precision` compact registers.
    /// Each register stores a packed prefix representation rather than a plain HLL value.
    fn new(precision: u8) -> Self {
        Self {
            precision,
            state: vec![0; 2usize.pow(precision as u32)],
        }
    }

    /// Hashes a byte slice and folds the result into the packed ULL state.
    /// The update preserves the mergeable semantics of the sketch.
    fn add(&mut self, value: &[u8]) {
        self.add_hashed_value(xxh3::xxh3_64(value));
    }

    /// Hashes a typed value and inserts it into the sketch.
    /// This is equivalent to calling `add` on the value's hashed bytes.
    fn add_hash<T: Hash>(&mut self, value: &T) {
        let mut hasher = xxh3::Xxh3Default::new();
        value.hash(&mut hasher);
        self.add_hashed_value(hasher.finish());
    }

    /// Merges another ULL sketch with matching precision into this one.
    /// Packed register prefixes are combined with bitwise union and then repacked.
    fn merge(&mut self, other: &Self) -> Result<()> {
        if self.precision != other.precision {
            return Err(crate::error::Error::PrecisionMismatch {
                left: self.precision,
                right: other.precision,
            });
        }

        if self.precision == 0 {
            self.state[0] = max(self.state[0], other.state[0]);
            return Ok(());
        }

        for (left, right) in self.state.iter_mut().zip(&other.state) {
            *left = Self::pack(Self::unpack(*left) | Self::unpack(*right));
        }

        Ok(())
    }

    /// Estimates the distinct count represented by the sketch.
    /// The current implementation maps the packed ULL state back to HLL-style registers for estimation.
    fn count(&self) -> usize {
        if self.precision == 0 {
            return estimate_cardinality(&self.state);
        }

        let registers = self.to_hll_registers();
        estimate_cardinality(&registers)
    }
}

impl UltraLogLog {
    /// Applies a pre-hashed value to the packed ULL state.
    /// Zero-precision sketches fall back to a single-register HLL-style update.
    fn add_hashed_value(&mut self, hash: u64) {
        if self.precision == 0 {
            let zeros = rho(hash, 0);
            self.state[0] = max(self.state[0], zeros);
            return;
        }

        let index = address(hash, self.precision);
        let nlz = rho(hash, self.precision) - 1;
        let old_state = self.state[index];
        let shift = u32::from(nlz) + u32::from(self.precision) - 1;
        let hash_prefix = Self::unpack(old_state) | (1u64 << shift);
        self.state[index] = Self::pack(hash_prefix);
    }

    /// Expands a packed ULL register into the canonical prefix bit pattern it represents.
    /// A zero register denotes an empty state and expands to zero.
    fn unpack(register: u8) -> u64 {
        if register == 0 {
            return 0;
        }

        (4u64 | u64::from(register & 0b11)) << ((register >> 2) - 2)
    }

    /// Packs a canonical prefix bit pattern into the compact ULL register representation.
    /// The encoding keeps only the information needed for later merges and estimation.
    fn pack(hash_prefix: u64) -> u8 {
        if hash_prefix == 0 {
            return 0;
        }

        let nlz = hash_prefix.leading_zeros() + 1;
        (((0u32.wrapping_sub(nlz)) << 2) as u8) | (((hash_prefix << nlz) >> 62) as u8)
    }

    /// Converts packed ULL registers into HLL-style register values.
    /// This adapter is used by the current estimator to reuse the shared HLL counting path.
    fn to_hll_registers(&self) -> Vec<u8> {
        self.state
            .iter()
            .map(|&register| {
                let mapped = i16::from(register >> 2) + 2 - i16::from(self.precision);
                mapped.max(0) as u8
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Hll};

    fn assert_count_within(actual: usize, expected: usize, tolerance_ratio: f64) {
        let allowed_error = ((expected as f64) * tolerance_ratio).ceil() as usize;
        let lower = expected.saturating_sub(allowed_error);
        let upper = expected + allowed_error;
        assert!(
            (lower..=upper).contains(&actual),
            "expected {actual} to be within [{lower}, {upper}] for target {expected}"
        );
    }

    fn assert_canonical_state(state: &[u8]) {
        for &register in state {
            assert_eq!(
                UltraLogLog::pack(UltraLogLog::unpack(register)),
                register,
                "register {register} was not in canonical packed form"
            );
        }
    }

    #[test]
    fn new_initializes_state_for_precision() {
        let value = UltraLogLog::new(4);

        assert_eq!(value.precision, 4);
        assert_eq!(value.state.len(), 16);
        assert!(value.state.iter().all(|&register| register == 0));
    }

    #[test]
    fn pack_and_unpack_round_trip_preserves_canonical_state() {
        let prefix = 0b1011u64 << 60;

        let packed = UltraLogLog::pack(prefix);
        let repacked = UltraLogLog::pack(UltraLogLog::unpack(packed));

        assert_eq!(repacked, packed);
    }

    #[test]
    fn add_hashed_value_updates_expected_register() {
        let mut value = UltraLogLog::new(4);
        let hash = (0b0101u64 << 60) | (1u64 << 57);
        let index = address(hash, 4);
        let expected = UltraLogLog::pack(1u64 << (u32::from(rho(hash, 4) - 1) + 3));

        value.add_hashed_value(hash);

        assert_eq!(value.state[index], expected);
        assert_eq!(
            value
                .state
                .iter()
                .filter(|&&register| register != 0)
                .count(),
            1
        );
    }

    #[test]
    fn add_is_idempotent_for_duplicate_values() {
        let mut value = UltraLogLog::new(10);

        value.add_hash(&42_u64);
        let snapshot = value.state.clone();
        value.add_hash(&42_u64);

        assert_eq!(value.state, snapshot);
    }

    #[test]
    fn merge_rejects_precision_mismatch() {
        let mut left = UltraLogLog::new(4);
        let right = UltraLogLog::new(5);

        let err = left.merge(&right).unwrap_err();

        assert_eq!(err, Error::PrecisionMismatch { left: 4, right: 5 });
    }

    #[test]
    fn merge_is_idempotent() {
        let mut value = UltraLogLog::new(10);

        for i in 0_u64..2_000 {
            value.add_hash(&i);
        }

        let snapshot = value.state.clone();
        let other = UltraLogLog {
            precision: value.precision,
            state: value.state.clone(),
        };

        value.merge(&other).unwrap();

        assert_eq!(value.state, snapshot);
    }

    #[test]
    fn merge_is_commutative() {
        let mut left_then_right = UltraLogLog::new(10);
        let mut right_then_left = UltraLogLog::new(10);
        let mut left = UltraLogLog::new(10);
        let mut right = UltraLogLog::new(10);

        for i in 0_u64..2_000 {
            if i % 2 == 0 {
                left.add_hash(&i);
            } else {
                right.add_hash(&i);
            }
        }

        left_then_right.merge(&left).unwrap();
        left_then_right.merge(&right).unwrap();
        right_then_left.merge(&right).unwrap();
        right_then_left.merge(&left).unwrap();

        assert_eq!(left_then_right.state, right_then_left.state);
        assert_eq!(left_then_right.count(), right_then_left.count());
    }

    #[test]
    fn merge_is_associative() {
        let mut a = UltraLogLog::new(10);
        let mut b = UltraLogLog::new(10);
        let mut c = UltraLogLog::new(10);

        for i in 0_u64..3_000 {
            match i % 3 {
                0 => a.add_hash(&i),
                1 => b.add_hash(&i),
                _ => c.add_hash(&i),
            }
        }

        let mut left_grouped = UltraLogLog::new(10);
        left_grouped.merge(&a).unwrap();
        left_grouped.merge(&b).unwrap();
        left_grouped.merge(&c).unwrap();

        let mut bc = UltraLogLog::new(10);
        bc.merge(&b).unwrap();
        bc.merge(&c).unwrap();

        let mut right_grouped = UltraLogLog::new(10);
        right_grouped.merge(&a).unwrap();
        right_grouped.merge(&bc).unwrap();

        assert_eq!(left_grouped.state, right_grouped.state);
        assert_eq!(left_grouped.count(), right_grouped.count());
    }

    #[test]
    fn insertion_order_does_not_change_state() {
        let mut forward = UltraLogLog::new(10);
        let mut reverse = UltraLogLog::new(10);

        for i in 0_u64..2_000 {
            forward.add_hash(&i);
        }

        for i in (0_u64..2_000).rev() {
            reverse.add_hash(&i);
        }

        assert_eq!(forward.state, reverse.state);
        assert_eq!(forward.count(), reverse.count());
    }

    #[test]
    fn packed_registers_stay_canonical_after_updates() {
        let mut value = UltraLogLog::new(10);

        for i in 0_u64..5_000 {
            value.add_hash(&i);
        }

        assert_canonical_state(&value.state);
    }

    #[test]
    fn packed_registers_stay_canonical_after_merge() {
        let mut left = UltraLogLog::new(10);
        let mut right = UltraLogLog::new(10);

        for i in 0_u64..4_000 {
            if i % 2 == 0 {
                left.add_hash(&i);
            } else {
                right.add_hash(&i);
            }
        }

        left.merge(&right).unwrap();

        assert_canonical_state(&left.state);
    }

    #[test]
    fn count_is_zero_for_empty_sketch() {
        let value = UltraLogLog::new(8);

        assert_eq!(value.count(), 0);
    }

    #[test]
    fn count_handles_zero_precision() {
        let mut value = UltraLogLog::new(0);

        for i in 0_u64..64 {
            value.add_hash(&i);
        }

        assert!(value.count() > 0);
    }

    #[test]
    fn count_ignores_duplicate_values() {
        let mut value = UltraLogLog::new(10);

        for i in 0_u64..128 {
            value.add_hash(&i);
            value.add_hash(&i);
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 128, 0.05);
    }

    #[test]
    fn count_is_monotonic_for_unique_stream() {
        let mut value = UltraLogLog::new(10);
        let mut previous = 0;

        for i in 0_u64..5_000 {
            value.add_hash(&i);
            if i % 100 == 99 {
                let current = value.count();
                assert!(
                    current >= previous,
                    "count decreased from {previous} to {current}"
                );
                previous = current;
            }
        }
    }

    #[test]
    fn count_is_close_for_medium_stream() {
        let mut value = UltraLogLog::new(10);

        for i in 0_u64..10_000 {
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 10_000, 0.10);
    }

    #[test]
    fn count_is_close_for_large_stream() {
        let mut value = UltraLogLog::new(12);

        for i in 0_u64..100_000 {
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 100_000, 0.08);
    }

    #[test]
    fn count_is_close_across_precisions_and_cardinalities() {
        let cases = [
            (8_u8, 100_usize, 0.10_f64),
            (8, 1_000, 0.10),
            (8, 10_000, 0.10),
            (10, 100, 0.06),
            (10, 1_000, 0.08),
            (10, 10_000, 0.10),
            (12, 1_000, 0.05),
            (12, 10_000, 0.08),
            (12, 50_000, 0.08),
        ];

        for (precision, cardinality, tolerance) in cases {
            let mut value = UltraLogLog::new(precision);
            for i in 0..cardinality as u64 {
                value.add_hash(&i);
            }
            assert_count_within(value.count(), cardinality, tolerance);
        }
    }

    #[test]
    fn merged_sketch_matches_single_sketch() {
        let mut merged = UltraLogLog::new(10);
        let mut left = UltraLogLog::new(10);
        let mut right = UltraLogLog::new(10);

        for i in 0_u64..6_000 {
            merged.add_hash(&i);
            if i % 2 == 0 {
                left.add_hash(&i);
            } else {
                right.add_hash(&i);
            }
        }

        left.merge(&right).unwrap();

        assert_eq!(left.state, merged.state);
        assert_eq!(left.count(), merged.count());
    }

    #[test]
    fn merged_overlapping_stream_matches_single_sketch_union() {
        let mut merged = UltraLogLog::new(10);
        let mut left = UltraLogLog::new(10);
        let mut right = UltraLogLog::new(10);

        for i in 0_u64..1_500 {
            merged.add_hash(&i);
            left.add_hash(&i);
        }

        for i in 1_000_u64..2_000 {
            merged.add_hash(&i);
            right.add_hash(&i);
        }

        left.merge(&right).unwrap();

        assert_eq!(left.state, merged.state);
        assert_eq!(left.count(), merged.count());
        assert_count_within(left.count(), 2_000, 0.08);
    }

    #[test]
    fn ull_and_hll_stay_close_on_same_stream() {
        let mut ull = UltraLogLog::new(10);
        let mut hll = Hll::new(10);

        for i in 0_u64..20_000 {
            ull.add_hash(&i);
            hll.add_hash(&i);
        }

        let ull_count = ull.count();
        let hll_count = hll.count();
        let allowed_gap = ((hll_count as f64) * 0.05).ceil() as usize;

        assert!(
            ull_count.abs_diff(hll_count) <= allowed_gap,
            "expected ULL count {ull_count} to stay within {allowed_gap} of HLL count {hll_count}"
        );
    }
}
