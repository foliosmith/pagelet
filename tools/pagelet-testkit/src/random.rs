/// Small deterministic generator for reproducible tests.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    /// Create a deterministic generator from a seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Return the next pseudo-random `u64`.
    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    /// Return a value in `0..upper`.
    #[must_use]
    pub fn bounded(&mut self, upper: u64) -> u64 {
        assert!(upper > 0, "upper bound must be non-zero");
        self.next_u64() % upper
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rng_replays_sequence() {
        let mut first = DeterministicRng::new(42);
        let mut second = DeterministicRng::new(42);

        for _ in 0..16 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }
}
