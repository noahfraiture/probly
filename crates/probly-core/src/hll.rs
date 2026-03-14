use crate::Probly;
use crate::error::Result;
use std::{
    cmp::max,
    hash::{Hash, Hasher},
};
use xxhash_rust::xxh3;

pub struct Hll {
    // Number of bits to keep
    precision: u8,
    // Length = 2 ** precision
    registers: Vec<u8>,
}

impl Default for Hll {
    fn default() -> Self {
        Self {
            precision: 0,
            registers: vec![0; 1],
        }
    }
}

impl Probly for Hll {
    fn new(precision: u8) -> Self {
        Self {
            precision: precision,
            registers: vec![0; 2usize.pow(precision as u32)],
        }
    }

    fn add(&mut self, value: &[u8]) {
        let h = xxh3::xxh3_64(value);
        let address = self.address(h);
        let leading_zeros = self.leading_zeros(h);
        self.registers[address] = max(self.registers[address], leading_zeros);
    }

    fn add_hash<T: Hash>(&mut self, value: &T) {
        let mut hasher = xxh3::Xxh3Default::new();
        value.hash(&mut hasher);
        let h = hasher.finish();
        let address = self.address(h);
        let leading_zeros = self.leading_zeros(h);
        self.registers[address] = max(self.registers[address], leading_zeros);
    }

    fn merge(&mut self, other: &Self) -> Result<()> {
        if self.precision != other.precision {
            return Err(crate::error::Error::PrecisionMismatch {
                left: self.precision,
                right: other.precision,
            });
        }
        for (a, b) in self.registers.iter_mut().zip(&other.registers) {
            *a = max(*a, *b);
        }
        Ok(())
    }

    fn count(&self) -> usize {
        let m = self.registers.len() as f64;
        let estimate = self.harmonic_mean(m);

        let zero_registers = self
            .registers
            .iter()
            .filter(|&&register| register == 0)
            .count() as f64;

        // HLL is biased for small cardinalities. In this case, we use linear counting instead.
        if estimate <= 2.5 * m && zero_registers > 0.0 {
            self.linear_counting(m, zero_registers).round() as usize
        } else {
            estimate.round() as usize
        }
    }
}

impl Hll {
    // getting the first b bits (where b is log2(m)), and adding 1 to
    // them to obtain the address of the register to modify
    fn address(&self, h: u64) -> usize {
        if self.precision == 0 {
            0
        } else {
            (h >> (64 - self.precision)) as usize
        }
    }

    // With the remaining bits compute ρ(w) which returns the position of
    // the leftmost 1, where leftmost position is 1 (in other words: number
    // of leading zeros plus 1)
    fn leading_zeros(&self, h: u64) -> u8 {
        let w = if self.precision == 0 {
            h
        } else {
            (h << self.precision) | (1u64 << (self.precision - 1))
        };
        (w.leading_zeros() as u8) + 1
    }

    fn harmonic_mean(&self, m: f64) -> f64 {
        let sum: f64 = self
            .registers
            .iter()
            .map(|&register| 2f64.powi(-(register as i32)))
            .sum();
        let z = 1.0 / sum;
        // The constant alpha is hard to calculate. We can approximate it with
        // the following values.
        let alpha = match self.registers.len() {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m),
        };

        alpha * m * m * z
    }

    fn linear_counting(&self, m: f64, zero_registers: f64) -> f64 {
        m * (m / zero_registers).ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use std::collections::HashSet;

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

    fn hashed_address<T: Hash>(precision: u8, value: &T) -> usize {
        let mut hasher = xxh3::Xxh3Default::new();
        value.hash(&mut hasher);
        let hash = hasher.finish();
        let sketch = Hll::new(precision);
        sketch.address(hash)
    }

    fn byte_values_with_unique_addresses(precision: u8, count: usize) -> Vec<Vec<u8>> {
        let sketch = Hll::new(precision);
        let mut seen = HashSet::new();
        let mut values = Vec::with_capacity(count);
        let mut candidate = 0usize;

        while values.len() < count {
            let value = format!("value-{candidate}").into_bytes();
            let address = sketch.address(xxh3::xxh3_64(&value));
            if seen.insert(address) {
                values.push(value);
            }
            candidate += 1;
        }

        values
    }

    fn hashed_values_with_unique_addresses(precision: u8, count: usize) -> Vec<u64> {
        let mut seen = HashSet::new();
        let mut values = Vec::with_capacity(count);
        let mut candidate = 0u64;

        while values.len() < count {
            let address = hashed_address(precision, &candidate);
            if seen.insert(address) {
                values.push(candidate);
            }
            candidate += 1;
        }

        values
    }

    #[test]
    fn default_hll_has_zero_precision() {
        let value = Hll::default();
        assert_eq!(value.precision, 0);
        assert_eq!(value.registers, vec![0]);
    }

    #[test]
    fn default_matches_new_with_zero_precision() {
        let default_value = Hll::default();
        let new_value = Hll::new(0);

        assert_eq!(default_value.precision, new_value.precision);
        assert_eq!(default_value.registers, new_value.registers);
    }

    #[test]
    fn new_initializes_registers_for_precision() {
        let value = Hll::new(4);

        assert_eq!(value.precision, 4);
        assert_eq!(value.registers.len(), 16);
        assert!(value.registers.iter().all(|&register| register == 0));
    }

    #[test]
    fn new_with_zero_precision_creates_one_register() {
        let value = Hll::new(0);

        assert_eq!(value.precision, 0);
        assert_eq!(value.registers, vec![0]);
    }

    #[test]
    fn address_uses_most_significant_precision_bits() {
        let value = Hll::new(4);
        let hash = 0b1011u64 << 60;

        assert_eq!(value.address(hash), 0b1011);
    }

    #[test]
    fn address_is_zero_when_precision_is_zero() {
        let value = Hll::new(0);

        assert_eq!(value.address(u64::MAX), 0);
    }

    #[test]
    fn leading_zeros_is_one_when_first_remaining_bit_is_set() {
        let value = Hll::new(4);
        let hash = (0b0101u64 << 60) | (1u64 << 59);

        assert_eq!(value.leading_zeros(hash), 1);
    }

    #[test]
    fn leading_zeros_counts_zero_bits_after_the_prefix() {
        let value = Hll::new(4);
        let hash = (0b0101u64 << 60) | (1u64 << 57);

        assert_eq!(value.leading_zeros(hash), 3);
    }

    #[test]
    fn leading_zeros_is_capped_when_remaining_bits_are_zero() {
        let value = Hll::new(4);
        let hash = 0b0101u64 << 60;

        assert_eq!(value.leading_zeros(hash), 61);
    }

    #[test]
    fn harmonic_mean_uses_special_alpha_for_sixteen_registers() {
        let value = Hll::new(4);
        let m = value.registers.len() as f64;

        assert_close(value.harmonic_mean(m), 0.673 * 16.0);
    }

    #[test]
    fn harmonic_mean_uses_generic_alpha_for_other_register_counts() {
        let value = Hll::new(7);
        let m = value.registers.len() as f64;
        let expected = (0.7213 / (1.0 + 1.079 / m)) * m;

        assert_close(value.harmonic_mean(m), expected);
    }

    #[test]
    fn linear_counting_matches_formula() {
        let value = Hll::new(4);

        assert_close(value.linear_counting(16.0, 8.0), 16.0 * (2.0f64).ln());
    }

    #[test]
    fn add_updates_expected_register() {
        let mut value = Hll::new(4);
        let input = b"alpha";
        let hash = xxh3::xxh3_64(input);
        let address = value.address(hash);
        let leading_zeros = value.leading_zeros(hash);

        value.add(input);

        assert_eq!(value.registers[address], leading_zeros);
        assert_eq!(
            value
                .registers
                .iter()
                .filter(|&&register| register > 0)
                .count(),
            1
        );
    }

    #[test]
    fn add_does_not_decrease_an_existing_register_value() {
        let mut value = Hll::new(4);
        let input = b"alpha";
        let hash = xxh3::xxh3_64(input);
        let address = value.address(hash);
        let leading_zeros = value.leading_zeros(hash);

        value.registers[address] = leading_zeros + 3;
        value.add(input);

        assert_eq!(value.registers[address], leading_zeros + 3);
    }

    #[test]
    fn add_same_value_twice_is_idempotent() {
        let mut value = Hll::new(4);
        let input = b"alpha";

        value.add(input);
        let registers_after_first_add = value.registers.clone();
        value.add(input);

        assert_eq!(value.registers, registers_after_first_add);
    }

    #[test]
    fn add_hash_updates_expected_register() {
        let mut value = Hll::new(4);
        let input = 42_u64;
        let mut hasher = xxh3::Xxh3Default::new();
        input.hash(&mut hasher);
        let hash = hasher.finish();
        let address = value.address(hash);
        let leading_zeros = value.leading_zeros(hash);

        value.add_hash(&input);

        assert_eq!(value.registers[address], leading_zeros);
    }

    #[test]
    fn add_hash_same_value_twice_is_idempotent() {
        let mut value = Hll::new(4);

        value.add_hash(&42_u64);
        let registers_after_first_add = value.registers.clone();
        value.add_hash(&42_u64);

        assert_eq!(value.registers, registers_after_first_add);
    }

    #[test]
    fn merge_takes_register_wise_maximum() {
        let mut left = Hll::new(2);
        let mut right = Hll::new(2);

        left.registers = vec![1, 5, 0, 3];
        right.registers = vec![4, 2, 7, 3];

        left.merge(&right).unwrap();

        assert_eq!(left.registers, vec![4, 5, 7, 3]);
    }

    #[test]
    fn merge_rejects_precision_mismatch() {
        let mut left = Hll::new(1);
        let right = Hll::new(2);

        left.registers = vec![3, 1];

        let err = left.merge(&right).unwrap_err();

        assert_eq!(err, Error::PrecisionMismatch { left: 1, right: 2 });
        assert_eq!(left.registers, vec![3, 1]);
    }

    #[test]
    fn merge_with_empty_sketch_preserves_registers() {
        let mut left = Hll::new(4);
        left.add(b"alpha");
        left.add(b"beta");
        let registers_before_merge = left.registers.clone();
        let right = Hll::new(4);

        left.merge(&right).unwrap();

        assert_eq!(left.registers, registers_before_merge);
    }

    #[test]
    fn merge_is_idempotent() {
        let mut value = Hll::new(10);

        for i in 0_u64..500 {
            value.add_hash(&i);
        }

        let snapshot = value.registers.clone();
        let other = Hll {
            precision: value.precision,
            registers: value.registers.clone(),
        };

        value.merge(&other).unwrap();

        assert_eq!(value.registers, snapshot);
    }

    #[test]
    fn merge_is_commutative() {
        let mut left_then_right = Hll::new(10);
        let mut right_then_left = Hll::new(10);
        let mut left = Hll::new(10);
        let mut right = Hll::new(10);

        for i in 0_u64..1_000 {
            if i % 3 == 0 {
                left.add_hash(&i);
            } else {
                right.add_hash(&i);
            }
        }

        left_then_right.merge(&left).unwrap();
        left_then_right.merge(&right).unwrap();
        right_then_left.merge(&right).unwrap();
        right_then_left.merge(&left).unwrap();

        assert_eq!(left_then_right.registers, right_then_left.registers);
        assert_eq!(left_then_right.count(), right_then_left.count());
    }

    #[test]
    fn merge_is_associative() {
        let mut a = Hll::new(10);
        let mut b = Hll::new(10);
        let mut c = Hll::new(10);

        for i in 0_u64..1_500 {
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

        assert_eq!(left_grouped.registers, right_grouped.registers);
        assert_eq!(left_grouped.count(), right_grouped.count());
    }

    #[test]
    fn empty_hll_counts_zero() {
        let value = Hll::new(4);

        assert_eq!(value.count(), 0);
    }

    #[test]
    fn count_is_exact_for_small_unique_byte_values_without_collisions() {
        let values = byte_values_with_unique_addresses(10, 32);
        let mut value = Hll::new(10);

        for input in &values {
            value.add(input);
        }

        assert_count_within(value.count(), values.len(), 0.04);
    }

    #[test]
    fn count_is_exact_for_small_unique_hashed_values_without_collisions() {
        let values = hashed_values_with_unique_addresses(10, 32);
        let mut value = Hll::new(10);

        for input in &values {
            value.add_hash(input);
        }

        assert_count_within(value.count(), values.len(), 0.04);
    }

    #[test]
    fn count_ignores_duplicate_byte_values() {
        let values = byte_values_with_unique_addresses(10, 12);
        let mut value = Hll::new(10);

        for input in &values {
            value.add(input);
            value.add(input);
            value.add(input);
        }

        assert_eq!(value.count(), values.len());
    }

    #[test]
    fn count_ignores_duplicate_hashed_values() {
        let values = hashed_values_with_unique_addresses(10, 12);
        let mut value = Hll::new(10);

        for input in &values {
            value.add_hash(input);
            value.add_hash(input);
            value.add_hash(input);
        }

        assert_eq!(value.count(), values.len());
    }

    #[test]
    fn count_is_monotonic_for_unique_hashed_stream() {
        let mut value = Hll::new(10);
        let mut previous = value.count();

        for i in 0_u64..2_000 {
            value.add_hash(&i);
            if i % 50 == 49 {
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
    fn insertion_order_does_not_change_registers() {
        let mut forward = Hll::new(10);
        let mut reverse = Hll::new(10);

        for i in 0_u64..1_000 {
            forward.add_hash(&i);
        }

        for i in (0_u64..1_000).rev() {
            reverse.add_hash(&i);
        }

        assert_eq!(forward.registers, reverse.registers);
        assert_eq!(forward.count(), reverse.count());
    }

    #[test]
    fn count_is_close_for_medium_unique_byte_stream() {
        let mut value = Hll::new(10);

        for i in 0_u64..1_000 {
            let input = format!("value-{i}");
            value.add(input.as_bytes());
        }

        assert_count_within(value.count(), 1_000, 0.06);
    }

    #[test]
    fn count_is_close_for_medium_unique_hashed_stream() {
        let mut value = Hll::new(10);

        for i in 0_u64..1_000 {
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 1_000, 0.06);
    }

    #[test]
    fn count_is_close_for_large_unique_hashed_stream() {
        let mut value = Hll::new(12);

        for i in 0_u64..50_000 {
            value.add_hash(&i);
        }

        assert_count_within(value.count(), 50_000, 0.05);
    }

    #[test]
    fn count_is_close_across_precisions_and_cardinalities() {
        let cases = [
            (8_u8, 100_usize, 0.08_f64),
            (8, 1_000, 0.08),
            (8, 10_000, 0.08),
            (10, 100, 0.05),
            (10, 1_000, 0.06),
            (10, 10_000, 0.06),
            (12, 1_000, 0.03),
            (12, 10_000, 0.06),
            (12, 50_000, 0.05),
        ];

        for (precision, cardinality, tolerance) in cases {
            let mut value = Hll::new(precision);
            for i in 0..cardinality as u64 {
                value.add_hash(&i);
            }
            assert_count_within(value.count(), cardinality, tolerance);
        }
    }

    #[test]
    fn merged_sketch_matches_single_sketch_for_partitioned_byte_stream() {
        let mut merged = Hll::new(10);
        let mut left = Hll::new(10);
        let mut right = Hll::new(10);

        for i in 0_u64..2_000 {
            let input = format!("value-{i}");
            merged.add(input.as_bytes());
            if i % 2 == 0 {
                left.add(input.as_bytes());
            } else {
                right.add(input.as_bytes());
            }
        }

        left.merge(&right).unwrap();

        assert_eq!(left.registers, merged.registers);
        assert_eq!(left.count(), merged.count());
    }

    #[test]
    fn merged_sketch_matches_single_sketch_for_partitioned_hashed_stream() {
        let mut merged = Hll::new(10);
        let mut left = Hll::new(10);
        let mut right = Hll::new(10);

        for i in 0_u64..2_000 {
            merged.add_hash(&i);
            if i % 2 == 0 {
                left.add_hash(&i);
            } else {
                right.add_hash(&i);
            }
        }

        left.merge(&right).unwrap();

        assert_eq!(left.registers, merged.registers);
        assert_eq!(left.count(), merged.count());
    }

    #[test]
    fn merged_overlapping_stream_matches_single_sketch_union() {
        let mut merged = Hll::new(10);
        let mut left = Hll::new(10);
        let mut right = Hll::new(10);

        for i in 0_u64..1_500 {
            merged.add_hash(&i);
            left.add_hash(&i);
        }

        for i in 1_000_u64..2_000 {
            merged.add_hash(&i);
            right.add_hash(&i);
        }

        left.merge(&right).unwrap();

        assert_eq!(left.registers, merged.registers);
        assert_eq!(left.count(), merged.count());
        assert_count_within(left.count(), 2_000, 0.06);
    }
}
