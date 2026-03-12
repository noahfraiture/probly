#[derive(Default)]
struct HLL {
    precision: u8,
}

impl HLL {
    pub fn new(precision: u8) -> Self {
        Self { precision }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hll_has_zero_precision() {
        let value = HLL::default();
        assert_eq!(value.precision, 0);
    }

    #[test]
    fn new_sets_precision() {
        let value = HLL::new(12);
        assert_eq!(value.precision, 12);
    }
}
