//! Experiment specification for the matrix runner.
//!
//! An [`ExperimentSpec`] is a JSON document that declares a matrix of
//! `(dataset × algorithm × per-algorithm sweep)` cells. The runner takes the
//! Cartesian product and produces one [`crate::experiment::cell::MatrixCell`] per
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
//!   "max_parallel": 4,
//!   "output_dir": "out/paper"
//! }
//! ```

use schedulers::metrics::RankingWeights;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::experiment::config::{
    EstSweepAxes, HapSweepAxes, HorizonOverride, MultiCursorSweepAxes,
};

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
    /// Legacy ranking weights.
    ///
    /// This field is kept only so older experiment specs still deserialize.
    /// Sweep execution records objective metrics; ranking is a query-time
    /// concern handled by registry/query commands.
    #[serde(default)]
    pub ranking: Option<RankingWeightsSpec>,
    /// Maximum number of cells to execute concurrently.
    /// Defaults to the number of logical CPU cores when absent.
    #[serde(default)]
    pub max_parallel: Option<usize>,
    /// Root directory for all output artifacts.
    ///
    /// This field is kept only so older experiment specs still deserialise
    /// cleanly.  The DB-only runner ignores this value; schedule JSON is
    /// stored in the SQLite registry instead.
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
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
    /// LST sweep block.
    ///
    /// LST (Latest Start Time) uses the same parameter axes as EST.
    Lst {
        /// Parameter axes to expand.
        #[serde(default)]
        axes: EstSweepAxes,
    },
    /// Multi-cursor sweep block (Plan A fixed-territory layouts).
    MultiCursor {
        /// Parameter axes to expand.
        #[serde(default)]
        axes: MultiCursorSweepAxes,
    },
}

impl AlgorithmSweep {
    /// Returns `"est"`, `"hap"`, `"lst"`, or `"multi_cursor"`.
    pub const fn algorithm(&self) -> &'static str {
        match self {
            Self::Est { .. } => "est",
            Self::Hap { .. } => "hap",
            Self::Lst { .. } => "lst",
            Self::MultiCursor { .. } => "multi_cursor",
        }
    }
}

/// Serialisable mirror of [`schedulers::metrics::RankingWeights`].
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
            "max_parallel": 4,
            "output_dir": "out/demo"
        }"#
    }

    #[test]
    fn deserialize_spec_with_lst() {
        let json = r#"{
            "name": "fast-comparison",
            "datasets": [{ "id": "isdc_n", "path": "data/isdc_n.json" }],
            "algorithms": [
                { "kind": "est", "axes": { "k_beams": [1, 4] } },
                { "kind": "lst", "axes": { "k_beams": [1, 4], "branching_factors": [1, 2] } }
            ],
            "output_dir": "out/fast-comparison"
        }"#;
        let spec: ExperimentSpec = serde_json::from_str(json).expect("parse");
        assert_eq!(spec.algorithms.len(), 2);
        assert_eq!(spec.algorithms[0].algorithm(), "est");
        assert_eq!(spec.algorithms[1].algorithm(), "lst");
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
        assert!(spec.ranking.is_none());
        assert_eq!(spec.max_parallel, Some(4));
    }

    #[test]
    fn legacy_ranking_field_is_still_accepted() {
        let json = r#"{
            "name": "demo",
            "datasets": [{ "id": "ctao_n", "path": "data/ctao_n.json" }],
            "algorithms": [{ "kind": "est", "axes": { "k_beams": [1] } }],
            "ranking": { "completion": 2.0, "priority": 1.0 },
            "output_dir": "out/demo"
        }"#;
        let spec: ExperimentSpec = serde_json::from_str(json).expect("parse");
        let weights: RankingWeights = spec.ranking.unwrap().into();
        assert_eq!(weights.scheduled_task, 2.0);
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
    fn spec_without_output_dir_is_accepted() {
        let json = r#"{
            "name": "no-dir",
            "datasets": [{ "id": "ds", "path": "data/ds.json" }],
            "algorithms": [{ "kind": "est", "axes": { "k_beams": [1] } }]
        }"#;
        let spec: ExperimentSpec = serde_json::from_str(json).expect("parse without output_dir");
        assert!(spec.output_dir.is_none());
    }

    #[test]
    fn spec_with_output_dir_still_parses() {
        let json = r#"{
            "name": "with-dir",
            "datasets": [{ "id": "ds", "path": "data/ds.json" }],
            "algorithms": [{ "kind": "est", "axes": { "k_beams": [1] } }],
            "output_dir": "out/with-dir"
        }"#;
        let spec: ExperimentSpec = serde_json::from_str(json).expect("parse with output_dir");
        assert!(spec.output_dir.is_some());
    }

    #[test]
    fn ranking_old_field_names_accepted_as_aliases() {
        let json = r#"{ "completion": 2.0, "priority": 0.5 }"#;
        let r: RankingWeightsSpec = serde_json::from_str(json).expect("parse");
        assert_eq!(r.scheduled_task, 2.0);
        assert_eq!(r.scheduled_priority, 0.5);
    }
}
