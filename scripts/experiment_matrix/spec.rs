//! Experiment-matrix specification types.
//!
//! An [`ExperimentSpec`] is a JSON document declaring a matrix of
//! `(dataset × algorithm × per-algorithm sweep)` cells. The runner takes the
//! cartesian product and produces one [`crate::cell::MatrixCell`] per
//! combination.

use scheduler::metrics::RankingWeights;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::est_experiment::config::{EstSweepAxes, HapSweepAxes, HorizonOverride};

/// Top-level experiment specification, typically loaded from a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentSpec {
    pub name: String,
    pub datasets: Vec<DatasetEntry>,
    pub algorithms: Vec<AlgorithmSweep>,
    #[serde(default)]
    pub ranking: Option<RankingWeightsSpec>,
    #[serde(default = "default_emit_trace")]
    pub emit_trace: bool,
    #[serde(default)]
    pub max_parallel: Option<usize>,
    pub output_dir: PathBuf,
}

fn default_emit_trace() -> bool {
    true
}

/// A single input dataset to sweep over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetEntry {
    /// Filesystem-safe slug, used as a component of `cell_id`.
    pub id: String,
    /// Path to the scheduling-problem JSON. May be absolute or relative to
    /// the spec file.
    pub path: PathBuf,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub horizon_override: Option<HorizonOverride>,
}

/// Per-algorithm sweep block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlgorithmSweep {
    Est {
        #[serde(default)]
        axes: EstSweepAxes,
    },
    Hap {
        #[serde(default)]
        axes: HapSweepAxes,
    },
}

impl AlgorithmSweep {
    pub const fn algorithm(&self) -> &'static str {
        match self {
            Self::Est { .. } => "est",
            Self::Hap { .. } => "hap",
        }
    }
}

/// Serializable mirror of [`scheduler::metrics::RankingWeights`].
///
/// Re-declared here so the spec can use `#[serde(default)]` on fields and
/// remain forward-compatible with new ranking terms added to the metrics
/// crate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankingWeightsSpec {
    #[serde(default = "one")]
    pub completion: f64,
    #[serde(default = "one")]
    pub priority: f64,
    #[serde(default = "one")]
    pub utilization: f64,
    #[serde(default = "one")]
    pub fragmentation: f64,
}

fn one() -> f64 {
    1.0
}

impl From<RankingWeightsSpec> for RankingWeights {
    fn from(s: RankingWeightsSpec) -> Self {
        Self {
            completion: s.completion,
            priority: s.priority,
            utilization: s.utilization,
            fragmentation: s.fragmentation,
        }
    }
}

impl From<RankingWeights> for RankingWeightsSpec {
    fn from(w: RankingWeights) -> Self {
        Self {
            completion: w.completion,
            priority: w.priority,
            utilization: w.utilization,
            fragmentation: w.fragmentation,
        }
    }
}

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
            "emit_trace": false,
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
        assert_eq!(weights.completion, 2.0);
        assert!(!spec.emit_trace);
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
        let json = r#"{
            "completion": 2.0,
            "priority": 0.0
        }"#;
        let r: RankingWeightsSpec = serde_json::from_str(json).expect("parse");
        assert_eq!(r.completion, 2.0);
        assert_eq!(r.priority, 0.0);
        assert_eq!(r.utilization, 1.0);
        assert_eq!(r.fragmentation, 1.0);
    }
}
