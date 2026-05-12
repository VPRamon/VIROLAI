//! Filesystem-backed workspace store.
//!
//! Layout under `<root>`:
//!
//! ```text
//! workspaces/
//!   index.json                           # top-level workspace registry
//!   <workspace_id>/
//!     workspace.json                     # workspace metadata
//!     manifests/
//!       <manifest_id>.json               # manifest payload (stored once)
//!     schedules/
//!       <sha256>.json                    # full schedule payload (deduped by content)
//!     index.json                         # ordered membership list + schedule
//!                                        # registry (per ws)
//! ```
//!
//! Schedules are persisted with content-addressed storage: identical
//! schedules across runs share the same on-disk file. The per-workspace
//! `index.json` keeps a registry of `ScheduleArtifact { sha256,
//! size_bytes, stored_at, manifest_ids }`; when the last manifest
//! referencing a schedule is removed with `delete_artifact=true` the
//! file is garbage-collected.
//!
//! All writes use a tmp-file + rename to keep the store atomic on crash.
//! Concurrent access from multiple handler tasks is serialised with a
//! single `std::sync::Mutex`; this is sufficient for the expected
//! single-process workspace deployment and avoids pulling in an async
//! lock for the relatively small surface here.

use chrono::{DateTime, Utc};
use scheduler::manifest::{ArtifactRef, Manifest, ValidationStatus, WorkspaceContext};
use scheduler::metrics::ScheduledPriorityStair;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::workspaces::errors::{WorkspaceError, WorkspaceResult};
use crate::workspaces::preschedule_cache::PrescheduleCache;

/// Maximum size of a `file://` schedule artifact accepted by drill-down.
const MAX_FILE_SCHEDULE_BYTES: u64 = 256 * 1024 * 1024;
const PRESCHEDULE_CACHE_DIR: &str = ".preschedule-cache";

/// Lifecycle state of a workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
    #[default]
    Active,
    Archived,
}

/// Persisted workspace metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub status: WorkspaceStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Number of manifests currently bound to this workspace.
    #[serde(default)]
    pub manifest_count: usize,
}

/// One entry in `<workspace_id>/index.json`. Heavy `Manifest` payloads
/// are stored separately so listing pages can stay small.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub manifest_id: String,
    pub display_name: String,
    pub algorithm_id: String,
    pub dataset_id: String,
    pub added_at: DateTime<Utc>,
    /// `idempotency_key` set by the publisher (defaults to `manifest_id`).
    pub idempotency_key: String,
    /// Stable cohort grouping key derived from
    /// `(dataset_id, observatory_id, period, block_pool_hash)`. Empty for
    /// older entries persisted before cohorts existed; readers should
    /// recompute from the manifest when blank.
    #[serde(default)]
    pub cohort_key: String,
}

/// Lightweight metric summary returned by the comparison endpoint. All
/// fields are pulled from the manifest's metric block — no schedule
/// loading is performed.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestSummary {
    pub manifest_id: String,
    pub display_name: String,
    pub algorithm_id: String,
    pub dataset_id: String,
    pub completion_ratio: f64,
    pub utilization: f64,
    pub composite_rank_score: f64,
    pub scheduled_task_count: usize,
    pub total_task_count: usize,
    pub priority_sum: f64,
    pub fragmentation_index: f64,
    pub stair_block_count: usize,
    pub validation_status: ValidationStatus,
    pub has_full_schedule: bool,
    pub tsi_schedule_id: Option<i64>,
    pub cohort_key: String,
    /// Algorithm-specific configuration captured for reproducibility.
    pub algorithm_config: Value,
    /// Run-length encoding of scheduled-task priorities.
    pub scheduled_priority_stair: ScheduledPriorityStair,
    /// Number of blocks that have been fully placed by the scheduler.
    /// `None` when neither the schedule nor the dataset are resolvable.
    pub completed_block_count: Option<u64>,
    /// Number of blocks the prescheduler considers schedulable.
    pub schedulable_block_count: Option<u64>,
    /// Total block count derived from the input scheduling problem.
    pub total_block_count: Option<u64>,
    /// `completed / total` when `total > 0`.
    pub block_completion_ratio: Option<f64>,
    /// `completed / schedulable` when `schedulable > 0`.
    pub schedulable_block_completion_ratio: Option<f64>,
}

/// Aggregate description of a cohort within a workspace. Cohorts group
/// manifests that share `(dataset_id, observatory_id, period, block_pool_hash)`
/// — i.e. comparable runs of different algorithms over the same input.
#[derive(Debug, Clone, Serialize)]
pub struct CohortSummary {
    pub cohort_key: String,
    pub dataset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observatory_id: Option<String>,
    pub period: scheduler::manifest::Horizon,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_pool_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_count: Option<u64>,
    pub manifest_count: usize,
    pub schedule_count: usize,
}

/// One row of the per-block breakdown table for a cohort. Only blocks
/// that appear in at least one persisted schedule are listed.
#[derive(Debug, Clone, Serialize)]
pub struct CohortBlockRow {
    pub block_id: String,
    pub priority: f64,
    pub duration_sec: f64,
    /// One placement per schedule that scheduled this block. Sorted by
    /// `(algorithm, manifest_id)` for stable column rendering.
    pub schedules: Vec<CohortBlockSchedulePlacement>,
}

/// Placement of a single block inside one schedule belonging to a cohort.
#[derive(Debug, Clone, Serialize)]
pub struct CohortBlockSchedulePlacement {
    pub manifest_id: String,
    pub algorithm: String,
    pub start_mjd_utc: f64,
}

/// In-memory cache + filesystem persistence for workspaces.
pub struct WorkspaceStore {
    root: PathBuf,
    /// `workspace_id` → record (mirrors `index.json`).
    workspaces: Mutex<HashMap<String, WorkspaceRecord>>,
    /// Persistent prescheduler cache shared across handlers.
    preschedule_cache: Arc<PrescheduleCache>,
}

const TOP_INDEX_FILE: &str = "index.json";
const WORKSPACE_FILE: &str = "workspace.json";
const WS_INDEX_FILE: &str = "index.json";
const MANIFESTS_DIR: &str = "manifests";
const SCHEDULES_DIR: &str = "schedules";
const SCHEDULE_URI_PREFIX: &str = "ws:///schedules/";

/// Registry record for a content-addressed schedule artifact stored
/// inside a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleArtifact {
    /// Hex-encoded SHA-256 of the canonical JSON bytes; doubles as the
    /// on-disk filename (`schedules/<sha256>.json`).
    pub sha256: String,
    pub size_bytes: u64,
    pub stored_at: DateTime<Utc>,
    /// Manifests in this workspace that point to this schedule via
    /// `artifacts.schedule`.
    #[serde(default)]
    pub manifest_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct TopIndex {
    workspaces: Vec<WorkspaceRecord>,
}

#[derive(Serialize, Deserialize, Default)]
struct WsIndex {
    entries: Vec<ManifestEntry>,
    /// Schedule artifacts owned by this workspace. Indexed by SHA-256.
    /// Older index files (pre-schedule-persistence) deserialise with an
    /// empty vec via `serde(default)`.
    #[serde(default)]
    schedules: Vec<ScheduleArtifact>,
}

impl WorkspaceStore {
    pub fn open(root: PathBuf) -> WorkspaceResult<Self> {
        fs::create_dir_all(&root)?;
        let path = root.join(TOP_INDEX_FILE);
        let map = if path.exists() {
            let bytes = fs::read(&path)?;
            let idx: TopIndex = serde_json::from_slice(&bytes).map_err(|e| {
                WorkspaceError::Internal(format!("corrupt {}: {e}", path.display()))
            })?;
            idx.workspaces
                .into_iter()
                .map(|w| (w.id.clone(), w))
                .collect()
        } else {
            HashMap::new()
        };
        Ok(Self {
            preschedule_cache: Arc::new(PrescheduleCache::new(root.join(PRESCHEDULE_CACHE_DIR))),
            root,
            workspaces: Mutex::new(map),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// URI prefix used to point manifests at workspace-stored schedules.
    /// The full URI for a schedule is `{prefix}{sha256}.json`.
    pub const fn schedule_uri_prefix() -> &'static str {
        SCHEDULE_URI_PREFIX
    }

    // ── workspace CRUD ────────────────────────────────────────────────

    pub fn list_workspaces(&self, include_archived: bool) -> Vec<WorkspaceRecord> {
        let g = self.workspaces.lock().expect("workspaces mutex poisoned");
        let mut out: Vec<WorkspaceRecord> = g
            .values()
            .filter(|w| include_archived || w.status == WorkspaceStatus::Active)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    pub fn create_workspace(
        &self,
        name: String,
        description: Option<String>,
    ) -> WorkspaceResult<WorkspaceRecord> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(WorkspaceError::BadRequest("name is required".into()));
        }
        let id = slugify(&name);
        let mut g = self.workspaces.lock().expect("workspaces mutex poisoned");
        if g.contains_key(&id) {
            return Err(WorkspaceError::Conflict(format!(
                "workspace `{id}` already exists"
            )));
        }
        let now = Utc::now();
        let rec = WorkspaceRecord {
            id: id.clone(),
            name,
            description,
            status: WorkspaceStatus::Active,
            created_at: now,
            updated_at: now,
            manifest_count: 0,
        };
        let dir = self.workspace_dir(&id);
        fs::create_dir_all(dir.join(MANIFESTS_DIR))?;
        write_atomic(&dir.join(WORKSPACE_FILE), &rec)?;
        write_atomic(&dir.join(WS_INDEX_FILE), &WsIndex::default())?;
        g.insert(id.clone(), rec.clone());
        self.persist_top_locked(&g)?;
        Ok(rec)
    }

    pub fn get_workspace(&self, id: &str) -> WorkspaceResult<WorkspaceRecord> {
        self.workspaces
            .lock()
            .expect("workspaces mutex poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| WorkspaceError::NotFound(format!("workspace `{id}`")))
    }

    pub fn update_workspace(
        &self,
        id: &str,
        new_name: Option<String>,
        new_description: Option<Option<String>>,
        new_status: Option<WorkspaceStatus>,
    ) -> WorkspaceResult<WorkspaceRecord> {
        let mut g = self.workspaces.lock().expect("workspaces mutex poisoned");
        let rec = g
            .get_mut(id)
            .ok_or_else(|| WorkspaceError::NotFound(format!("workspace `{id}`")))?;
        if let Some(n) = new_name {
            let n = n.trim().to_string();
            if n.is_empty() {
                return Err(WorkspaceError::BadRequest("name is required".into()));
            }
            rec.name = n;
        }
        if let Some(d) = new_description {
            rec.description = d;
        }
        if let Some(s) = new_status {
            rec.status = s;
        }
        rec.updated_at = Utc::now();
        let updated = rec.clone();
        write_atomic(&self.workspace_dir(id).join(WORKSPACE_FILE), &updated)?;
        self.persist_top_locked(&g)?;
        Ok(updated)
    }

    pub fn delete_workspace(&self, id: &str) -> WorkspaceResult<()> {
        let mut g = self.workspaces.lock().expect("workspaces mutex poisoned");
        if !g.contains_key(id) {
            return Err(WorkspaceError::NotFound(format!("workspace `{id}`")));
        }
        let dir = self.workspace_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        g.remove(id);
        self.persist_top_locked(&g)?;
        Ok(())
    }

    // ── manifest membership ───────────────────────────────────────────

    /// Add a manifest to a workspace. Returns the entry actually
    /// stored. If the same idempotency key is already present, returns
    /// the existing entry without rewriting the manifest payload.
    ///
    /// If the manifest references a workspace-stored schedule via
    /// `artifacts.schedule.uri = "ws:///schedules/<sha>.json"`, the
    /// schedule registry is updated so the schedule can be GC'd when
    /// its last referencing manifest is deleted.
    pub fn add_manifest(
        &self,
        workspace_id: &str,
        manifest: &Manifest,
        idempotency_key: Option<String>,
    ) -> WorkspaceResult<(ManifestEntry, /* created */ bool)> {
        let key = idempotency_key.unwrap_or_else(|| manifest.manifest_id.clone());
        let ws_dir = self.workspace_dir(workspace_id);

        // Hold the global lock for the entire read→modify→write cycle to
        // prevent concurrent requests from racing on the same index.json.
        let mut g = self.workspaces.lock().expect("workspaces mutex poisoned");
        if !g.contains_key(workspace_id) {
            return Err(WorkspaceError::NotFound(format!(
                "workspace `{workspace_id}`"
            )));
        }

        let mut idx: WsIndex = read_or_default(&ws_dir.join(WS_INDEX_FILE))?;
        if let Some(existing) = idx.entries.iter().find(|e| e.idempotency_key == key) {
            return Ok((existing.clone(), false));
        }
        // Persist the manifest body only once per `manifest_id`.
        let manifest_path = ws_dir
            .join(MANIFESTS_DIR)
            .join(format!("{}.json", manifest.manifest_id));
        if !manifest_path.exists() {
            write_atomic_value(&manifest_path, manifest)?;
        }
        let display_name = format!("{} · {}", manifest.algorithm.label, manifest.dataset.name);
        let entry = ManifestEntry {
            manifest_id: manifest.manifest_id.clone(),
            display_name,
            algorithm_id: manifest.algorithm.id.clone(),
            dataset_id: manifest.dataset.id.clone(),
            added_at: Utc::now(),
            idempotency_key: key,
            cohort_key: cohort_key(manifest),
        };
        idx.entries.push(entry.clone());
        // Cross-reference into the schedule registry, if applicable.
        if let Some(sha) = workspace_schedule_sha(manifest)
            && let Some(slot) = idx.schedules.iter_mut().find(|s| s.sha256 == sha)
            && !slot.manifest_ids.contains(&manifest.manifest_id)
        {
            slot.manifest_ids.push(manifest.manifest_id.clone());
        }
        write_atomic(&ws_dir.join(WS_INDEX_FILE), &idx)?;
        self.bump_count_locked(workspace_id, idx.entries.len(), &mut g)?;
        Ok((entry, true))
    }

    pub fn list_manifests(&self, workspace_id: &str) -> WorkspaceResult<Vec<ManifestEntry>> {
        self.ensure_workspace_exists(workspace_id)?;
        let idx: WsIndex = read_or_default(&self.workspace_dir(workspace_id).join(WS_INDEX_FILE))?;
        Ok(idx.entries)
    }

    pub fn get_manifest(&self, workspace_id: &str, manifest_id: &str) -> WorkspaceResult<Manifest> {
        self.ensure_workspace_exists(workspace_id)?;
        let path = self
            .workspace_dir(workspace_id)
            .join(MANIFESTS_DIR)
            .join(format!("{manifest_id}.json"));
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                WorkspaceError::NotFound(format!("manifest `{manifest_id}`"))
            } else {
                WorkspaceError::Io(e)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(|e| WorkspaceError::InvalidManifest(e.to_string()))
    }

    pub fn remove_manifest(
        &self,
        workspace_id: &str,
        manifest_id: &str,
        delete_artifact: bool,
    ) -> WorkspaceResult<()> {
        let ws_dir = self.workspace_dir(workspace_id);

        // Hold the global lock for the entire read→modify→write cycle.
        let mut g = self.workspaces.lock().expect("workspaces mutex poisoned");
        if !g.contains_key(workspace_id) {
            return Err(WorkspaceError::NotFound(format!(
                "workspace `{workspace_id}`"
            )));
        }

        let mut idx: WsIndex = read_or_default(&ws_dir.join(WS_INDEX_FILE))?;
        let before = idx.entries.len();
        idx.entries.retain(|e| e.manifest_id != manifest_id);
        if idx.entries.len() == before {
            return Err(WorkspaceError::NotFound(format!(
                "manifest `{manifest_id}` not in workspace"
            )));
        }
        // Drop this manifest from any schedule registry it referenced.
        let manifest_path = ws_dir
            .join(MANIFESTS_DIR)
            .join(format!("{manifest_id}.json"));
        let referenced_sha: Option<String> = if manifest_path.exists() {
            fs::read(&manifest_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
                .as_ref()
                .and_then(workspace_schedule_sha)
        } else {
            None
        };
        if let Some(sha) = referenced_sha.as_deref() {
            for slot in idx.schedules.iter_mut() {
                if slot.sha256 == sha {
                    slot.manifest_ids.retain(|m| m != manifest_id);
                }
            }
        }

        // GC: when delete_artifact is requested, drop both the manifest
        // body and any orphaned schedule files. Otherwise we keep the
        // bytes around (they may still be useful for re-import).
        if delete_artifact {
            if manifest_path.exists() {
                fs::remove_file(&manifest_path)?;
            }
            if let Some(sha) = referenced_sha.as_deref() {
                idx.schedules.retain(|s| {
                    if s.sha256 == sha && s.manifest_ids.is_empty() {
                        let path = ws_dir.join(SCHEDULES_DIR).join(format!("{sha}.json"));
                        let _ = fs::remove_file(path);
                        false
                    } else {
                        true
                    }
                });
            }
        }
        write_atomic(&ws_dir.join(WS_INDEX_FILE), &idx)?;
        self.bump_count_locked(workspace_id, idx.entries.len(), &mut g)?;
        Ok(())
    }

    /// Persist a schedule body to the workspace using content-addressed
    /// storage (filename = SHA-256 hex of canonical bytes). Idempotent:
    /// re-uploading identical bytes returns the existing artifact and
    /// does not rewrite the file.
    ///
    /// Returns `(sha256, size_bytes, stored_at)`.
    pub fn put_schedule(
        &self,
        workspace_id: &str,
        schedule: &Value,
    ) -> WorkspaceResult<(String, u64, DateTime<Utc>)> {
        let bytes = serde_json::to_vec(schedule)
            .map_err(|e| WorkspaceError::Internal(format!("serialize schedule: {e}")))?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha = format!("{:x}", hasher.finalize());
        let size = bytes.len() as u64;

        let ws_dir = self.workspace_dir(workspace_id);

        // Hold the global lock for the entire read→modify→write cycle.
        let guard = self.workspaces.lock().expect("workspaces mutex poisoned");
        if !guard.contains_key(workspace_id) {
            return Err(WorkspaceError::NotFound(format!(
                "workspace `{workspace_id}`"
            )));
        }

        let path = ws_dir.join(SCHEDULES_DIR).join(format!("{sha}.json"));
        let mut idx: WsIndex = read_or_default(&ws_dir.join(WS_INDEX_FILE))?;

        if let Some(existing) = idx.schedules.iter().find(|s| s.sha256 == sha) {
            return Ok((
                existing.sha256.clone(),
                existing.size_bytes,
                existing.stored_at,
            ));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;

        let stored_at = Utc::now();
        idx.schedules.push(ScheduleArtifact {
            sha256: sha.clone(),
            size_bytes: size,
            stored_at,
            manifest_ids: Vec::new(),
        });
        write_atomic(&ws_dir.join(WS_INDEX_FILE), &idx)?;
        Ok((sha, size, stored_at))
    }

    /// Load a workspace-stored schedule by SHA-256. Returns the parsed
    /// JSON `Value`; large bodies are not cached in memory.
    pub fn get_schedule(&self, workspace_id: &str, sha256: &str) -> WorkspaceResult<Value> {
        self.ensure_workspace_exists(workspace_id)?;
        let path = self
            .workspace_dir(workspace_id)
            .join(SCHEDULES_DIR)
            .join(format!("{sha256}.json"));
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                WorkspaceError::NotFound(format!("schedule `{sha256}`"))
            } else {
                WorkspaceError::Io(e)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(|e| WorkspaceError::Internal(e.to_string()))
    }

    /// Convenience: return the schedule referenced by a manifest's
    /// `artifacts.schedule.uri`. Resolves both workspace-stored URIs
    /// (`ws:///schedules/<sha>.json`) and `file://` artifacts written by
    /// `phd sweep`.
    pub fn get_schedule_for_manifest(
        &self,
        workspace_id: &str,
        manifest_id: &str,
    ) -> WorkspaceResult<Value> {
        let manifest = self.get_manifest(workspace_id, manifest_id)?;
        self.resolve_schedule_for_manifest(workspace_id, &manifest)
    }

    /// Resolve a schedule body for an already-loaded manifest. Tries the
    /// workspace-stored sha first, then falls back to a `file://`
    /// artifact on the local filesystem.
    pub fn resolve_schedule_for_manifest(
        &self,
        workspace_id: &str,
        manifest: &Manifest,
    ) -> WorkspaceResult<Value> {
        if let Some(sha) = workspace_schedule_sha(manifest) {
            let path = self
                .workspace_dir(workspace_id)
                .join(SCHEDULES_DIR)
                .join(format!("{sha}.json"));
            if path.exists() {
                return self.get_schedule(workspace_id, &sha);
            }
        }
        if let Some(path) = manifest_file_schedule_path(manifest) {
            let meta = fs::metadata(&path).map_err(|e| {
                if e.kind() == ErrorKind::NotFound {
                    WorkspaceError::NotFound(format!("schedule file `{}`", path.display()))
                } else {
                    WorkspaceError::Io(e)
                }
            })?;
            if meta.len() > MAX_FILE_SCHEDULE_BYTES {
                return Err(WorkspaceError::BadRequest(format!(
                    "schedule file `{}` exceeds 256 MiB cap",
                    path.display()
                )));
            }
            let bytes = fs::read(&path).map_err(WorkspaceError::Io)?;
            return serde_json::from_slice(&bytes)
                .map_err(|e| WorkspaceError::InvalidManifest(e.to_string()));
        }
        Err(WorkspaceError::NotFound(format!(
            "manifest `{}` has no resolvable schedule",
            manifest.manifest_id
        )))
    }

    /// Mirror of [`Self::resolve_schedule_for_manifest`] that only checks
    /// whether the schedule body would be readable (no IO of the body).
    pub fn manifest_has_resolvable_schedule(
        &self,
        workspace_id: &str,
        manifest: &Manifest,
    ) -> bool {
        if let Some(sha) = workspace_schedule_sha(manifest) {
            let path = self
                .workspace_dir(workspace_id)
                .join(SCHEDULES_DIR)
                .join(format!("{sha}.json"));
            if path.exists() {
                return true;
            }
        }
        if let Some(path) = manifest_file_schedule_path(manifest) {
            return path.exists();
        }
        false
    }

    /// List schedule artifacts registered in the workspace. Cheap: only
    /// reads the index file.
    #[allow(dead_code)]
    pub fn list_schedules(&self, workspace_id: &str) -> WorkspaceResult<Vec<ScheduleArtifact>> {
        self.ensure_workspace_exists(workspace_id)?;
        let idx: WsIndex = read_or_default(&self.workspace_dir(workspace_id).join(WS_INDEX_FILE))?;
        Ok(idx.schedules)
    }

    /// Build comparison summaries straight from manifest JSON. **Does
    /// not load the referenced full schedules.**
    pub fn comparison_summary(&self, workspace_id: &str) -> WorkspaceResult<Vec<ManifestSummary>> {
        let entries = self.list_manifests(workspace_id)?;
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            let m = self.get_manifest(workspace_id, &e.manifest_id)?;
            let has_full_schedule = self.manifest_has_resolvable_schedule(workspace_id, &m);
            let ck = if e.cohort_key.is_empty() {
                cohort_key(&m)
            } else {
                e.cohort_key.clone()
            };
            let block_ratios = match self.block_ratios_for_manifest(workspace_id, &m) {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!(
                        target: "phd.workspaces",
                        workspace = %workspace_id,
                        manifest = %m.manifest_id,
                        error = %err,
                        "block ratio computation failed",
                    );
                    None
                }
            };
            let (total_block_count, schedulable_block_count, completed_block_count) =
                match block_ratios {
                    Some((t, s, c)) => (Some(t), Some(s), Some(c)),
                    None => (None, None, None),
                };
            let block_completion_ratio = match (completed_block_count, total_block_count) {
                (Some(c), Some(t)) if t > 0 => Some(c as f64 / t as f64),
                _ => None,
            };
            let schedulable_block_completion_ratio =
                match (completed_block_count, schedulable_block_count) {
                    (Some(c), Some(s)) if s > 0 => Some(c as f64 / s as f64),
                    _ => None,
                };
            out.push(ManifestSummary {
                manifest_id: m.manifest_id.clone(),
                display_name: e.display_name,
                algorithm_id: m.algorithm.id.clone(),
                dataset_id: m.dataset.id.clone(),
                completion_ratio: m.metrics.completion_ratio,
                utilization: m.metrics.utilization,
                composite_rank_score: m.metrics.composite_rank_score,
                scheduled_task_count: m.metrics.scheduled_task_count,
                total_task_count: m.metrics.total_task_count,
                priority_sum: m.metrics.priority.sum,
                fragmentation_index: m.metrics.fragmentation.fragmentation_index,
                stair_block_count: m.metrics.scheduled_priority_stair.stairs.len(),
                validation_status: m.validation.status,
                has_full_schedule,
                tsi_schedule_id: m.links.tsi_schedule_id,
                cohort_key: ck,
                algorithm_config: m.algorithm.config.clone(),
                scheduled_priority_stair: m.metrics.scheduled_priority_stair.clone(),
                completed_block_count,
                schedulable_block_count,
                total_block_count,
                block_completion_ratio,
                schedulable_block_completion_ratio,
            });
        }
        Ok(out)
    }

    /// List every cohort represented in this workspace. Cheap: only
    /// loads manifests, never schedules.
    pub fn list_cohorts(&self, workspace_id: &str) -> WorkspaceResult<Vec<CohortSummary>> {
        let entries = self.list_manifests(workspace_id)?;
        let mut grouped: HashMap<String, CohortSummary> = HashMap::new();
        for e in entries {
            let m = self.get_manifest(workspace_id, &e.manifest_id)?;
            let key = if e.cohort_key.is_empty() {
                cohort_key(&m)
            } else {
                e.cohort_key.clone()
            };
            let ctx = m.workspace_context().unwrap_or_default();
            let period = ctx.period.unwrap_or(m.horizon);
            let block_pool_hash = ctx.block_pool_hash;
            let block_count = ctx.block_count;
            let observatory_id = ctx.observatory_id;
            let has_schedule = self.manifest_has_resolvable_schedule(workspace_id, &m);
            let summary = grouped.entry(key.clone()).or_insert_with(|| CohortSummary {
                cohort_key: key,
                dataset_id: m.dataset.id.clone(),
                observatory_id: observatory_id.clone(),
                period,
                block_pool_hash: block_pool_hash.clone(),
                block_count,
                manifest_count: 0,
                schedule_count: 0,
            });
            summary.manifest_count += 1;
            if has_schedule {
                summary.schedule_count += 1;
            }
        }
        let mut out: Vec<CohortSummary> = grouped.into_values().collect();
        out.sort_by(|a, b| {
            a.dataset_id
                .cmp(&b.dataset_id)
                .then(a.cohort_key.cmp(&b.cohort_key))
        });
        Ok(out)
    }

    /// Build the per-block breakdown table for a single cohort. Reads
    /// only the schedules that are persisted in the workspace and that
    /// belong to manifests in the cohort.
    pub fn cohort_blocks(
        &self,
        workspace_id: &str,
        cohort_key_q: &str,
    ) -> WorkspaceResult<Vec<CohortBlockRow>> {
        let entries = self.list_manifests(workspace_id)?;
        let mut rows: HashMap<String, CohortBlockRow> = HashMap::new();
        for e in entries {
            let m = self.get_manifest(workspace_id, &e.manifest_id)?;
            let key = if e.cohort_key.is_empty() {
                cohort_key(&m)
            } else {
                e.cohort_key.clone()
            };
            if key != cohort_key_q {
                continue;
            }
            if !self.manifest_has_resolvable_schedule(workspace_id, &m) {
                continue;
            }
            let schedule = match self.resolve_schedule_for_manifest(workspace_id, &m) {
                Ok(v) => v,
                Err(WorkspaceError::NotFound(_)) => continue,
                Err(other) => return Err(other),
            };
            harvest_block_rows(
                &schedule,
                &m.manifest_id,
                m.algorithm.id.as_str(),
                &mut rows,
            );
        }
        let mut out: Vec<CohortBlockRow> = rows.into_values().collect();
        out.sort_by(|a, b| a.block_id.cmp(&b.block_id));
        for row in out.iter_mut() {
            row.schedules.sort_by(|a, b| {
                a.algorithm
                    .cmp(&b.algorithm)
                    .then(a.manifest_id.cmp(&b.manifest_id))
            });
        }
        Ok(out)
    }

    // ── helpers ───────────────────────────────────────────────────────

    fn workspace_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn ensure_workspace_exists(&self, id: &str) -> WorkspaceResult<()> {
        if self
            .workspaces
            .lock()
            .expect("workspaces mutex poisoned")
            .contains_key(id)
        {
            Ok(())
        } else {
            Err(WorkspaceError::NotFound(format!("workspace `{id}`")))
        }
    }

    /// Like `bump_count` but called when the caller already holds the
    /// `workspaces` mutex guard. Avoids a re-lock (which would deadlock).
    fn bump_count_locked(
        &self,
        id: &str,
        count: usize,
        g: &mut HashMap<String, WorkspaceRecord>,
    ) -> WorkspaceResult<()> {
        if let Some(rec) = g.get_mut(id) {
            rec.manifest_count = count;
            rec.updated_at = Utc::now();
            write_atomic(&self.workspace_dir(id).join(WORKSPACE_FILE), rec)?;
            self.persist_top_locked(g)?;
        }
        Ok(())
    }

    fn persist_top_locked(&self, map: &HashMap<String, WorkspaceRecord>) -> WorkspaceResult<()> {
        let idx = TopIndex {
            workspaces: map.values().cloned().collect(),
        };
        write_atomic(&self.root.join(TOP_INDEX_FILE), &idx)
    }
}

// ── filesystem helpers ────────────────────────────────────────────────

fn write_atomic<T: Serialize>(path: &Path, value: &T) -> WorkspaceResult<()> {
    write_atomic_value(path, value)
}

fn write_atomic_value<T: Serialize>(path: &Path, value: &T) -> WorkspaceResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|e| WorkspaceError::Internal(format!("serialize: {e}")))?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn read_or_default<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> WorkspaceResult<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| WorkspaceError::Internal(format!("corrupt {}: {e}", path.display())))
}

/// Compute the canonical cohort key for a manifest. Stable across runs
/// and across processes.
///
/// Key inputs (in order): `dataset_id`, `observatory_id|""`,
/// `period.start_mjd_utc` (4-decimal-rounded), `period.end_mjd_utc`
/// (4-decimal-rounded), `block_pool_hash|""`. When the manifest has no
/// `extensions.workspace_context`, falls back to the manifest's
/// `dataset.id` + `horizon` + empty observatory + empty pool hash so all
/// pre-cohort manifests of the same dataset/horizon collapse into a
/// single deterministic fallback bucket.
pub fn cohort_key(manifest: &Manifest) -> String {
    let ctx = manifest.workspace_context().unwrap_or_default();
    let observatory = ctx.observatory_id.unwrap_or_default();
    let period = ctx.period.unwrap_or(manifest.horizon);
    let pool = ctx.block_pool_hash.unwrap_or_default();
    let key = serde_json::json!([
        manifest.dataset.id,
        observatory,
        round4(period.start_mjd_utc),
        round4(period.end_mjd_utc),
        pool,
    ]);
    let mut hasher = Sha256::new();
    hasher.update(key.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// Walk a self-contained schedule JSON and append per-block placement
/// rows into `rows`, keyed by block id. Tolerates missing fields by
/// skipping the affected block.
///
/// Walks `scheduling_blocks` first (current schedule shape) and falls
/// back to the legacy `blocks` array when present.
fn harvest_block_rows(
    schedule: &Value,
    manifest_id: &str,
    algorithm: &str,
    rows: &mut HashMap<String, CohortBlockRow>,
) {
    if let Some(blocks) = schedule.get("scheduling_blocks").and_then(|v| v.as_array()) {
        for block in blocks {
            harvest_scheduling_block(block, manifest_id, algorithm, rows);
        }
        return;
    }
    let Some(blocks) = schedule.get("blocks").and_then(|v| v.as_array()) else {
        return;
    };
    for block in blocks {
        harvest_legacy_block(block, manifest_id, algorithm, rows);
    }
}

fn harvest_scheduling_block(
    block: &Value,
    manifest_id: &str,
    algorithm: &str,
    rows: &mut HashMap<String, CohortBlockRow>,
) {
    let Some(id) = block.get("id").and_then(json_id_to_string) else {
        return;
    };
    let priority = block
        .get("priority")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // Sum per-task durations; ignore tasks lacking a numeric duration.
    let mut duration_sec = 0.0_f64;
    let mut min_start: Option<f64> = None;
    let mut any_scheduled = false;
    if let Some(tasks) = block.get("tasks").and_then(|v| v.as_array()) {
        for task in tasks {
            if let Some(d) = task
                .get("duration")
                .or_else(|| task.get("requested_duration_sec"))
                .or_else(|| task.get("duration_sec"))
                .and_then(|v| v.as_f64())
            {
                duration_sec += d;
            }
            let scheduled = task
                .get("scheduled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if scheduled {
                any_scheduled = true;
                if let Some(start) = task.get("scheduled_start_mjd_utc").and_then(|v| v.as_f64()) {
                    min_start = Some(min_start.map_or(start, |cur| cur.min(start)));
                }
            }
        }
    }

    let row = rows.entry(id.clone()).or_insert(CohortBlockRow {
        block_id: id.clone(),
        priority,
        duration_sec,
        schedules: Vec::new(),
    });
    row.priority = priority;
    row.duration_sec = duration_sec;
    if any_scheduled && let Some(start) = min_start {
        row.schedules.push(CohortBlockSchedulePlacement {
            manifest_id: manifest_id.to_string(),
            algorithm: algorithm.to_string(),
            start_mjd_utc: start,
        });
    }
}

fn harvest_legacy_block(
    block: &Value,
    manifest_id: &str,
    algorithm: &str,
    rows: &mut HashMap<String, CohortBlockRow>,
) {
    let Some(id) = block.get("id").and_then(json_id_to_string) else {
        return;
    };
    let priority = block
        .get("priority")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let duration_sec = block
        .get("duration_sec")
        .or_else(|| block.get("duration"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let row = rows.entry(id.clone()).or_insert(CohortBlockRow {
        block_id: id.clone(),
        priority,
        duration_sec,
        schedules: Vec::new(),
    });
    row.priority = priority;
    row.duration_sec = duration_sec;
    if let Some(start) = block
        .get("start_mjd_utc")
        .or_else(|| block.get("start"))
        .and_then(|v| v.as_f64())
    {
        row.schedules.push(CohortBlockSchedulePlacement {
            manifest_id: manifest_id.to_string(),
            algorithm: algorithm.to_string(),
            start_mjd_utc: start,
        });
    }
}

fn json_id_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Slugify a workspace name into a stable, filesystem-safe id.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("workspace");
    }
    out
}

/// Extract the SHA-256 of a workspace-stored schedule referenced by a
/// manifest, if its `artifacts.schedule.uri` matches the
/// `ws:///schedules/<sha>.json` convention written by `ingest_schedule`.
fn workspace_schedule_sha(manifest: &Manifest) -> Option<String> {
    let art: &ArtifactRef = manifest.artifacts.schedule.as_ref()?;
    let stripped = art.uri.strip_prefix(SCHEDULE_URI_PREFIX)?;
    let sha = stripped.strip_suffix(".json")?;
    if sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(sha.to_string())
    } else {
        None
    }
}

/// If the manifest's schedule artifact is a `file://` URI, return the
/// resulting filesystem path.
fn manifest_file_schedule_path(manifest: &Manifest) -> Option<PathBuf> {
    let art = manifest.artifacts.schedule.as_ref()?;
    let path = art.uri.strip_prefix("file://")?;
    Some(PathBuf::from(path))
}

/// Stable hash over the (sorted) block ids in a scheduling problem JSON,
/// matching the algorithm `phd sweep` uses.
fn block_pool_hash_from_ids(ids: &[String]) -> String {
    let mut sorted: Vec<&String> = ids.iter().collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    for id in sorted {
        hasher.update(id.as_bytes());
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

/// Inspect a self-contained schedule JSON and derive a [`WorkspaceContext`].
fn derive_workspace_context_from_schedule(schedule: &Value) -> WorkspaceContext {
    let mut ctx = WorkspaceContext::default();
    if let Some(period) = schedule
        .get("schedule_metadata")
        .and_then(|m| m.get("period"))
    {
        let s = period.get("start_mjd_utc").and_then(|v| v.as_f64());
        let e = period.get("end_mjd_utc").and_then(|v| v.as_f64());
        if let (Some(start_mjd_utc), Some(end_mjd_utc)) = (s, e) {
            ctx.period = Some(scheduler::manifest::Horizon {
                start_mjd_utc,
                end_mjd_utc,
            });
        }
    }
    if let Some(name) = schedule
        .get("schedule_metadata")
        .and_then(|m| m.get("location"))
        .and_then(|l| l.get("name"))
        .and_then(|v| v.as_str())
    {
        ctx.observatory_id = Some(name.to_string());
    }
    let block_ids: Vec<String> = schedule
        .get("scheduling_blocks")
        .or_else(|| schedule.get("blocks"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("id").and_then(json_id_to_string))
                .collect()
        })
        .unwrap_or_default();
    if !block_ids.is_empty() {
        ctx.block_count = Some(block_ids.len() as u64);
        ctx.block_pool_hash = Some(block_pool_hash_from_ids(&block_ids));
    }
    ctx
}

/// Public re-export so route layer (`ingest_schedule`) can populate the
/// extension subkey on the derived manifest.
pub(crate) fn workspace_context_from_schedule(schedule: &Value) -> WorkspaceContext {
    derive_workspace_context_from_schedule(schedule)
}

impl WorkspaceStore {
    /// Compute `(total_block_count, schedulable_block_count, completed_block_count)`
    /// for a manifest. Returns `None` when the input scheduling problem
    /// can not be resolved.
    ///
    /// `completed_block_count` is `0` when the schedule itself can not be
    /// resolved.
    pub fn block_ratios_for_manifest(
        &self,
        workspace_id: &str,
        manifest: &Manifest,
    ) -> WorkspaceResult<Option<(u64, u64, u64)>> {
        // Resolve the SchedulingProblem.
        let resolved_schedule = self
            .resolve_schedule_for_manifest(workspace_id, manifest)
            .ok();
        let problem = match self.resolve_problem_for_manifest(manifest, resolved_schedule.as_ref())
        {
            Some(p) => p,
            None => return Ok(None),
        };

        // Pick the horizon — prefer the workspace_context period.
        let horizon = manifest
            .workspace_context()
            .and_then(|c| c.period)
            .unwrap_or(manifest.horizon);

        let entry = match self.preschedule_cache.get_or_compute(&problem, horizon) {
            Ok(e) => e,
            Err(e) => {
                return Err(WorkspaceError::Internal(format!(
                    "preschedule cache failed: {e}"
                )));
            }
        };
        let total = entry.total_block_count;
        let schedulable = entry.schedulable_block_ids.len() as u64;

        // Completed: if we have a schedule, reconstruct placements and
        // count blocks that are complete. Otherwise 0.
        let completed = if let Some(schedule_value) = resolved_schedule {
            count_completed_blocks(&problem, &schedule_value)
        } else {
            0
        };

        Ok(Some((total, schedulable, completed)))
    }

    /// Resolve a [`SchedulingProblem`] for a manifest.  Order:
    /// 1. From a workspace-stored / file:// schedule (envelope shape).
    /// 2. From `manifest.dataset.source_path` if it points to a readable file.
    fn resolve_problem_for_manifest(
        &self,
        manifest: &Manifest,
        resolved_schedule: Option<&Value>,
    ) -> Option<scheduler::SchedulingProblem> {
        if let Some(value) = resolved_schedule
            && let Ok(p) = serde_json::from_value::<scheduler::SchedulingProblem>(value.clone())
        {
            return Some(p);
        }
        if let Some(path) = manifest_file_schedule_path(manifest)
            && let Ok(bytes) = fs::read(&path)
            && let Ok(p) = serde_json::from_slice::<scheduler::SchedulingProblem>(&bytes)
        {
            return Some(p);
        }
        let dataset_path = Path::new(&manifest.dataset.source_path);
        if dataset_path.is_file()
            && let Ok(bytes) = fs::read(dataset_path)
            && let Ok(p) = serde_json::from_slice::<scheduler::SchedulingProblem>(&bytes)
        {
            return Some(p);
        }
        None
    }
}

fn count_completed_blocks(problem: &scheduler::SchedulingProblem, schedule_value: &Value) -> u64 {
    let placements = collect_task_placements(schedule_value);
    if placements.is_empty() {
        return 0;
    }
    let mut schedule = scheduler::Schedule::new();
    for (task_id, start, end) in placements {
        let placement = scheduler::TaskPlacement {
            task_id: scheduler::time::TaskId(task_id),
            start: scheduler::time::Time::<scheduler::time::MJD>::new(start),
            end: scheduler::time::Time::<scheduler::time::MJD>::new(end),
        };
        schedule.insert_placement(placement);
    }
    let mut count = 0u64;
    for block in problem.blocks() {
        if block.is_complete(&schedule) {
            count += 1;
        }
    }
    count
}

fn collect_task_placements(schedule_value: &Value) -> Vec<(u64, f64, f64)> {
    let mut out = Vec::new();
    let blocks_iter: Box<dyn Iterator<Item = &Value>> = if let Some(arr) = schedule_value
        .get("scheduling_blocks")
        .and_then(|v| v.as_array())
    {
        Box::new(arr.iter())
    } else if let Some(arr) = schedule_value.get("blocks").and_then(|v| v.as_array()) {
        Box::new(arr.iter())
    } else {
        return out;
    };
    for block in blocks_iter {
        let Some(tasks) = block.get("tasks").and_then(|v| v.as_array()) else {
            continue;
        };
        for task in tasks {
            let scheduled = task
                .get("scheduled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !scheduled {
                continue;
            }
            let id = task.get("id").and_then(|v| v.as_u64());
            let start = task.get("scheduled_start_mjd_utc").and_then(|v| v.as_f64());
            let end = task.get("scheduled_end_mjd_utc").and_then(|v| v.as_f64());
            if let (Some(id), Some(start), Some(end)) = (id, start, end) {
                out.push((id, start, end));
            }
        }
    }
    out
}

/// JSON convenience: the `Manifest` validator on add. Keeps the route
/// layer thin and the policy in one place.
pub fn validate_manifest_payload(value: &Value) -> WorkspaceResult<Manifest> {
    let m: Manifest = serde_json::from_value(value.clone())
        .map_err(|e| WorkspaceError::InvalidManifest(format!("schema mismatch: {e}")))?;
    let report = m.validate();
    if report.status == ValidationStatus::Invalid {
        let msg = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, scheduler::manifest::IssueSeverity::Error))
            .map(|i| format!("{}: {}", i.code, i.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(WorkspaceError::InvalidManifest(msg));
    }
    Ok(m)
}
