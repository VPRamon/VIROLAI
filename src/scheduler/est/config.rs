/// Maximum allowed value for `k_beams`.
pub const MAX_K_BEAMS: usize = 100;

/// EST configuration — plain data, `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstConfig {
    /// Number of schedule states kept alive after each beam expansion round.
    pub k_beams: usize,
    /// Number of distinct candidates tried per beam per round.
    ///
    /// When `branching_factor == 1` and `k_beams == 1` the search degenerates
    /// to the classic greedy EST, matching the original single-beam behaviour
    /// exactly.
    pub branching_factor: usize,
}

impl Default for EstConfig {
    /// Return the classic greedy EST configuration.
    ///
    /// `k_beams = 1` and `branching_factor = 1` disable beam branching and keep
    /// only a single live state at each round.
    fn default() -> Self {
        Self {
            k_beams: 1,
            branching_factor: 1,
        }
    }
}
