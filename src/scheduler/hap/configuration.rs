/// Maximum allowed value for `num_crus`.
pub const MAX_NUM_CRUS: usize = 64;

/// HAP configuration — plain data, `Copy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Configuration {
    /// Number of parallel CRUs and survivor schedules kept between rounds.
    pub num_crus: usize,
    /// Maximum iterations per CRU repair run.
    pub cru_max_iterations: usize,
    /// Stochastic window selection: pick uniformly from the best N windows.
    pub stochastic_range: usize,
    /// Master seed for per-CRU RNG derivation.
    pub random_seed: u64,
    /// Impatience scaling factor (denominator).
    pub impatience_alpha: f64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            num_crus: 4,
            cru_max_iterations: 128,
            stochastic_range: 3,
            random_seed: 0,
            impatience_alpha: 1.0,
        }
    }
}
