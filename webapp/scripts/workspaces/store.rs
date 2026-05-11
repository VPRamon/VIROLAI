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
use scheduler::manifest::{ArtifactRef, Manifest, ValidationStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::workspaces::errors::{WorkspaceError, WorkspaceResult};

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
}

/// In-memory cache + filesystem persistence for workspaces.
pub struct WorkspaceStore {
    root: PathBuf,
    /// `workspace_id` → record (mirrors `index.json`).
    workspaces: Mutex<HashMap<String, WorkspaceRecord>>,
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
    /// `artifacts.schedule.uri`, when it points into this workspace.
    pub fn get_schedule_for_manifest(
        &self,
        workspace_id: &str,
        manifest_id: &str,
    ) -> WorkspaceResult<Value> {
        let manifest = self.get_manifest(workspace_id, manifest_id)?;
        let sha = workspace_schedule_sha(&manifest).ok_or_else(|| {
            WorkspaceError::NotFound(format!(
                "manifest `{manifest_id}` has no workspace-stored schedule"
            ))
        })?;
        self.get_schedule(workspace_id, &sha)
    }

    /// List schedule artifacts registered in the workspace. Cheap: only
    /// reads the index file.
    pub fn list_schedules(&self, workspace_id: &str) -> WorkspaceResult<Vec<ScheduleArtifact>> {
        self.ensure_workspace_exists(workspace_id)?;
        let idx: WsIndex = read_or_default(&self.workspace_dir(workspace_id).join(WS_INDEX_FILE))?;
        Ok(idx.schedules)
    }

    /// Build comparison summaries straight from manifest JSON. **Does
    /// not load the referenced full schedules.**
    pub fn comparison_summary(&self, workspace_id: &str) -> WorkspaceResult<Vec<ManifestSummary>> {
        let entries = self.list_manifests(workspace_id)?;
        let stored_shas: std::collections::HashSet<String> = self
            .list_schedules(workspace_id)?
            .into_iter()
            .map(|s| s.sha256)
            .collect();
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            let m = self.get_manifest(workspace_id, &e.manifest_id)?;
            let has_full_schedule = m.artifacts.schedule.is_some()
                || workspace_schedule_sha(&m)
                    .map(|sha| stored_shas.contains(&sha))
                    .unwrap_or(false);
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
