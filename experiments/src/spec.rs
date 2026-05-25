//! Experiment specification for the matrix runner.
//!
//! An [`ExperimentSpec`] is a JSON document that declares a matrix of
//! `(dataset × algorithm × per-algorithm sweep)` cells. The runner takes the
//! Cartesian product and produces one [`crate::cell::MatrixCell`] per
//! combination.
//!
//! # Spec format
//!
//! ```json
//! {
//!   "name": "paper-sweep",
//!   "datasets": [
//!     { "id": "ctao_n", "path": "data/ctao_n.json" },
//!     { "id": "ctao_s", "path": "data/ctao_s.json",
//!       "horizon_override": { "start_mjd": 60000.0, "end_mjd": 60001.0 } }
//!   ],
//!   "algorithms": [
//!     { "kind": "est", "axes": { "k_beams": [1, 4], "branching_factors": [1, 2] } },
//!     { "kind": "hap", "axes": { "iota_max_values": [64, 128], "seeds": [0, 1] } }
//!   ],
//!   "ranking": { "completion": 2.0, "priority": 1.0 },
//!   "max_parallel": 4,
//!   "output_dir": "out/paper"
//! }
//! ```

use scheduler::metrics::RankingWeights;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::{EstSweepAxes, HapSweepAxes, HorizonOverride};

/// Top-level experiment specification, typically loaded from a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentSpec {
    /// Human-readable experiment name; used as a directory component under
    /// `output_dir` (slugified).
    pub name: String,
    /// Ordered list of input datasets to sweep over.
    pub datasets: Vec<DatasetEntry>,
    /// Per-algorithm sweep blocks.
    pub algorithms: Vec<AlgorithmSweep>,
    /// Optional composite-score ranking weights for metrics output.
    #[serde(default)]
    pub ranking: Option<RankingWeightsSpec>,
    /// Maximum number of cells to execute concurrently.
    /// Defaults to the number of logical CPU cores when absent.
    #[serde(default)]
    pub max_parallel: Option<usize>,
    /// Root directory for all output artifacts.
    pub output_dir: PathBuf,
}

/// A single input dataset entry in an experiment spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetEntry {
    /// Filesystem-safe slug used as the first component of `cell_id`.
    /// Must consist only of ASCII alphanumeric characters, `_`, or `-`.
    pub id: String,
    /// Path to the scheduling-problem JSON.
    /// May be absolute or relative to the spec file.
    pub path: PathBuf,
    /// Optional human-readable label embedded in schedule metadata.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional horizon override for this dataset.
    #[serde(default)]
    pub horizon_override: Option<HorizonOverride>,
}

/// Per-algorithm sweep block, tagged by `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlgorithmSweep {
    /// EST sweep block.
    Est {
        /// Parameter axes to expand.
        #[serde(default)]
        axes: EstSweepAxes,
    },
    /// HAP sweep block.
    Hap {
        /// Parameter axes to expand.
        #[serde(default)]
        axes: HapSweepAxes,
    },
}

impl AlgorithmSweep {
    /// Returns `"est"` or `"hap"`.
    pub const fn algorithm(&self) -> &'static str {
        match self {
            Self::Est { .. } => "est",
            Self::Hap { .. } => "hap",
        }
    }
}

/// Serialisable mirror of [`scheduler::metrics::RankingWeights`].
///
/// Redeclared here so the spec can use `#[serde(default)]` on each field and
/// remain forward-compatible when new ranking terms are added to the metrics
/// crate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankingWeightsSpec {
    /// Weight for the scheduled-task ratio term (default: 1.0).
    #[serde(default = "one", alias = "completion")]
    pub scheduled_task: f64,
    /// Weight for the scheduled-priority term (default: 1.0).
    #[serde(default = "one", alias = "priority")]
    pub scheduled_priority: f64,
    /// Weight for the time-utilisation term (default: 1.0).
    #[serde(default = "one")]
    pub utilization: f64,
    /// Weight for the fragmentation penalty term (default: 1.0).
    #[serde(default = "one")]
    pub fragmentation: f64,
}

fn one() -> f64 {
    1.0
}

impl From<RankingWeightsSpec> for RankingWeights {
    fn from(s: RankingWeightsSpec) -> Self {
        Self {
            scheduled_task: s.scheduled_task,
            scheduled_priority: s.scheduled_priority,
            utilization: s.utilization,
            fragmentation: s.fragmentation,
        }
    }
}

impl From<RankingWeights> for RankingWeightsSpec {
    fn from(w: RankingWeights) -> Self {
        Self {
            scheduled_task: w.scheduled_task,
            scheduled_priority: w.scheduled_priority,
            utilization: w.utilization,
            fragmentation: w.fragmentation,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> &'static str {
        r#"{
            "name": "demo",
            "datasets": [
                { "id": "ctao_n", "path": "data/ctao_n.json" },
                { "id": "ctao_s", "path": "data/ctao_s.json", "label": "South",
                  "horizon_override": { "start_mjd": 60000.0, "end_mjd": 60001.0 } }
            ],
            "algorithms": [
                { "kind": "est", "axes": { "k_beams": [1, 2] } },
                { "kind": "hap", "axes": { "iota_max_values": [64], "rho_values": [2] } }
            ],
            "ranking": { "completion": 2.0, "priority": 1.0, "utilization": 1.0, "fragmentation": 0.5 },
            "max_parallel": 4,
            "output_dir": "out/demo"
        }"#
    }

    #[test]
    fn deserialize_full_spec() {
        let spec: ExperimentSpec = serde_json::from_str(sample_json()).expect("parse");
        assert_eq!(spec.name, "demo");
        assert_eq!(spec.datasets.len(), 2);
        assert_eq!(spec.datasets[1].id, "ctao_s");
        assert_eq!(spec.algorithms.len(), 2);
        assert_eq!(spec.algorithms[0].algorithm(), "est");
        assert_eq!(spec.algorithms[1].algorithm(), "hap");
        let weights: RankingWeights = spec.ranking.unwrap().into();
        assert_eq!(weights.scheduled_task, 2.0);
        assert_eq!(spec.max_parallel, Some(4));
    }

    #[test]
    fn round_trip_spec() {
        let spec: ExperimentSpec = serde_json::from_str(sample_json()).expect("parse");
        let text = serde_json::to_string(&spec).expect("ser");
        let again: ExperimentSpec = serde_json::from_str(&text).expect("re-parse");
        assert_eq!(again.name, spec.name);
        assert_eq!(again.datasets.len(), spec.datasets.len());
        assert_eq!(again.algorithms.len(), spec.algorithms.len());
    }

    #[test]
    fn ranking_defaults_to_ones_when_field_missing() {
        let json = r#"{ "scheduled_task": 2.0, "scheduled_priority": 0.0 }"#;
        let r: RankingWeightsSpec = serde_json::from_str(json).expect("parse");
        assert_eq!(r.scheduled_task, 2.0);
        assert_eq!(r.scheduled_priority, 0.0);
        assert_eq!(r.utilization, 1.0);
        assert_eq!(r.fragmentation, 1.0);
    }

    #[test]
    fn ranking_old_field_names_accepted_as_aliases() {
        let json = r#"{ "completion": 2.0, "priority": 0.5 }"#;
        let r: RankingWeightsSpec = serde_json::from_str(json).expect("parse");
        assert_eq!(r.scheduled_task, 2.0);
        assert_eq!(r.scheduled_priority, 0.5);
    }
}
