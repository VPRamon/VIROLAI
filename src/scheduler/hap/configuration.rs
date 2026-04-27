//! HAP scheduler configuration.

/// Runtime configuration for the HAP scheduler.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Configuration {
    /// Number of cheapest candidates offered to the stochastic picker when no
    /// zero-cost option is available. Larger values increase exploration at the
    /// cost of schedule quality.
    pub stochastic_range: usize,
    /// Maximum iterations of the inner Task Scheduling Cycle (ι_max in the
    /// CRU description). Caps the lobby-drain loop per block task.
    pub max_iter: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            stochastic_range: 3,
            max_iter: 100,
        }
    }
}
