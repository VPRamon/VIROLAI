use crate::scheduler::est::{EstConfig, EstFomKind, EstScheduler};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// An explicit observing window to substitute for the one detected in the input JSON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizonOverride {
    pub start_mjd: f64,
    pub end_mjd: f64,
}

/// Optional field overrides for a single EST run, sourced from a spec file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstRunConfigOverride {
    #[serde(default)]
    pub fom: Option<EstFomKind>,
    #[serde(default)]
    pub endangered_threshold: Option<u32>,
    #[serde(default)]
    pub k_beams: Option<usize>,
    #[serde(default)]
    pub branching_factor: Option<usize>,
}

/// Sweep axes defined in an experiment spec file.
///
/// The Cartesian product of all non-empty axes is used to generate run configurations.
/// An empty axis falls back to the baseline value for that axis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstSweepSpec {
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
pub struct EstExperimentSpec {
    /// Path to the scheduling-problem JSON (absolute, or relative to the spec file).
    pub input_json: PathBuf,
    /// Directory where all output artifacts will be written.
    pub output_dir: PathBuf,
    #[serde(default)]
    pub horizon_override: Option<HorizonOverride>,
    /// Baseline run configuration; defaults to EST defaults when omitted.
    #[serde(default)]
    pub baseline: EstRunConfigOverride,
    /// Parameter axes to sweep.
    #[serde(default)]
    pub sweep: EstSweepSpec,
}

/// CLI flags that override any spec-file values.
#[derive(Debug, Clone, Default)]
pub struct EstExperimentCliOverrides {
    pub input_json: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub horizon_override: Option<HorizonOverride>,
    pub fom_values: Option<Vec<EstFomKind>>,
    pub endangered_threshold_values: Option<Vec<u32>>,
    pub k_beam_values: Option<Vec<usize>>,
    pub branching_factor_values: Option<Vec<usize>>,
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
            fom: EstFomKind::TaskCount,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

impl EstRunConfig {
    pub const fn est_config(self) -> EstConfig {
        EstConfig {
            endangered_threshold: self.endangered_threshold,
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
        }
    }

    /// Constructs and validates an [`EstScheduler`] for this configuration.
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
            self.schedule_fom_label()
        )
    }

    const fn schedule_fom_label(self) -> &'static str {
        match self.fom {
            EstFomKind::TaskCount => "count",
            EstFomKind::SoftConstraint => "fitness",
        }
    }

    pub(crate) fn apply_override(mut self, override_config: &EstRunConfigOverride) -> Self {
        if let Some(fom) = override_config.fom {
            self.fom = fom;
        }
        if let Some(endangered_threshold) = override_config.endangered_threshold {
            self.endangered_threshold = endangered_threshold;
        }
        if let Some(k_beams) = override_config.k_beams {
            self.k_beams = k_beams;
        }
        if let Some(branching_factor) = override_config.branching_factor {
            self.branching_factor = branching_factor;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_slug_encodes_all_configuration_axes() {
        let run = EstRunConfig {
            fom: EstFomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        };
        assert_eq!(run.slug(), "fom-soft_constraint__e-2__k-5__b-3");
    }

    #[test]
    fn schedule_file_stem_uses_compact_pattern() {
        let count_run = EstRunConfig {
            fom: EstFomKind::TaskCount,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        };
        let fitness_run = EstRunConfig {
            fom: EstFomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        };

        assert_eq!(count_run.schedule_file_stem(), "e2-k5-b3-count");
        assert_eq!(fitness_run.schedule_file_stem(), "e2-k5-b3-fitness");
    }
}
