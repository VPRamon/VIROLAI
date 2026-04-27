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

/// Survivor-selection strategy applied at the **planner level** (AP / HAP)
/// after CRU has produced its candidate set for a block.
///
/// This is independent of [`Selector`], which controls per-window choice
/// inside CRU. The planner-level selector decides which completed
/// candidate schedules survive into the next block iteration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurvivorSelector {
    /// Keep exactly one schedule: the highest-fitness candidate, with
    /// deterministic tie-breaking. Used by AP.
    GreedyOne,
    /// Keep the top `k` schedules by scalar fitness.
    ElitistTopK { k: usize },
    /// Keep the Pareto front over `(scheduling_rate, priority_sum)`. If
    /// the front exceeds `cap`, prune by crowding distance.
    ParetoFront { cap: usize },
}

/// Top-level configuration for the Accumulative Planner (AP / HAP).
///
/// AP and HAP share one core; they differ only in this configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlannerConfig {
    /// CRU configuration applied per source schedule (selector, ι_max).
    pub cru: Configuration,
    /// Number of CRU attempts per block (`ν`). AP uses 1; HAP uses ν > 1.
    /// Clamped to `max(1, …)` at runtime.
    pub population_size: usize,
    /// Planner-level survivor-selection policy.
    pub survivor: SurvivorSelector,
    /// When `true`, every source schedule used to seed CRU for the
    /// current block is also added to the candidate set, allowing the
    /// planner to **reject** the block when every CRU result reduces
    /// fitness. Recommended default.
    pub include_rejection_candidate: bool,
    /// Master RNG seed for HAP-style stochastic CRU. AP variants ignore
    /// this (the deterministic CRU selector never consults the RNG).
    pub seed: u64,
}

impl Default for PlannerConfig {
    /// AP-flavoured defaults: ν=1, deterministic CRU, single-best
    /// selection, rejection candidate enabled.
    fn default() -> Self {
        Self {
            cru: Configuration::default(),
            population_size: 1,
            survivor: SurvivorSelector::GreedyOne,
            include_rejection_candidate: true,
            seed: 0,
        }
    }
}

impl PlannerConfig {
    /// AP preset: deterministic CRU, ν=1, greedy single-best selection.
    pub fn ap(iota_max: usize) -> Self {
        Self {
            cru: Configuration {
                selector: Selector::Deterministic,
                stochastic_range: 3,
                max_iter: iota_max,
            },
            population_size: 1,
            survivor: SurvivorSelector::GreedyOne,
            include_rejection_candidate: true,
            seed: 0,
        }
    }

    /// HAP preset: CRU-S (`Stochastic { rho }`), `population_size` sources
    /// per block, configurable [`SurvivorSelector`].
    pub fn hap(
        iota_max: usize,
        rho: usize,
        population_size: usize,
        survivor: SurvivorSelector,
        seed: u64,
    ) -> Self {
        Self {
            cru: Configuration {
                selector: Selector::Stochastic { rho },
                stochastic_range: rho.max(1),
                max_iter: iota_max,
            },
            population_size: population_size.max(1),
            survivor,
            include_rejection_candidate: true,
            seed,
        }
    }
}
