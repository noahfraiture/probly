use crate::error::Result;
use std::{
    cmp::max,
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3;

#[derive(Debug)]
pub struct UltraLogLog {
    precision: u8,
    state: Vec<u8>,
}

/// Allocates a new UltraLogLog sketch and returns an opaque pointer for C callers.
#[unsafe(no_mangle)]
pub extern "C" fn probly_ull_new(precision: u8) -> *mut UltraLogLog {
    Box::into_raw(Box::new(UltraLogLog::new(precision)))
}

/// Adds a byte slice to a sketch through the C ABI.
///
/// Returns `false` if `sketch` is null or if `value` is null while `len > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probly_ull_add_bytes(
    sketch: *mut UltraLogLog,
    value: *const u8,
    len: usize,
) -> bool {
    let Some(sketch) = (unsafe { sketch.as_mut() }) else {
        return false;
    };

    let bytes = if len == 0 {
        &[]
    } else {
        let Some(_) = (unsafe { value.as_ref() }) else {
            return false;
        };
        unsafe { std::slice::from_raw_parts(value, len) }
    };

    sketch.add_bytes(bytes);
    true
}

/// Merges `other` into `sketch` through the C ABI.
///
/// Returns `false` on null pointers or precision mismatch.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probly_ull_merge(
    sketch: *mut UltraLogLog,
    other: *const UltraLogLog,
) -> bool {
    let Some(sketch) = (unsafe { sketch.as_mut() }) else {
        return false;
    };
    let Some(other) = (unsafe { other.as_ref() }) else {
        return false;
    };

    sketch.merge(other).is_ok()
}

/// Returns the approximate distinct count for a sketch through the C ABI.
///
/// Null pointers return zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probly_ull_count(sketch: *const UltraLogLog) -> usize {
    let Some(sketch) = (unsafe { sketch.as_ref() }) else {
        return 0;
    };

    sketch.count()
}

/// Frees a sketch previously allocated by `probly_ull_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probly_ull_free(sketch: *mut UltraLogLog) {
    if sketch.is_null() {
        return;
    }

    unsafe {
        drop(Box::from_raw(sketch));
    }
}

impl Default for UltraLogLog {
    /// Creates the smallest possible sketch with a single logical register.
    fn default() -> Self {
        Self::new(0)
    }
}

impl UltraLogLog {
    /// Creates an empty UltraLogLog sketch with `2^precision` compact registers.
    /// Each register stores a packed prefix representation rather than a plain HLL value.
    pub fn new(precision: u8) -> Self {
        Self {
            precision,
            state: vec![0; 2usize.pow(precision as u32)],
        }
    }

    /// Hashes a typed value and inserts it into the sketch.
    /// This is equivalent to calling `add_bytes` on the value's hashed bytes.
    pub fn add<T: Hash>(&mut self, value: &T) {
        let mut hasher = xxh3::Xxh3Default::new();
        value.hash(&mut hasher);
        self.add_hashed_value(hasher.finish());
    }

    /// Hashes a byte slice and folds the result into the packed ULL state.
    /// The update preserves the mergeable semantics of the sketch.
    pub fn add_bytes(&mut self, value: &[u8]) {
        self.add_hashed_value(xxh3::xxh3_64(value));
    }

    /// Merges another ULL sketch with matching precision into this one.
    /// Packed register prefixes are combined with bitwise union and then repacked.
    pub fn merge(&mut self, other: &Self) -> Result<()> {
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
    pub fn count(&self) -> usize {
        if self.precision == 0 {
            return Self::estimate_cardinality(&self.state);
        }

        let registers = self.to_hll_registers();
        Self::estimate_cardinality(&registers)
    }

    /// Applies a pre-hashed value to the packed ULL state.
    /// Zero-precision sketches fall back to a single-register HLL-style update.
    fn add_hashed_value(&mut self, hash: u64) {
        if self.precision == 0 {
            let zeros = Self::rho(hash, 0);
            self.state[0] = max(self.state[0], zeros);
            return;
        }

        let index = Self::address(hash, self.precision);
        let nlz = Self::rho(hash, self.precision) - 1;
        let old_state = self.state[index];
        let shift = u32::from(nlz) + u32::from(self.precision) - 1;
        let hash_prefix = Self::unpack(old_state) | (1u64 << shift);
        self.state[index] = Self::pack(hash_prefix);
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

    /// Returns the register index encoded by the leading `precision` bits of the hash.
    /// A precision of zero collapses the sketch to a single register.
    fn address(hash: u64, precision: u8) -> usize {
        if precision == 0 {
            0
        } else {
            (hash >> (64 - precision)) as usize
        }
    }

    /// Computes `rho(w)`, the position of the first set bit after the register prefix.
    /// The result is one-based, matching the standard HLL definition.
    fn rho(hash: u64, precision: u8) -> u8 {
        let suffix = if precision == 0 {
            hash
        } else {
            (hash << precision) | (1u64 << (precision - 1))
        };
        suffix.leading_zeros() as u8 + 1
    }

    /// Estimates cardinality from a dense register array.
    /// Small ranges use linear counting; larger ranges use the raw harmonic-mean estimate.
    fn estimate_cardinality(registers: &[u8]) -> usize {
        if registers.is_empty() {
            return 0;
        }

        let m = registers.len() as f64;
        let estimate = Self::harmonic_mean(registers);
        let zero_registers = registers.iter().filter(|&&register| register == 0).count() as f64;

        if estimate <= 2.5 * m && zero_registers > 0.0 {
            Self::linear_counting(m, zero_registers).round() as usize
        } else {
            estimate.round() as usize
        }
    }

    /// Computes the raw harmonic-mean cardinality estimate for dense registers.
    fn harmonic_mean(registers: &[u8]) -> f64 {
        let m = registers.len() as f64;
        let sum: f64 = registers
            .iter()
            .map(|&register| 2f64.powi(-(register as i32)))
            .sum();
        Self::alpha(m) * m * m / sum
    }

    /// Computes the linear-counting correction for sparse occupancy.
    fn linear_counting(m: f64, zero_registers: f64) -> f64 {
        m * (m / zero_registers).ln()
    }

    /// Returns the bias-correction constant used by classic HyperLogLog counting.
    fn alpha(m: f64) -> f64 {
        match m as usize {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m),
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
    fn default_matches_new_with_zero_precision() {
        let default_value = UltraLogLog::default();
        let new_value = UltraLogLog::new(0);

        assert_eq!(default_value.precision, new_value.precision);
        assert_eq!(default_value.state, new_value.state);
    }

    #[test]
    fn address_uses_most_significant_precision_bits() {
        let hash = 0b1011u64 << 60;

        assert_eq!(UltraLogLog::address(hash, 4), 0b1011);
    }

    #[test]
    fn address_is_zero_when_precision_is_zero() {
        assert_eq!(UltraLogLog::address(u64::MAX, 0), 0);
    }

    #[test]
    fn leading_zeros_is_one_when_first_remaining_bit_is_set() {
        let hash = (0b0101u64 << 60) | (1u64 << 59);

        assert_eq!(UltraLogLog::rho(hash, 4), 1);
    }

    #[test]
    fn leading_zeros_counts_zero_bits_after_the_prefix() {
        let hash = (0b0101u64 << 60) | (1u64 << 57);

        assert_eq!(UltraLogLog::rho(hash, 4), 3);
    }

    #[test]
    fn leading_zeros_is_capped_when_remaining_bits_are_zero() {
        let hash = 0b0101u64 << 60;

        assert_eq!(UltraLogLog::rho(hash, 4), 61);
    }

    #[test]
    fn harmonic_mean_uses_special_alpha_for_sixteen_registers() {
        let registers = vec![0; 16];

        assert_close(UltraLogLog::harmonic_mean(&registers), 0.673 * 16.0);
    }

    #[test]
    fn linear_counting_matches_formula() {
        assert_close(
            UltraLogLog::linear_counting(16.0, 8.0),
            16.0 * (2.0f64).ln(),
        );
    }

    #[test]
    fn new_initializes_state_for_precision() {
        let value = UltraLogLog::new(4);

        assert_eq!(value.precision, 4);
        assert_eq!(value.state.len(), 16);
        assert!(value.state.iter().all(|&register| register == 0));
    }

    #[test]
    fn ffi_new_uses_requested_precision() {
        let sketch = probly_ull_new(4);

        assert!(!sketch.is_null());
        unsafe {
            assert_eq!((*sketch).precision, 4);
            assert_eq!((*sketch).state.len(), 16);
            probly_ull_free(sketch);
        }
    }

    #[test]
    fn ffi_add_bytes_and_count_follow_core_behavior() {
        let sketch = probly_ull_new(10);

        unsafe {
            assert!(probly_ull_add_bytes(
                sketch,
                b"alpha".as_ptr(),
                b"alpha".len()
            ));
            assert!(probly_ull_add_bytes(
                sketch,
                b"beta".as_ptr(),
                b"beta".len()
            ));
            assert!(probly_ull_add_bytes(
                sketch,
                b"alpha".as_ptr(),
                b"alpha".len()
            ));
            assert!(probly_ull_count(sketch) >= 2);
            probly_ull_free(sketch);
        }
    }

    #[test]
    fn ffi_merge_returns_false_for_precision_mismatch() {
        let left = probly_ull_new(8);
        let right = probly_ull_new(10);

        unsafe {
            assert!(!probly_ull_merge(left, right));
            probly_ull_free(left);
            probly_ull_free(right);
        }
    }

    #[test]
    fn ffi_functions_handle_null_pointers() {
        unsafe {
            assert!(!probly_ull_add_bytes(
                std::ptr::null_mut(),
                b"x".as_ptr(),
                1
            ));
            assert!(!probly_ull_add_bytes(
                probly_ull_new(4),
                std::ptr::null(),
                1
            ));
            assert!(!probly_ull_merge(std::ptr::null_mut(), std::ptr::null()));
            assert_eq!(probly_ull_count(std::ptr::null()), 0);
            probly_ull_free(std::ptr::null_mut());
        }
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
        let index = UltraLogLog::address(hash, 4);
        let expected = UltraLogLog::pack(1u64 << (u32::from(UltraLogLog::rho(hash, 4) - 1) + 3));

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

        value.add(&42_u64);
        let snapshot = value.state.clone();
        value.add(&42_u64);

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
            value.add(&i);
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
                left.add(&i);
            } else {
                right.add(&i);
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
                0 => a.add(&i),
                1 => b.add(&i),
                _ => c.add(&i),
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
            forward.add(&i);
        }

        for i in (0_u64..2_000).rev() {
            reverse.add(&i);
        }

        assert_eq!(forward.state, reverse.state);
        assert_eq!(forward.count(), reverse.count());
    }

    #[test]
    fn packed_registers_stay_canonical_after_updates() {
        let mut value = UltraLogLog::new(10);

        for i in 0_u64..5_000 {
            value.add(&i);
        }

        assert_canonical_state(&value.state);
    }

    #[test]
    fn packed_registers_stay_canonical_after_merge() {
        let mut left = UltraLogLog::new(10);
        let mut right = UltraLogLog::new(10);

        for i in 0_u64..4_000 {
            if i % 2 == 0 {
                left.add(&i);
            } else {
                right.add(&i);
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
            value.add(&i);
        }

        assert!(value.count() > 0);
    }

    #[test]
    fn count_ignores_duplicate_values() {
        let mut value = UltraLogLog::new(10);

        for i in 0_u64..128 {
            value.add(&i);
            value.add(&i);
            value.add(&i);
        }

        assert_count_within(value.count(), 128, 0.05);
    }

    #[test]
    fn count_is_monotonic_for_unique_stream() {
        let mut value = UltraLogLog::new(10);
        let mut previous = 0;

        for i in 0_u64..5_000 {
            value.add(&i);
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
            value.add(&i);
        }

        assert_count_within(value.count(), 10_000, 0.10);
    }

    #[test]
    fn count_is_close_for_large_stream() {
        let mut value = UltraLogLog::new(12);

        for i in 0_u64..100_000 {
            value.add(&i);
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
                value.add(&i);
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
            merged.add(&i);
            if i % 2 == 0 {
                left.add(&i);
            } else {
                right.add(&i);
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
            merged.add(&i);
            left.add(&i);
        }

        for i in 1_000_u64..2_000 {
            merged.add(&i);
            right.add(&i);
        }

        left.merge(&right).unwrap();

        assert_eq!(left.state, merged.state);
        assert_eq!(left.count(), merged.count());
        assert_count_within(left.count(), 2_000, 0.08);
    }
}
