use crate::distinct_count::{address, estimate_cardinality, rho};
use crate::error::Result;
use std::{
    cmp::max,
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Storage {
    Sparse(Vec<u64>),
    Dense(Vec<u8>),
}

pub struct Hll {
    precision: u8,
    storage: Storage,
}

impl Default for Hll {
    /// Creates the smallest possible sketch with a single logical register.
    /// This matches `Hll::new(0)` so default construction stays predictable.
    fn default() -> Self {
        Self::new(0)
    }
}

impl Hll {
    /// Creates an empty HLL sketch in sparse mode.
    /// The sketch stays sparse until enough coupons accumulate to justify dense registers.
    fn new(precision: u8) -> Self {
        Self {
            precision,
            storage: Storage::Sparse(Vec::new()),
        }
    }

    /// Hashes a byte slice and inserts the resulting coupon or register update.
    /// Sparse sketches record distinct coupons, while dense sketches update registers in place.
    fn add(&mut self, value: &[u8]) {
        self.add_hashed_value(xxh3::xxh3_64(value));
    }

    /// Hashes a typed value and inserts it into the sketch.
    /// This follows the same update path as `add` after hashing.
    fn add_hash<T: Hash>(&mut self, value: &T) {
        let mut hasher = xxh3::Xxh3Default::new();
        value.hash(&mut hasher);
        self.add_hashed_value(hasher.finish());
    }

    /// Merges another HLL sketch with matching precision into this one.
    /// Sparse sketches merge by coupon union, and any dense participant forces dense materialization.
    fn merge(&mut self, other: &Self) -> Result<()> {
        if self.precision != other.precision {
            return Err(crate::error::Error::PrecisionMismatch {
                left: self.precision,
                right: other.precision,
            });
        }

        let precision = self.precision;
        let sparse_threshold = self.sparse_threshold();
        match (&mut self.storage, &other.storage) {
            (Storage::Sparse(left), Storage::Sparse(right)) => {
                for &coupon in right {
                    Self::insert_sparse_coupon(left, coupon);
                }

                if left.len() > sparse_threshold {
                    let dense = Self::dense_from_sparse(precision, left);
                    self.storage = Storage::Dense(dense);
                }
            }
            _ => {
                let right_dense = other.dense_registers();
                let left_dense = self.materialize_dense();
                for (left, right) in left_dense.iter_mut().zip(right_dense) {
                    *left = max(*left, right);
                }
            }
        }

        Ok(())
    }

    /// Estimates the distinct count represented by the sketch.
    /// Sparse storage is materialized logically into dense registers before estimation.
    fn count(&self) -> usize {
        let registers = self.dense_registers();
        estimate_cardinality(&registers)
    }
}

impl Hll {
    /// Updates the sketch from a pre-hashed 64-bit value.
    /// This keeps sparse mode efficient and promotes to dense mode when the coupon set grows too large.
    fn add_hashed_value(&mut self, hash: u64) {
        let precision = self.precision;
        let sparse_threshold = self.sparse_threshold();
        match &mut self.storage {
            Storage::Sparse(coupons) => {
                let coupon = Self::coupon_for_precision(precision, hash);
                if Self::insert_sparse_coupon(coupons, coupon) && coupons.len() > sparse_threshold {
                    let dense = Self::dense_from_sparse(precision, coupons);
                    self.storage = Storage::Dense(dense);
                }
            }
            Storage::Dense(registers) => Self::apply_hash(registers, precision, hash),
        }
    }

    /// Applies a hashed value directly to a dense register array.
    /// The register update is the standard HLL register-wise maximum.
    fn apply_hash(registers: &mut [u8], precision: u8, hash: u64) {
        let index = address(hash, precision);
        let zeros = rho(hash, precision);
        registers[index] = max(registers[index], zeros);
    }

    /// Encodes a hashed value as a sparse coupon.
    /// The coupon stores the register index together with the observed `rho` value.
    fn coupon_for_precision(precision: u8, hash: u64) -> u64 {
        ((address(hash, precision) as u64) << 8) | u64::from(rho(hash, precision))
    }

    /// Returns the maximum coupon count tolerated before switching to dense storage.
    /// The threshold is a simple size heuristic rather than a byte-exact HLL++ sparse encoding limit.
    fn sparse_threshold(&self) -> usize {
        let dense_len = 2usize.pow(self.precision as u32);
        max(32, dense_len / 8)
    }

    /// Inserts a coupon into the sorted sparse set if it is not already present.
    /// The return value indicates whether the sparse state changed.
    fn insert_sparse_coupon(coupons: &mut Vec<u64>, coupon: u64) -> bool {
        match coupons.binary_search(&coupon) {
            Ok(_) => false,
            Err(index) => {
                coupons.insert(index, coupon);
                true
            }
        }
    }

    /// Expands sparse coupons into a dense register array.
    /// Multiple coupons targeting the same register are reduced with a register-wise maximum.
    fn dense_from_sparse(precision: u8, coupons: &[u64]) -> Vec<u8> {
        let mut registers = vec![0; 2usize.pow(precision as u32)];
        for &coupon in coupons {
            let index = (coupon >> 8) as usize;
            let zeros = (coupon & 0xff) as u8;
            registers[index] = max(registers[index], zeros);
        }
        registers
    }

    /// Returns a dense register view of the sketch state.
    /// Sparse sketches are converted on the fly, while dense sketches are cloned directly.
    fn dense_registers(&self) -> Vec<u8> {
        match &self.storage {
            Storage::Sparse(coupons) => Self::dense_from_sparse(self.precision, coupons),
            Storage::Dense(registers) => registers.clone(),
        }
    }

    /// Ensures the internal state is stored densely and returns the register array.
    /// This is used by merge paths that need in-place register mutation.
    fn materialize_dense(&mut self) -> &mut Vec<u8> {
        if matches!(self.storage, Storage::Sparse(_)) {
            let dense = match &self.storage {
                Storage::Sparse(coupons) => Self::dense_from_sparse(self.precision, coupons),
                Storage::Dense(_) => unreachable!(),
            };
            self.storage = Storage::Dense(dense);
        }

        match &mut self.storage {
            Storage::Dense(registers) => registers,
            Storage::Sparse(_) => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    fn assert_close(actual: f64, expected: f64) {
        let diff = (actual - expected).abs();
        assert!(
            diff < 1e-12,
            "expected {expected}, got {actual}, diff was {diff}"
        );
    }

    fn assert_count_within(actual: usize, expected: usize, tolerance_ratio: f64) {
        let allowed_error = ((expected as f64) * tolerance_ratio).ceil() as usize;
        let lower = expected.saturating_sub(allowed_error);
        let upper = expected + allowed_error;
        assert!(
            (lower..=upper).contains(&actual),
            "expected {actual} to be within [{lower}, {upper}] for target {expected}"
        );
    }

    #[test]
    fn default_matches_new_with_zero_precision() {
        let default_value = Hll::default();
        let new_value = Hll::new(0);

        assert_eq!(default_value.precision, new_value.precision);
        assert_eq!(default_value.storage, new_value.storage);
    }

    #[test]
    fn new_initializes_sparse_storage() {
        let value = Hll::new(10);

        assert_eq!(value.precision, 10);
        assert_eq!(value.storage, Storage::Sparse(Vec::new()));
    }

    #[test]
    fn address_uses_most_significant_precision_bits() {
        let hash = 0b1011u64 << 60;

        assert_eq!(address(hash, 4), 0b1011);
    }

    #[test]
    fn address_is_zero_when_precision_is_zero() {
        assert_eq!(address(u64::MAX, 0), 0);
    }

    #[test]
    fn leading_zeros_is_one_when_first_remaining_bit_is_set() {
        let hash = (0b0101u64 << 60) | (1u64 << 59);

        assert_eq!(rho(hash, 4), 1);
    }

    #[test]
    fn leading_zeros_counts_zero_bits_after_the_prefix() {
        let hash = (0b0101u64 << 60) | (1u64 << 57);

        assert_eq!(rho(hash, 4), 3);
    }

    #[test]
    fn leading_zeros_is_capped_when_remaining_bits_are_zero() {
        let hash = 0b0101u64 << 60;

        assert_eq!(rho(hash, 4), 61);
    }

    #[test]
    fn harmonic_mean_uses_special_alpha_for_sixteen_registers() {
        let registers = vec![0; 16];

        assert_close(
            crate::distinct_count::harmonic_mean(&registers),
            0.673 * 16.0,
        );
    }

    #[test]
    fn linear_counting_matches_formula() {
        assert_close(
            crate::distinct_count::linear_counting(16.0, 8.0),
            16.0 * (2.0f64).ln(),
        );
    }

    #[test]
    fn add_keeps_small_sketch_sparse() {
        let mut value = Hll::new(10);

        for i in 0_u64..32 {
            value.add_hash(&i);
        }

        assert!(matches!(value.storage, Storage::Sparse(_)));
    }

    #[test]
    fn add_transitions_to_dense_when_sparse_threshold_is_exceeded() {
        let mut value = Hll::new(10);

        for i in 0_u64..1_000 {
            value.add_hash(&i);
        }

        assert!(matches!(value.storage, Storage::Dense(_)));
    }

    #[test]
    fn add_same_value_twice_is_idempotent() {
        let mut value = Hll::new(10);

        value.add_hash(&42_u64);
        let snapshot = value.dense_registers();
        value.add_hash(&42_u64);

        assert_eq!(value.dense_registers(), snapshot);
    }

    #[test]
    fn merge_takes_register_wise_maximum_after_materialization() {
        let mut left = Hll::new(4);
        let mut right = Hll::new(4);

        left.storage = Storage::Dense(vec![1, 5, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        right.storage = Storage::Dense(vec![4, 2, 7, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        left.merge(&right).unwrap();

        assert_eq!(
            left.dense_registers(),
            vec![4, 5, 7, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn merge_rejects_precision_mismatch() {
        let mut left = Hll::new(4);
        let right = Hll::new(5);

        let err = left.merge(&right).unwrap_err();

        assert_eq!(err, Error::PrecisionMismatch { left: 4, right: 5 });
    }

    #[test]
    fn merge_is_commutative() {
        let mut left_then_right = Hll::new(10);
        let mut right_then_left = Hll::new(10);
        let mut left = Hll::new(10);
        let mut right = Hll::new(10);

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

        assert_eq!(
            left_then_right.dense_registers(),
            right_then_left.dense_registers()
        );
        assert_eq!(left_then_right.count(), right_then_left.count());
    }

    #[test]
    fn merge_is_associative() {
        let mut a = Hll::new(10);
        let mut b = Hll::new(10);
        let mut c = Hll::new(10);

        for i in 0_u64..3_000 {
            match i % 3 {
                0 => a.add_hash(&i),
                1 => b.add_hash(&i),
                _ => c.add_hash(&i),
            }
        }

        let mut left_grouped = Hll::new(10);
        left_grouped.merge(&a).unwrap();
        left_grouped.merge(&b).unwrap();
        left_grouped.merge(&c).unwrap();

        let mut bc = Hll::new(10);
        bc.merge(&b).unwrap();
        bc.merge(&c).unwrap();

        let mut right_grouped = Hll::new(10);
        right_grouped.merge(&a).unwrap();
        right_grouped.merge(&bc).unwrap();

        assert_eq!(
            left_grouped.dense_registers(),
            right_grouped.dense_registers()
        );
        assert_eq!(left_grouped.count(), right_grouped.count());
    }

    #[test]
    fn insertion_order_does_not_change_registers() {
        let mut forward = Hll::new(10);
        let mut reverse = Hll::new(10);

        for i in 0_u64..2_000 {
            forward.add_hash(&i);
        }

        for i in (0_u64..2_000).rev() {
            reverse.add_hash(&i);
        }

        assert_eq!(forward.dense_registers(), reverse.dense_registers());
        assert_eq!(forward.count(), reverse.count());
    }

    #[test]
    fn empty_hll_counts_zero() {
        let value = Hll::new(10);

        assert_eq!(value.count(), 0);
    }

    #[test]
    fn count_ignores_duplicate_values() {
        let mut value = Hll::new(10);

        for i in 0_u64..128 {
            value.add_hash(&i);
            value.add_hash(&i);
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 128, 0.05);
    }

    #[test]
    fn count_is_monotonic_for_unique_stream() {
        let mut value = Hll::new(10);
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
    fn count_is_close_for_medium_unique_stream() {
        let mut value = Hll::new(10);

        for i in 0_u64..10_000 {
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 10_000, 0.07);
    }

    #[test]
    fn count_is_close_for_large_unique_stream() {
        let mut value = Hll::new(12);

        for i in 0_u64..100_000 {
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 100_000, 0.06);
    }

    #[test]
    fn merged_sketch_matches_single_sketch_for_partitioned_stream() {
        let mut merged = Hll::new(10);
        let mut left = Hll::new(10);
        let mut right = Hll::new(10);

        for i in 0_u64..6_000 {
            merged.add_hash(&i);
            if i % 2 == 0 {
                left.add_hash(&i);
            } else {
                right.add_hash(&i);
            }
        }

        left.merge(&right).unwrap();

        assert_eq!(left.dense_registers(), merged.dense_registers());
        assert_eq!(left.count(), merged.count());
    }
}
