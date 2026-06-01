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

use schedulers::scheduler::MultiCursorScheduler;
use schedulers::scheduler::cursor::{
    CursorConfig, CursorTerritory, MultiCursorConfig as CursorEngineConfig,
};
use schedulers::scheduler::est::{Configuration as EstConfiguration, EstScheduler, FomKind};
use schedulers::scheduler::fom::ScheduleFom;
use schedulers::scheduler::hap::{
    HapScheduler, PlannerConfig, SurvivorSelector as HapSurvivorSelector,
};
use schedulers::scheduler::lst::LstScheduler;
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

// ── Multi-cursor sweep axes ──────────────────────────────────────────────────

/// Multi-cursor parameter axes to sweep.
///
/// Shares the EST beam axes and adds a list of [`MultiCursorLayout`] values.
/// The runner takes the Cartesian product of all axes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MultiCursorSweepAxes {
    /// Cursor layouts to sweep. Empty falls back to `est_lst_split`.
    #[serde(default)]
    pub layouts: Vec<MultiCursorLayout>,
    /// Values of `endangered_threshold` (ε) to sweep.
    #[serde(default)]
    pub endangered_thresholds: Vec<u32>,
    /// Values of beam count (k) to sweep.
    #[serde(default)]
    pub k_beams: Vec<usize>,
    /// Values of branching factor (b) to sweep.
    #[serde(default)]
    pub branching_factors: Vec<usize>,
    /// FOM variants to sweep. Empty falls back to the default.
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
// [`crate::experiment::spec::ExperimentSpec`] exclusively.

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

// ── LST run configuration ────────────────────────────────────────────────────

/// Fully resolved, immutable configuration for a single LST scheduler run.
///
/// LST uses the same parameter axes as EST (figure of merit, endangered
/// threshold, beam count, branching factor), but schedules tasks as *late* as
/// possible by mirroring the horizon before running EST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LstRunConfig {
    /// Figure-of-merit variant used to rank candidate placements.
    pub fom: FomKind,
    /// Minimum number of competing tasks required to trigger beam splitting.
    pub endangered_threshold: u32,
    /// Number of parallel beams maintained during the search.
    pub k_beams: usize,
    /// Maximum branching factor applied at each decision point.
    pub branching_factor: usize,
}

impl Default for LstRunConfig {
    fn default() -> Self {
        Self {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

impl LstRunConfig {
    /// Returns the [`EstConfiguration`] struct used internally by the LST
    /// scheduler (which drives EST on a mirrored problem).
    pub const fn est_config(self) -> EstConfiguration {
        EstConfiguration {
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
            endangered_threshold: self.endangered_threshold,
        }
    }

    /// Instantiates an [`LstScheduler`] for this configuration.
    pub fn build_scheduler(self) -> Result<LstScheduler, String> {
        LstScheduler::with_fom(self.est_config(), self.fom.into_fom())
            .map_err(|e| format!("invalid LST configuration for {}: {e}", self.slug()))
    }

    /// Returns a short, filesystem-safe string that uniquely encodes all LST
    /// configuration axes.  Uses the same format as [`EstRunConfig::slug`]
    /// since the algorithm kind is already encoded in the cell ID.
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

// ── Multi-cursor run configuration ───────────────────────────────────────────

/// Predefined multi-cursor cursor layouts.
///
/// A *layout* fixes how many cursors exist and which territory/direction each
/// owns. Both layouts split the horizon at the midpoint. Single-cursor layouts
/// (plain EST / LST) are intentionally **not** included here — they remain the
/// dedicated [`RunConfig::Est`] / [`RunConfig::Lst`] variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiCursorLayout {
    /// Forward cursor over `[0, 0.5)` and a backward cursor over `[0.5, 1.0)`.
    EstLstSplit,
    /// Forward cursor over `[0, 0.5)` and a forward cursor over `[0.5, 1.0)`.
    StartMidForward,
}

impl MultiCursorLayout {
    /// Stable slug fragment encoding the layout.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EstLstSplit => "est_lst_split",
            Self::StartMidForward => "start_mid_forward",
        }
    }

    /// Build the two-cursor list (in horizon-relative fractions) for this layout.
    fn cursors(self) -> Vec<CursorConfig> {
        let front = CursorTerritory::FractionRange {
            start: 0.0,
            end: 0.5,
        };
        let back = CursorTerritory::FractionRange {
            start: 0.5,
            end: 1.0,
        };
        match self {
            Self::EstLstSplit => {
                vec![
                    CursorConfig::forward(0, front),
                    CursorConfig::backward(1, back),
                ]
            }
            Self::StartMidForward => {
                vec![
                    CursorConfig::forward(0, front),
                    CursorConfig::forward(1, back),
                ]
            }
        }
    }
}

impl std::fmt::Display for MultiCursorLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Fully resolved, immutable configuration for one multi-cursor scheduler run.
///
/// Shares the EST-style beam parameters (figure of merit, endangered threshold,
/// beam count, branching factor) and adds a [`MultiCursorLayout`] selecting the
/// cursor arrangement. Arbitrary cursor lists are intentionally not exposed
/// here so [`RunConfig`] stays `Copy` and sweep-friendly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MultiCursorRunConfig {
    /// Figure-of-merit variant used to rank candidate placements.
    pub fom: FomKind,
    /// Minimum number of competing tasks required to trigger beam splitting.
    pub endangered_threshold: u32,
    /// Number of parallel beams maintained during the search.
    pub k_beams: usize,
    /// Maximum branching factor applied at each decision point.
    pub branching_factor: usize,
    /// Cursor arrangement.
    pub layout: MultiCursorLayout,
}

impl Default for MultiCursorRunConfig {
    fn default() -> Self {
        Self {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
            layout: MultiCursorLayout::EstLstSplit,
        }
    }
}

impl MultiCursorRunConfig {
    /// Build the engine-level [`CursorEngineConfig`] for this run.
    pub fn cursor_config(self) -> CursorEngineConfig {
        CursorEngineConfig {
            cursors: self.layout.cursors(),
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
            endangered_threshold: self.endangered_threshold,
            cursor_policy: schedulers::scheduler::cursor::CursorPolicy::BestCandidateGlobal,
        }
    }

    /// Instantiates a [`MultiCursorScheduler`] for this configuration.
    pub fn build_scheduler(self) -> Result<MultiCursorScheduler, String> {
        MultiCursorScheduler::new(self.cursor_config(), self.fom.into_fom()).map_err(|e| {
            format!(
                "invalid multi-cursor configuration for {}: {e}",
                self.slug()
            )
        })
    }

    /// Returns a short, filesystem-safe string encoding the layout and beam
    /// axes (e.g. `"est_lst_split-e1-k4-b2"`).
    pub fn slug(self) -> String {
        let fom_suffix = if self.fom == FomKind::default() {
            String::new()
        } else {
            format!("-{}", self.fom.as_str())
        };
        format!(
            "{}-e{}-k{}-b{}{}",
            self.layout.as_str(),
            self.endangered_threshold,
            self.k_beams,
            self.branching_factor,
            fom_suffix
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

/// Fully resolved configuration for one scheduler run (EST, HAP, or LST).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case")]
pub enum RunConfig {
    /// An EST (Early Start Time beam-search) run.
    Est(EstRunConfig),
    /// A HAP (Heuristic Assignment Protocol) run.
    Hap(HapRunConfig),
    /// An LST (Latest Start Time) run.
    Lst(LstRunConfig),
    /// A multi-cursor run (Plan A: fixed-territory cursors).
    MultiCursor(MultiCursorRunConfig),
}

impl Default for RunConfig {
    fn default() -> Self {
        Self::Est(EstRunConfig::default())
    }
}

impl RunConfig {
    /// Returns `"est"`, `"hap"`, `"lst"`, or `"multi_cursor"`.
    pub const fn algorithm(self) -> &'static str {
        match self {
            Self::Est(_) => "est",
            Self::Hap(_) => "hap",
            Self::Lst(_) => "lst",
            Self::MultiCursor(_) => "multi_cursor",
        }
    }

    /// Returns the unique configuration slug (see [`EstRunConfig::slug`],
    /// [`HapRunConfig::slug`], [`LstRunConfig::slug`], and
    /// [`MultiCursorRunConfig::slug`]).
    pub fn slug(self) -> String {
        match self {
            Self::Est(config) => config.slug(),
            Self::Hap(config) => config.slug(),
            Self::Lst(config) => config.slug(),
            Self::MultiCursor(config) => config.slug(),
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
    fn lst_slug_matches_est_format() {
        let run = RunConfig::Lst(LstRunConfig {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 1,
            k_beams: 4,
            branching_factor: 2,
        });
        assert_eq!(run.slug(), "e1-k4-b2");
        assert_eq!(run.algorithm(), "lst");
    }

    #[test]
    fn lst_slug_includes_non_default_fom() {
        let run = RunConfig::Lst(LstRunConfig {
            fom: FomKind::FutureFlexibility,
            endangered_threshold: 1,
            k_beams: 4,
            branching_factor: 2,
        });
        assert_eq!(run.slug(), "e1-k4-b2-future_flexibility");
    }

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
    fn multi_cursor_slug_encodes_layout_and_axes() {
        let run = RunConfig::MultiCursor(MultiCursorRunConfig {
            fom: FomKind::SoftConstraint,
            endangered_threshold: 1,
            k_beams: 4,
            branching_factor: 2,
            layout: MultiCursorLayout::EstLstSplit,
        });
        assert_eq!(run.slug(), "est_lst_split-e1-k4-b2");
        assert_eq!(run.algorithm(), "multi_cursor");
    }

    #[test]
    fn multi_cursor_slug_distinguishes_layouts() {
        let split = MultiCursorRunConfig {
            layout: MultiCursorLayout::EstLstSplit,
            ..MultiCursorRunConfig::default()
        };
        let start_mid = MultiCursorRunConfig {
            layout: MultiCursorLayout::StartMidForward,
            ..MultiCursorRunConfig::default()
        };
        assert_ne!(split.slug(), start_mid.slug());
        assert_eq!(start_mid.slug(), "start_mid_forward-e1-k1-b1");
    }

    #[test]
    fn multi_cursor_build_scheduler_succeeds() {
        assert!(MultiCursorRunConfig::default().build_scheduler().is_ok());
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
