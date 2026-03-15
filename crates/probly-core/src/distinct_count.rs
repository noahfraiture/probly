/// Returns the register index encoded by the leading `precision` bits of the hash.
/// A precision of zero collapses the sketch to a single register.
pub(crate) fn address(hash: u64, precision: u8) -> usize {
    if precision == 0 {
        0
    } else {
        (hash >> (64 - precision)) as usize
    }
}

/// Computes `rho(w)`, the position of the first set bit after the register prefix.
/// The result is one-based, matching the standard HLL definition.
pub(crate) fn rho(hash: u64, precision: u8) -> u8 {
    let suffix = if precision == 0 {
        hash
    } else {
        (hash << precision) | (1u64 << (precision - 1))
    };
    suffix.leading_zeros() as u8 + 1
}

/// Returns the bias-correction constant used by classic HyperLogLog.
/// Small register counts use the original tabulated constants and larger ones use the asymptotic form.
pub(crate) fn alpha(m: f64) -> f64 {
    match m as usize {
        16 => 0.673,
        32 => 0.697,
        64 => 0.709,
        _ => 0.7213 / (1.0 + 1.079 / m),
    }
}

/// Computes the raw HLL estimate from a dense register array.
/// This is the harmonic-mean estimator before any small-range correction is applied.
pub(crate) fn harmonic_mean(registers: &[u8]) -> f64 {
    let m = registers.len() as f64;
    let sum: f64 = registers
        .iter()
        .map(|&register| 2f64.powi(-(register as i32)))
        .sum();
    alpha(m) * m * m / sum
}

/// Computes the linear-counting estimate for sparse occupancy.
/// This correction is more accurate than raw HLL when many registers are still zero.
pub(crate) fn linear_counting(m: f64, zero_registers: f64) -> f64 {
    m * (m / zero_registers).ln()
}

/// Estimates cardinality from a dense register array using classic HLL rules.
/// It switches to linear counting for small cardinalities and otherwise uses the raw HLL estimate.
pub(crate) fn estimate_cardinality(registers: &[u8]) -> usize {
    if registers.is_empty() {
        return 0;
    }

    let m = registers.len() as f64;
    let estimate = harmonic_mean(registers);
    let zero_registers = registers.iter().filter(|&&register| register == 0).count() as f64;

    if estimate <= 2.5 * m && zero_registers > 0.0 {
        linear_counting(m, zero_registers).round() as usize
    } else {
        estimate.round() as usize
    }
}
