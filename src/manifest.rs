//! Lightweight, versioned manifest format that the CLI emits and the
//! webapp consumes.
//!
//! A manifest is the *primary lightweight exchange artifact* between the
//! scheduling CLI and the webapp workspace section: it captures the
//! producer, dataset, algorithm, run, and metrics of a single scheduler
//! result and *references* heavy artifacts (full schedule JSON, traces)
//! by URI + SHA-256 instead of embedding them.
//!
//! The on-disk schema lives at `schemas/scheduling_statistics/manifest.schema.json`. This module
//! provides the canonical Rust representation and a thin `validate`
//! helper. JSON Schema validation (cross-version compatibility checks,
//! field-presence checks against the schema document) is implemented as a
//! best-effort structural check here; the CLI binary may layer a richer
//! validator on top later without breaking this API.
//!
//! Versioning
//! ----------
//! `manifest_schema_version` follows SemVer:
//! - **major** bump — required-field removed/renamed; readers must reject.
//! - **minor** bump — additive only; readers ignore unknown fields.
//! - **patch** — documentation/clarification only.
//!
//! The current version is exposed as [`MANIFEST_SCHEMA_VERSION`].

use crate::metrics::ScheduleMetrics;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current manifest schema version. Bump per the rules in the module docs.
pub const MANIFEST_SCHEMA_VERSION: &str = "2.0.0";

/// Where a heavy artifact lives and how to verify it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// `file://`, `s3://`, `tsi://schedule/<id>`, etc.
    pub uri: String,
    pub size_bytes: u64,
    /// Hex-encoded SHA-256 of the bytes at `uri`.
    pub sha256: String,
    /// MIME type, e.g. `application/json`.
    pub media_type: String,
}

/// Information about the producing tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Producer {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// Reference to the input dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRef {
    pub id: String,
    pub name: String,
    pub source_path: String,
    pub sha256: String,
    pub schema_version: String,
}

/// Reference to the algorithm + its configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlgorithmRef {
    pub id: String,
    pub label: String,
    pub version: String,
    /// Opaque, algorithm-specific configuration captured for reproducibility.
    pub config: Value,
}

/// What kind of run produced this manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Single,
    MatrixCell,
}

/// Aggregate run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Failed,
    Skipped,
    Partial,
}

/// Run-level metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    pub run_id: String,
    pub kind: RunKind,
    /// RFC 3339 UTC.
    pub started_at: String,
    /// RFC 3339 UTC.
    pub finished_at: String,
    pub status: RunStatus,
    pub exit_code: i32,
}

/// Scheduling horizon expressed in MJD UTC, matching
/// [`crate::time::MJD`] semantics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Horizon {
    pub start_mjd_utc: f64,
    pub end_mjd_utc: f64,
}

/// Optional pointers to heavy artifacts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ArtifactRef>,
}

/// Webapp-side hints / cross-references.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Links {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tsi_schedule_id: Option<i64>,
}

/// Provenance information enabling reproduction of this run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cli_args: Vec<String>,
}

/// Outcome of [`Manifest::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Warning,
    Invalid,
}

/// A single validation issue surfaced by [`Manifest::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Stable machine-friendly identifier, e.g. `"empty_run_id"`.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// `error` | `warning`.
    pub severity: IssueSeverity,
}

/// Severity of a [`ValidationIssue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Error,
    Warning,
}

/// Validation report attached to a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub status: ValidationStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ValidationIssue>,
}

impl Default for ValidationReport {
    fn default() -> Self {
        Self {
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        }
    }
}

/// Top-level manifest payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_schema_version: String,
    pub manifest_id: String,
    pub created_at: String,
    pub producer: Producer,
    pub dataset: DatasetRef,
    pub algorithm: AlgorithmRef,
    pub run: RunInfo,
    pub horizon: Horizon,
    pub metrics: ScheduleMetrics,
    #[serde(default, skip_serializing_if = "is_default_artifacts")]
    pub artifacts: Artifacts,
    #[serde(default, skip_serializing_if = "is_default_links")]
    pub links: Links,
    pub provenance: Provenance,
    #[serde(default)]
    pub validation: ValidationReport,
    /// Reserved namespace for additive, non-breaking extensions. Readers
    /// must ignore unknown keys here.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extensions: Value,
}

fn is_default_artifacts(a: &Artifacts) -> bool {
    a == &Artifacts::default()
}

fn is_default_links(l: &Links) -> bool {
    l == &Links::default()
}

impl Manifest {
    /// Run a structural validation pass over the manifest. Populates and
    /// returns a fresh [`ValidationReport`] without mutating `self`.
    ///
    /// Checks performed:
    /// - Required string fields are non-empty.
    /// - Major version of `manifest_schema_version` matches
    ///   [`MANIFEST_SCHEMA_VERSION`].
    /// - `horizon.end_mjd_utc > horizon.start_mjd_utc`.
    /// - `metrics.scheduled_priority_stair.total_scheduled_items` matches
    ///   the sum of stair counts (consistency check).
    /// - Artifact SHA-256s, when present, look like 64-hex strings.
    pub fn validate(&self) -> ValidationReport {
        let mut issues: Vec<ValidationIssue> = Vec::new();

        macro_rules! err {
            ($code:expr, $msg:expr) => {{
                issues.push(ValidationIssue {
                    code: $code.to_string(),
                    message: $msg.to_string(),
                    severity: IssueSeverity::Error,
                });
            }};
        }
        macro_rules! warn_ {
            ($code:expr, $msg:expr) => {{
                issues.push(ValidationIssue {
                    code: $code.to_string(),
                    message: $msg.to_string(),
                    severity: IssueSeverity::Warning,
                });
            }};
        }

        if self.manifest_id.trim().is_empty() {
            err!("empty_manifest_id", "manifest_id must not be empty");
        }
        if self.created_at.trim().is_empty() {
            err!("empty_created_at", "created_at must not be empty");
        }
        if self.producer.name.trim().is_empty() {
            err!("empty_producer_name", "producer.name must not be empty");
        }
        if self.dataset.id.trim().is_empty() {
            err!("empty_dataset_id", "dataset.id must not be empty");
        }
        if self.algorithm.id.trim().is_empty() {
            err!("empty_algorithm_id", "algorithm.id must not be empty");
        }
        if self.run.run_id.trim().is_empty() {
            err!("empty_run_id", "run.run_id must not be empty");
        }

        match parse_major(&self.manifest_schema_version) {
            Some(major) if major == parse_major(MANIFEST_SCHEMA_VERSION).unwrap_or(1) => {}
            Some(_) => err!(
                "incompatible_schema_version",
                format!(
                    "manifest_schema_version {} is incompatible with reader {}",
                    self.manifest_schema_version, MANIFEST_SCHEMA_VERSION
                )
            ),
            None => err!(
                "invalid_schema_version",
                format!(
                    "manifest_schema_version '{}' is not valid SemVer",
                    self.manifest_schema_version
                )
            ),
        }

        if self.horizon.end_mjd_utc <= self.horizon.start_mjd_utc {
            err!(
                "invalid_horizon",
                "horizon.end_mjd_utc must be strictly greater than start_mjd_utc"
            );
        }

        let stair = &self.metrics.scheduled_priority_stair;
        let stair_sum: usize = stair.stairs.iter().map(|s| s.count).sum();
        if stair_sum != stair.total_scheduled_items {
            err!(
                "stair_count_mismatch",
                format!(
                    "stair counts sum to {stair_sum} but total_scheduled_items is {}",
                    stair.total_scheduled_items
                )
            );
        }

        for (name, art) in [
            ("schedule", self.artifacts.schedule.as_ref()),
            ("trace", self.artifacts.trace.as_ref()),
            ("problem", self.artifacts.problem.as_ref()),
        ] {
            if let Some(a) = art {
                if !is_hex_sha256(&a.sha256) {
                    warn_!(
                        "weak_artifact_sha256",
                        format!("artifacts.{name}.sha256 is not a 64-char hex string")
                    );
                }
                if a.uri.trim().is_empty() {
                    err!(
                        "empty_artifact_uri",
                        format!("artifacts.{name}.uri must not be empty")
                    );
                }
            }
        }

        let status = if issues.iter().any(|i| i.severity == IssueSeverity::Error) {
            ValidationStatus::Invalid
        } else if issues.is_empty() {
            ValidationStatus::Valid
        } else {
            ValidationStatus::Warning
        };

        ValidationReport { status, issues }
    }
}

/// Reserved subkey under `Manifest::extensions` carrying cohort-grouping
/// hints for the webapp `/workspace` UI.
///
/// All fields are optional and additive. Producers populate what they
/// know; readers tolerate missing fields by falling back to the manifest
/// `dataset` + `horizon`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observatory_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Horizon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_pool_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_count: Option<u64>,
}

impl Manifest {
    /// Read the optional `extensions.workspace_context` block. Returns
    /// `None` when the manifest has no extensions, no `workspace_context`
    /// subkey, or the subkey fails to deserialize.
    pub fn workspace_context(&self) -> Option<WorkspaceContext> {
        let ext = self.extensions.as_object()?;
        let value = ext.get("workspace_context")?;
        serde_json::from_value(value.clone()).ok()
    }
}

fn parse_major(version: &str) -> Option<u64> {
    version.split('.').next().and_then(|s| s.parse().ok())
}

fn is_hex_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::ScheduleMetrics;

    fn sample_metrics() -> ScheduleMetrics {
        // Build an empty-but-valid ScheduleMetrics via serde_json so the
        // test does not depend on the (large) compute() surface.
        serde_json::from_value(serde_json::json!({
            "scheduled_task_count": 0,
            "total_task_count": 0,
            "scheduled_task_ratio": 0.0,
            "scheduled_priority": {"count":0,"sum":0,"min":0,"max":0,"mean":0,"std":0,"p25":0,"p50":0,"p75":0,"p90":0},
            "scheduled_priority_sum": 0.0,
            "total_priority_sum": 0.0,
            "scheduled_priority_ratio": 0.0,
            "priority_density": 0.0,
            "fragmentation": {"gap_count":0,"gap_total_sec":0,"largest_gap_sec":0,"fragmentation_index":0},
            "total_horizon_sec": 86400.0,
            "available_time_sec": 86400.0,
            "scheduled_time_sec": 0.0,
            "utilization": 0.0,
            "per_resource": [],
            "composite_rank_score": 0.0,
            "ranking_weights": {"scheduled_task":1.0,"scheduled_priority":1.0,"utilization":1.0,"fragmentation":1.0}
        }))
        .unwrap()
    }

    fn sample_manifest() -> Manifest {
        Manifest {
            manifest_schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            manifest_id: "0192f6e4-0000-7000-8000-000000000000".to_string(),
            created_at: "2026-05-07T12:00:00Z".to_string(),
            producer: Producer {
                name: "phd".to_string(),
                version: "0.2.0".to_string(),
                git_sha: Some("abcd123".to_string()),
                host: None,
            },
            dataset: DatasetRef {
                id: "ctao-n".to_string(),
                name: "CTA-N".to_string(),
                source_path: "data/CTA-N/scheduling_problem.json".to_string(),
                sha256: "a".repeat(64),
                schema_version: "scheduling_problem/1".to_string(),
            },
            algorithm: AlgorithmRef {
                id: "est".to_string(),
                label: "EST".to_string(),
                version: "0.1.0".to_string(),
                config: serde_json::json!({"fom":"soft_constraint","e":1,"k":1,"b":1}),
            },
            run: RunInfo {
                run_id: "run-20260507T120000Z".to_string(),
                kind: RunKind::Single,
                started_at: "2026-05-07T11:58:00Z".to_string(),
                finished_at: "2026-05-07T12:00:00Z".to_string(),
                status: RunStatus::Completed,
                exit_code: 0,
            },
            horizon: Horizon {
                start_mjd_utc: 61710.0,
                end_mjd_utc: 62076.0,
            },
            metrics: sample_metrics(),
            artifacts: Artifacts::default(),
            links: Links::default(),
            provenance: Provenance::default(),
            validation: ValidationReport::default(),
            extensions: Value::Null,
        }
    }

    #[test]
    fn round_trips_through_json() {
        let m = sample_manifest();
        let text = serde_json::to_string_pretty(&m).unwrap();
        let back: Manifest = serde_json::from_str(&text).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn validate_accepts_well_formed_manifest() {
        let m = sample_manifest();
        let report = m.validate();
        assert_eq!(report.status, ValidationStatus::Valid);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn validate_rejects_empty_required_fields() {
        let mut m = sample_manifest();
        m.manifest_id = String::new();
        m.run.run_id = String::new();
        let report = m.validate();
        assert_eq!(report.status, ValidationStatus::Invalid);
        let codes: Vec<&str> = report.issues.iter().map(|i| i.code.as_str()).collect();
        assert!(codes.contains(&"empty_manifest_id"));
        assert!(codes.contains(&"empty_run_id"));
    }

    #[test]
    fn validate_rejects_incompatible_major_version() {
        let mut m = sample_manifest();
        m.manifest_schema_version = "3.0.0".to_string();
        let report = m.validate();
        assert_eq!(report.status, ValidationStatus::Invalid);
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "incompatible_schema_version")
        );
    }

    #[test]
    fn validate_rejects_invalid_horizon() {
        let mut m = sample_manifest();
        m.horizon.end_mjd_utc = m.horizon.start_mjd_utc;
        let report = m.validate();
        assert_eq!(report.status, ValidationStatus::Invalid);
        assert!(report.issues.iter().any(|i| i.code == "invalid_horizon"));
    }

    #[test]
    fn validate_warns_on_non_hex_artifact_sha() {
        let mut m = sample_manifest();
        m.artifacts.schedule = Some(ArtifactRef {
            uri: "file:///tmp/schedule.json".to_string(),
            size_bytes: 1024,
            sha256: "not-hex".to_string(),
            media_type: "application/json".to_string(),
        });
        let report = m.validate();
        assert_eq!(report.status, ValidationStatus::Warning);
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "weak_artifact_sha256")
        );
    }

    #[test]
    fn validate_detects_stair_count_mismatch() {
        let mut m = sample_manifest();
        // Inject an inconsistent stair: total_scheduled_items != sum of counts.
        m.metrics.scheduled_priority_stair.total_scheduled_items = 5;
        let report = m.validate();
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "stair_count_mismatch")
        );
    }

    #[test]
    fn forward_compat_unknown_extensions_field_round_trips() {
        let m = sample_manifest();
        let mut value = serde_json::to_value(&m).unwrap();
        value["extensions"] = serde_json::json!({"future_field": 42});
        let back: Manifest = serde_json::from_value(value).unwrap();
        assert_eq!(back.extensions["future_field"], 42);
    }
}
