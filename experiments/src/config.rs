//! Run-configuration types for the experiment runner.
//!
//! This module provides the fully-resolved, immutable configuration records
//! for a single scheduler run ([`EstRunConfig`], [`HapRunConfig`], [`RunConfig`])
//! as well as the sweep-axis descriptors ([`EstSweepAxes`], [`HapSweepAxes`])
//! used to expand a JSON experiment spec into a list of concrete runs.
//!
//! # Design
//! Every configuration type implements [`Copy`] and derives `Ord` so that a
//! [`BTreeSet`](std::collections::BTreeSet) naturally deduplicates and sorts the
//! expanded product.  [`RunConfig::slug`] produces a short, filesystem-safe
//! string that uniquely identifies each configuration — used as the stem of
//! schedule output files and as the last component of `cell_id` strings.

use scheduler::scheduler::est::{Configuration as EstConfiguration, EstScheduler, FomKind};
use scheduler::scheduler::fom::ScheduleFom;
use scheduler::scheduler::hap::{
    HapScheduler, PlannerConfig, SurvivorSelector as HapSurvivorSelector,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Horizon override ─────────────────────────────────────────────────────────

/// An explicit observing window (in MJD) that overrides the one detected in
/// the input JSON's `schedule_time_window`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizonOverride {
    /// Start of the observing window (Modified Julian Date, UTC).
    pub start_mjd: f64,
    /// End of the observing window (Modified Julian Date, UTC).
    pub end_mjd: f64,
}

// ── EST sweep axes ───────────────────────────────────────────────────────────

/// EST parameter axes to sweep.
///
/// Every field is a list of values; the runner takes the Cartesian product.
/// Empty axes fall back to the single-element default for that parameter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstSweepAxes {
    /// Values of `endangered_threshold` (ε) to sweep.
    #[serde(default)]
    pub endangered_thresholds: Vec<u32>,
    /// Values of beam count (k) to sweep.
    #[serde(default)]
    pub k_beams: Vec<usize>,
    /// Values of branching factor (b) to sweep.
    #[serde(default)]
    pub branching_factors: Vec<usize>,
    /// FOM variants to sweep.
    ///
    /// Empty falls back to the default (`soft_constraint`). The FOM is
    /// included in the cell slug only when it is not the default.
    #[serde(default)]
    pub foms: Vec<FomKind>,
}

// ── HAP sweep axes ───────────────────────────────────────────────────────────

/// HAP survivor-selection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HapSurvivorMode {
    /// Keep the single best individual.
    GreedyOne,
    /// Elitist top-k: keep the *k* best individuals by composite rank.
    ElitistTopK,
    /// Pareto front: keep the non-dominated front up to `cap` individuals.
    ParetoFront,
}

impl HapSurvivorMode {
    /// Returns the canonical snake-case string representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GreedyOne => "greedy_one",
            Self::ElitistTopK => "elitist_top_k",
            Self::ParetoFront => "pareto_front",
        }
    }

    /// Converts this mode (with a capacity) into the scheduler's
    /// [`HapSurvivorSelector`].
    pub fn into_selector(self, cap: usize) -> HapSurvivorSelector {
        match self {
            Self::GreedyOne => HapSurvivorSelector::GreedyOne,
            Self::ElitistTopK => HapSurvivorSelector::ElitistTopK { k: cap.max(1) },
            Self::ParetoFront => HapSurvivorSelector::ParetoFront { cap: cap.max(1) },
        }
    }
}

impl std::fmt::Display for HapSurvivorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// HAP parameter axes to sweep.
///
/// Every field is a list of values; the runner takes the Cartesian product.
/// Empty axes fall back to the single-element default for that parameter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HapSweepAxes {
    /// CRU task-scheduling iteration cap (ι_max).
    #[serde(default)]
    pub iota_max_values: Vec<usize>,
    /// CRU-S stochastic candidate range (ρ).
    #[serde(default)]
    pub rho_values: Vec<usize>,
    /// HAP multi-start population size per block.
    #[serde(default)]
    pub population_sizes: Vec<usize>,
    /// Survivor-selection strategies to test.
    #[serde(default)]
    pub survivor_modes: Vec<HapSurvivorMode>,
    /// Capacity caps for the selected survivor mode.
    #[serde(default)]
    pub survivor_caps: Vec<usize>,
    /// Deterministic master RNG seeds.
    #[serde(default)]
    pub seeds: Vec<u64>,
}

// ── Algorithm sweep block ────────────────────────────────────────────────────

/// Algorithm sweep block from an experiment specification.
///
/// New specs should use:
///
/// ```json
/// { "sweep": { "est": { ... }, "hap": { ... } } }
/// ```
///
/// The legacy EST shape remains accepted:
///
/// ```json
/// { "sweep": { "endangered_thresholds": [1], "k_beams": [1] } }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExperimentSweep {
    /// Flattened legacy EST axes (for backward compatibility).
    #[serde(flatten)]
    pub legacy_est: EstSweepAxes,
    /// New-style per-algorithm EST sweep block.
    #[serde(default)]
    pub est: Option<EstSweepAxes>,
    /// New-style per-algorithm HAP sweep block.
    #[serde(default)]
    pub hap: Option<HapSweepAxes>,
}

// ── Top-level experiment spec ────────────────────────────────────────────────
//
// The legacy single-dataset `ExperimentSpec` (used only by the removed
// `est_experiment` binary) has been deleted. The matrix runner uses
// [`crate::spec::ExperimentSpec`] exclusively.

// ── EST run configuration ────────────────────────────────────────────────────

/// Fully resolved, immutable configuration for a single EST scheduler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EstRunConfig {
    /// Figure-of-merit variant used to rank candidate placements.
    pub fom: FomKind,
    /// Minimum number of competing tasks required to trigger beam splitting.
    pub endangered_threshold: u32,
    /// Number of parallel beams maintained during the search.
    pub k_beams: usize,
    /// Maximum branching factor applied at each decision point.
    pub branching_factor: usize,
}

impl Default for EstRunConfig {
    fn default() -> Self {
        Self {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

impl EstRunConfig {
    /// Returns the [`EstConfiguration`] struct expected by the scheduler.
    pub const fn est_config(self) -> EstConfiguration {
        EstConfiguration {
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
            endangered_threshold: self.endangered_threshold,
        }
    }

    /// Instantiates an [`EstScheduler`] for this configuration.
    pub fn build_scheduler(self) -> Result<EstScheduler<Arc<dyn ScheduleFom>>, String> {
        EstScheduler::from_parts(self.est_config(), self.fom.into_fom())
            .map_err(|e| format!("invalid EST configuration for {}: {e}", self.slug()))
    }

    /// Returns a short, filesystem-safe string that uniquely encodes all EST
    /// configuration axes (e.g. `"e2-k5-b3"` or `"e2-k5-b3-future_flexibility"`).
    ///
    /// The FOM suffix is omitted when it is the default (`soft_constraint`) so
    /// existing run-directory names remain stable.
    pub fn slug(self) -> String {
        let fom_suffix = if self.fom == FomKind::default() {
            String::new()
        } else {
            format!("-{}", self.fom.as_str())
        };
        format!(
            "e{}-k{}-b{}{}",
            self.endangered_threshold, self.k_beams, self.branching_factor, fom_suffix
        )
    }
}

// ── HAP run configuration ────────────────────────────────────────────────────

/// Fully resolved, immutable configuration for one HAP scheduler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HapRunConfig {
    /// CRU task-scheduling iteration cap (ι_max).
    pub iota_max: usize,
    /// CRU-S stochastic candidate range (ρ).
    pub rho: usize,
    /// HAP multi-start population size per block.
    pub population_size: usize,
    /// Survivor-selection strategy.
    pub survivor_mode: HapSurvivorMode,
    /// Capacity cap for the survivor mode.
    pub survivor_cap: usize,
    /// Deterministic master RNG seed.
    pub seed: u64,
}

impl Default for HapRunConfig {
    fn default() -> Self {
        Self {
            iota_max: 128,
            rho: 3,
            population_size: 4,
            survivor_mode: HapSurvivorMode::ElitistTopK,
            survivor_cap: 4,
            seed: 0,
        }
    }
}

impl HapRunConfig {
    /// Converts this configuration into the [`PlannerConfig`] expected by the
    /// HAP scheduler.
    pub fn planner_config(self) -> PlannerConfig {
        PlannerConfig::hap(
            self.iota_max,
            self.rho,
            self.population_size,
            self.survivor_mode.into_selector(self.survivor_cap),
            self.seed,
        )
    }

    /// Instantiates a [`HapScheduler`] for this configuration.
    ///
    /// Returns an error if any parameter is zero (e.g. `iota_max`, `rho`,
    /// `population_size`, or `survivor_cap`).
    pub fn build_scheduler(self) -> Result<HapScheduler, String> {
        if self.iota_max == 0 {
            return Err(format!(
                "invalid HAP configuration for {}: iota_max must be at least 1",
                self.slug()
            ));
        }
        if self.rho == 0 {
            return Err(format!(
                "invalid HAP configuration for {}: rho must be at least 1",
                self.slug()
            ));
        }
        if self.population_size == 0 {
            return Err(format!(
                "invalid HAP configuration for {}: population_size must be at least 1",
                self.slug()
            ));
        }
        if self.survivor_cap == 0 {
            return Err(format!(
                "invalid HAP configuration for {}: survivor_cap must be at least 1",
                self.slug()
            ));
        }
        Ok(HapScheduler::new(self.planner_config()))
    }

    /// Returns a short, filesystem-safe string that uniquely encodes all HAP
    /// configuration axes (e.g. `"hap-i64-r2-p8-pareto5-s42"`).
    pub fn slug(self) -> String {
        let survivor = match self.survivor_mode {
            HapSurvivorMode::GreedyOne => "greedy1".to_string(),
            HapSurvivorMode::ElitistTopK => format!("elitist{}", self.survivor_cap),
            HapSurvivorMode::ParetoFront => format!("pareto{}", self.survivor_cap),
        };
        format!(
            "hap-i{}-r{}-p{}-{}-s{}",
            self.iota_max, self.rho, self.population_size, survivor, self.seed
        )
    }
}

// ── Unified run configuration ────────────────────────────────────────────────

/// Fully resolved configuration for one scheduler run (EST or HAP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case")]
pub enum RunConfig {
    /// An EST (Early Start Time beam-search) run.
    Est(EstRunConfig),
    /// A HAP (Heuristic Assignment Protocol) run.
    Hap(HapRunConfig),
}

impl Default for RunConfig {
    fn default() -> Self {
        Self::Est(EstRunConfig::default())
    }
}

impl RunConfig {
    /// Returns `"est"` or `"hap"`.
    pub const fn algorithm(self) -> &'static str {
        match self {
            Self::Est(_) => "est",
            Self::Hap(_) => "hap",
        }
    }

    /// Returns the unique configuration slug (see [`EstRunConfig::slug`] and
    /// [`HapRunConfig::slug`]).
    pub fn slug(self) -> String {
        match self {
            Self::Est(config) => config.slug(),
            Self::Hap(config) => config.slug(),
        }
    }

    /// Returns the compact filename stem used for schedule output files.
    pub fn schedule_file_stem(self) -> String {
        self.slug()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn est_slug_encodes_all_configuration_axes() {
        let run = RunConfig::Est(EstRunConfig {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        });
        assert_eq!(run.slug(), "e2-k5-b3");
    }

    #[test]
    fn hap_slug_encodes_all_configuration_axes() {
        let run = RunConfig::Hap(HapRunConfig {
            iota_max: 64,
            rho: 2,
            population_size: 8,
            survivor_mode: HapSurvivorMode::ParetoFront,
            survivor_cap: 5,
            seed: 42,
        });
        assert_eq!(run.slug(), "hap-i64-r2-p8-pareto5-s42");
    }

    #[test]
    fn schedule_file_stem_uses_slug() {
        let run = RunConfig::Est(EstRunConfig {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        });
        assert_eq!(run.schedule_file_stem(), "e2-k5-b3");
    }
}
