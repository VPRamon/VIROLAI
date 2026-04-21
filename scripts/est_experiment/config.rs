use scheduler::scheduler::est::{EstConfig, EstFomKind, EstScheduler};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// An explicit observing window to substitute for the one detected in the input JSON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizonOverride {
    pub start_mjd: f64,
    pub end_mjd: f64,
}

/// Parameter axes to sweep; used both in the JSON spec and as CLI state.
///
/// Empty axes fall back to the EST defaults when building the run list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SweepAxes {
    #[serde(default)]
    pub foms: Vec<EstFomKind>,
    #[serde(default)]
    pub endangered_thresholds: Vec<u32>,
    #[serde(default)]
    pub k_beams: Vec<usize>,
    #[serde(default)]
    pub branching_factors: Vec<usize>,
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
    /// Parameter axes to sweep; defaults to the EST defaults when omitted.
    #[serde(default)]
    pub sweep: SweepAxes,
}

/// Fully resolved, immutable configuration for a single EST scheduler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RunConfig {
    pub fom: EstFomKind,
    pub endangered_threshold: u32,
    pub k_beams: usize,
    pub branching_factor: usize,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            fom: EstFomKind::TaskCount,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

impl RunConfig {
    pub const fn est_config(self) -> EstConfig {
        EstConfig {
            endangered_threshold: self.endangered_threshold,
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
        }
    }

    pub fn build_scheduler(self) -> Result<EstScheduler, String> {
        EstScheduler::with_kind(self.est_config(), self.fom)
            .map_err(|e| format!("invalid EST configuration for {}: {e}", self.slug()))
    }

    /// A filesystem-safe string that uniquely encodes all four configuration axes.
    pub fn slug(self) -> String {
        format!(
            "fom-{}__e-{}__k-{}__b-{}",
            self.fom, self.endangered_threshold, self.k_beams, self.branching_factor
        )
    }

    /// Compact filename stem for schedule outputs.
    pub fn schedule_file_stem(self) -> String {
        format!(
            "e{}-k{}-b{}-{}",
            self.endangered_threshold,
            self.k_beams,
            self.branching_factor,
            self.fom_label()
        )
    }

    const fn fom_label(self) -> &'static str {
        match self.fom {
            EstFomKind::TaskCount => "count",
            EstFomKind::SoftConstraint => "fitness",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_encodes_all_configuration_axes() {
        let run = RunConfig {
            fom: EstFomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        };
        assert_eq!(run.slug(), "fom-soft_constraint__e-2__k-5__b-3");
    }

    #[test]
    fn schedule_file_stem_uses_compact_pattern() {
        let count_run = RunConfig {
            fom: EstFomKind::TaskCount,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        };
        let fitness_run = RunConfig {
            fom: EstFomKind::SoftConstraint,
            ..count_run
        };
        assert_eq!(count_run.schedule_file_stem(), "e2-k5-b3-count");
        assert_eq!(fitness_run.schedule_file_stem(), "e2-k5-b3-fitness");
    }
}
