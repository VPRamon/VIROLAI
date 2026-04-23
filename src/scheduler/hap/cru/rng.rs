//! Deterministic RNG used by CRU stochastic tie-breakers.

pub(super) struct Xorshift64(u64);

impl Xorshift64 {
    pub(super) fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    pub(super) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub(super) fn next_usize(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next() % n as u64) as usize
    }
}
