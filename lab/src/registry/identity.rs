//! Stable identity and hashing for scheduler runs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Metrics version tag included in the identity hash.
/// Increment (e.g. `"schedule_metrics/2"`) whenever the metric surface changes
/// in a way that makes old cached values incompatible.
pub const METRICS_VERSION: &str = "schedule_metrics/1";

/// Scheduler version string: `$GIT_SHA` (injected at build time via
/// `build.rs`) when present, otherwise `"lab/<version>"`.
pub fn scheduler_version() -> String {
    if let Some(sha) = option_env!("GIT_SHA")
        && !sha.is_empty()
    {
        return sha.to_string();
    }
    let lab_ver = env!("CARGO_PKG_VERSION");
    format!("lab/{lab_ver}")
}

/// The semantic identity of one scheduler run.
///
/// Two runs with the same `RunIdentity` are considered deterministically
/// equivalent; only one needs to be executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunIdentity {
    /// Dataset ID string from the experiment spec.
    pub dataset_id: String,
    /// Absolute or canonical dataset path for provenance only; not hashed.
    pub dataset_path: String,
    /// SHA-256 hex of the raw dataset file bytes.
    pub dataset_hash: String,
    /// Algorithm name (`"est"`, `"hap"`, or `"lst"`).
    pub algorithm: String,
    /// Configuration slug.
    pub config_slug: String,
    /// Full configuration as a compact JSON string.
    pub config_json: String,
    /// Horizon override serialized as compact JSON, or `null`.
    pub horizon_json: Option<String>,
    /// Scheduler/lab version string.
    pub scheduler_version: String,
    /// Metrics schema version.
    pub metrics_version: String,
}

/// The subset of fields that contribute to the `run_key` hash.
#[derive(Serialize)]
struct RunIdentityHashable<'a> {
    dataset_id: &'a str,
    dataset_hash: &'a str,
    algorithm: &'a str,
    config_slug: &'a str,
    config_json: &'a str,
    horizon_json: Option<&'a str>,
    scheduler_version: &'a str,
    metrics_version: &'a str,
}

impl RunIdentity {
    /// Computes the stable `run_key`: SHA-256 of the canonical compact JSON of
    /// the semantic identity fields. `dataset_path` is intentionally excluded.
    pub fn run_key(&self) -> String {
        let hashable = RunIdentityHashable {
            dataset_id: &self.dataset_id,
            dataset_hash: &self.dataset_hash,
            algorithm: &self.algorithm,
            config_slug: &self.config_slug,
            config_json: &self.config_json,
            horizon_json: self.horizon_json.as_deref(),
            scheduler_version: &self.scheduler_version,
            metrics_version: &self.metrics_version,
        };
        let canonical = serde_json::to_string(&hashable)
            .expect("RunIdentityHashable serialization is infallible");
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{digest:x}")
    }
}

/// Computes the SHA-256 hex digest of the file at `path`.
pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read dataset for hashing {}: {e}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}
