use scheduler::scheduler::est::{Configuration as EstConfiguration, EstFomKind, EstScheduler};
use scheduler::scheduler::hap::{
    HapScheduler, PlannerConfig, SurvivorSelector as HapSurvivorSelector,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// An explicit observing window to substitute for the one detected in the input JSON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizonOverride {
    pub start_mjd: f64,
    pub end_mjd: f64,
}

/// EST parameter axes to sweep; used by JSON specs and EST CLI flags.
///
/// Empty axes fall back to EST defaults when building the run list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstSweepAxes {
    #[serde(default)]
    pub endangered_thresholds: Vec<u32>,
    #[serde(default)]
    pub k_beams: Vec<usize>,
    #[serde(default)]
    pub branching_factors: Vec<usize>,
}

/// HAP survivor-selection mode used by experiment specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HapSurvivorMode {
    GreedyOne,
    ElitistTopK,
    ParetoFront,
}

impl HapSurvivorMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GreedyOne => "greedy_one",
            Self::ElitistTopK => "elitist_top_k",
            Self::ParetoFront => "pareto_front",
        }
    }

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
/// Empty axes fall back to [`HapRunConfig`] defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HapSweepAxes {
    #[serde(default)]
    pub iota_max_values: Vec<usize>,
    #[serde(default)]
    pub rho_values: Vec<usize>,
    #[serde(default)]
    pub population_sizes: Vec<usize>,
    #[serde(default)]
    pub survivor_modes: Vec<HapSurvivorMode>,
    #[serde(default)]
    pub survivor_caps: Vec<usize>,
    #[serde(default)]
    pub seeds: Vec<u64>,
}

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
    #[serde(flatten)]
    pub legacy_est: EstSweepAxes,
    #[serde(default)]
    pub est: Option<EstSweepAxes>,
    #[serde(default)]
    pub hap: Option<HapSweepAxes>,
}

/// Top-level experiment specification, typically loaded from a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentSpec {
    /// Path to the scheduling-problem JSON (absolute, or relative to the spec file).
    pub input_json: PathBuf,
    /// Directory where all output artifacts will be written.
    pub output_dir: PathBuf,
    #[serde(default)]
    pub horizon_override: Option<HorizonOverride>,
    /// Parameter axes to sweep; defaults to one EST run when omitted.
    #[serde(default)]
    pub sweep: ExperimentSweep,
    /// When true (default), each EST run also writes
    /// `<schedule_stem>.est_trace.jsonl` next to the schedule JSON. HAP runs do
    /// not currently emit trace files.
    #[serde(default = "default_emit_trace")]
    pub emit_trace: bool,
}

fn default_emit_trace() -> bool {
    true
}

/// Fully resolved, immutable configuration for a single EST scheduler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EstRunConfig {
    pub fom: EstFomKind,
    pub endangered_threshold: u32,
    pub k_beams: usize,
    pub branching_factor: usize,
}

impl Default for EstRunConfig {
    fn default() -> Self {
        Self {
            fom: EstFomKind::SoftConstraint,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

impl EstRunConfig {
    pub const fn est_config(self) -> EstConfiguration {
        EstConfiguration {
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
            endangered_threshold: self.endangered_threshold,
        }
    }

    pub fn build_scheduler(self) -> Result<EstScheduler, String> {
        EstScheduler::with_fom(self.est_config(), self.fom.into_fom())
            .map_err(|e| format!("invalid EST configuration for {}: {e}", self.slug()))
    }

    /// A filesystem-safe string that uniquely encodes all EST configuration axes.
    pub fn slug(self) -> String {
        format!(
            "e{}-k{}-b{}",
            self.endangered_threshold, self.k_beams, self.branching_factor
        )
    }
}

/// Fully resolved, immutable configuration for one HAP scheduler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HapRunConfig {
    pub iota_max: usize,
    pub rho: usize,
    pub population_size: usize,
    pub survivor_mode: HapSurvivorMode,
    pub survivor_cap: usize,
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
    pub fn planner_config(self) -> PlannerConfig {
        PlannerConfig::hap(
            self.iota_max,
            self.rho,
            self.population_size,
            self.survivor_mode.into_selector(self.survivor_cap),
            self.seed,
        )
    }

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

    /// A filesystem-safe string that uniquely encodes all HAP configuration axes.
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

/// Fully resolved configuration for one scheduler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case")]
pub enum RunConfig {
    Est(EstRunConfig),
    Hap(HapRunConfig),
}

impl Default for RunConfig {
    fn default() -> Self {
        Self::Est(EstRunConfig::default())
    }
}

impl RunConfig {
    pub const fn algorithm(self) -> &'static str {
        match self {
            Self::Est(_) => "est",
            Self::Hap(_) => "hap",
        }
    }

    pub fn slug(self) -> String {
        match self {
            Self::Est(config) => config.slug(),
            Self::Hap(config) => config.slug(),
        }
    }

    /// Compact filename stem for schedule outputs.
    pub fn schedule_file_stem(self) -> String {
        self.slug()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn est_slug_encodes_all_configuration_axes() {
        let run = RunConfig::Est(EstRunConfig {
            fom: EstFomKind::SoftConstraint,
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
            fom: EstFomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        });
        assert_eq!(run.schedule_file_stem(), "e2-k5-b3");
    }
}
