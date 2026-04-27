//! HAP scheduler configuration.

/// Strategy for picking a candidate window from a cost-sorted list of options
/// inside the CRU Task Scheduling Cycle.
///
/// All variants operate on the same cost-sorted candidate list and, when
/// zero-cost candidates exist, behave identically (any zero-cost option is
/// always preferred). They differ only in how they break the tie among
/// non-zero-cost candidates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selector {
    /// Base CRU: pick the lowest-cost candidate, tie-breaking
    /// deterministically. No RNG draw is performed.
    Deterministic,
    /// CRU-S: pick uniformly from the `rho` cheapest candidates.
    Stochastic { rho: usize },
    /// CRU-R: pick from *all* candidates with weight inversely proportional
    /// to cost (lower cost = higher probability).
    Random,
}

/// Runtime configuration for the HAP scheduler.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Configuration {
    /// Selector strategy applied by the inner Task Scheduling Cycle.
    pub selector: Selector,
    /// Legacy field: number of cheapest candidates offered to the stochastic
    /// picker. Used as a fallback when [`Selector::Stochastic`] is configured
    /// with `rho == 0`.
    pub stochastic_range: usize,
    /// Maximum iterations of the inner Task Scheduling Cycle (ι_max in the
    /// CRU description). Caps the lobby-drain loop per block task.
    pub max_iter: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            selector: Selector::Deterministic,
            stochastic_range: 3,
            max_iter: 100,
        }
    }
}
