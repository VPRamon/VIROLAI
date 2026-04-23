//! Deterministic RNG used by CRU stochastic tie-breakers.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

#[cfg(test)]
use rand::RngCore;

pub(super) struct CruRng(StdRng);

impl CruRng {
    pub(super) fn new(seed: u64) -> Self {
        Self(StdRng::seed_from_u64(seed))
    }

    #[cfg(test)]
    pub(super) fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    pub(super) fn next_usize(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        self.0.gen_range(0..n)
    }
}
