//! REST endpoints for the workspaces domain. Mounted under `/v1`.
//!
//! Endpoints (all JSON unless noted):
//!
//! ```text
//! GET    /v1/workspaces                                list (active by default; ?include_archived=1)
//! POST   /v1/workspaces                                create   { name, description? }
//! GET    /v1/workspaces/{id}                           detail
//! PATCH  /v1/workspaces/{id}                           rename / archive
//! DELETE /v1/workspaces/{id}                           delete (cascades manifests in this ws only)
//! GET    /v1/workspaces/{id}/manifests                 list manifest entries
//! POST   /v1/workspaces/{id}/manifests                 add one  ({manifest, idempotency_key?})
//! POST   /v1/workspaces/{id}/manifests/batch           add many ({items: [{manifest, idempotency_key?}, ...]})
//! GET    /v1/workspaces/{id}/manifests/{mid}           full manifest payload
//! GET    /v1/workspaces/{id}/manifests/{mid}/schedule  full schedule payload (drill-down; 404 if not stored)
//! DELETE /v1/workspaces/{id}/manifests/{mid}           remove (?delete_artifact=1 to drop manifest + orphan schedule)
//! GET    /v1/workspaces/{id}/comparison                lightweight summary across all manifests (no schedules)
//! POST   /v1/workspaces/{id}/schedules                 ingest one schedule JSON → persist + auto-build manifest
//! POST   /v1/workspaces/{id}/schedules/batch           ingest many schedules
//! ```

use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use schedulers::manifest::{
    AlgorithmRef, ArtifactRef, Artifacts, DatasetRef, Horizon, Links, MANIFEST_SCHEMA_VERSION,
    Manifest, Producer, Provenance, RunInfo, RunKind, RunStatus,
};
use schedulers::metrics::ScheduleMetrics;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::workspaces::errors::{WorkspaceError, WorkspaceResult};
use crate::workspaces::store::{
    CohortBlockRow, CohortSummary, ManifestEntry, ManifestSummary, WorkspaceRecord,
    WorkspaceStatus, WorkspaceStore, validate_manifest_payload,
};

#[derive(Clone)]
pub struct WorkspacesState {
    pub store: Arc<WorkspaceStore>,
}

pub fn workspaces_router<S>(state: Arc<WorkspacesState>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::<S>::new()
        .route("/workspaces", get(list_workspaces).post(create_workspace))
        .route(
            "/workspaces/{id}",
            get(get_workspace)
                .patch(update_workspace)
                .delete(delete_workspace),
        )
        .route(
            "/workspaces/{id}/manifests",
            get(list_manifests).post(add_manifest),
        )
        .route("/workspaces/{id}/manifests/batch", post(add_manifest_batch))
        .route(
            "/workspaces/{id}/manifests/{mid}",
            get(get_manifest).delete(remove_manifest),
        )
        .route(
            "/workspaces/{id}/manifests/{mid}/schedule",
            get(get_manifest_schedule),
        )
        .route("/workspaces/{id}/comparison", get(comparison_summary))
        .route("/workspaces/{id}/cohorts", get(list_cohorts))
        .route(
            "/workspaces/{id}/cohorts/{cohort_key}/blocks",
            get(cohort_blocks),
        )
        .route("/workspaces/{id}/schedules", post(ingest_schedule))
        .route(
            "/workspaces/{id}/schedules/batch",
            post(ingest_schedule_batch),
        )
        .layer(Extension(state))
}

// ── workspace handlers ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    include_archived: Option<u8>,
}

async fn list_workspaces(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Query(q): Query<ListQuery>,
) -> WorkspaceResult<Json<Value>> {
    let include_archived = q.include_archived.unwrap_or(0) != 0;
    let entries: Vec<WorkspaceRecord> = state.store.list_workspaces(include_archived);
    Ok(Json(json!({ "workspaces": entries })))
}

#[derive(Debug, Deserialize)]
struct CreateBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

async fn create_workspace(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Json(body): Json<CreateBody>,
) -> WorkspaceResult<(StatusCode, Json<Value>)> {
    let rec = state.store.create_workspace(body.name, body.description)?;
    tracing::info!(target: "phd.workspaces", workspace = %rec.id, "workspace.created");
    Ok((StatusCode::CREATED, Json(json!({ "workspace": rec }))))
}

async fn get_workspace(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
) -> WorkspaceResult<Json<Value>> {
    let rec = state.store.get_workspace(&id)?;
    let manifests = state.store.list_manifests(&id)?;
    Ok(Json(json!({ "workspace": rec, "manifests": manifests })))
}

#[derive(Debug, Deserialize)]
struct UpdateBody {
    #[serde(default)]
    name: Option<String>,
    /// Use a missing key to leave description unchanged; `null` to clear it.
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    description: Option<Option<String>>,
    #[serde(default)]
    status: Option<WorkspaceStatus>,
}

fn deserialize_optional_field<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

async fn update_workspace(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> WorkspaceResult<Json<Value>> {
    let rec = state
        .store
        .update_workspace(&id, body.name, body.description, body.status)?;
    Ok(Json(json!({ "workspace": rec })))
}

async fn delete_workspace(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
) -> WorkspaceResult<impl IntoResponse> {
    state.store.delete_workspace(&id)?;
    tracing::warn!(target: "phd.workspaces", workspace = %id, "workspace.deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ── manifest handlers ─────────────────────────────────────────────────

async fn list_manifests(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
) -> WorkspaceResult<Json<Value>> {
    let entries: Vec<ManifestEntry> = state.store.list_manifests(&id)?;
    Ok(Json(json!({ "manifests": entries })))
}

#[derive(Debug, Deserialize)]
struct AddManifestBody {
    manifest: Value,
    #[serde(default)]
    idempotency_key: Option<String>,
}

async fn add_manifest(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
    Json(body): Json<AddManifestBody>,
) -> WorkspaceResult<(StatusCode, Json<Value>)> {
    let manifest = validate_manifest_payload(&body.manifest)?;
    let (entry, created) = state
        .store
        .add_manifest(&id, &manifest, body.idempotency_key)?;
    tracing::info!(
        target: "phd.workspaces",
        workspace = %id,
        manifest = %entry.manifest_id,
        created,
        "manifest.imported"
    );
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(json!({ "manifest": entry, "created": created })),
    ))
}

#[derive(Debug, Deserialize)]
struct BatchBody {
    items: Vec<AddManifestBody>,
}

async fn add_manifest_batch(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
    Json(body): Json<BatchBody>,
) -> WorkspaceResult<Json<Value>> {
    if body.items.is_empty() {
        return Err(WorkspaceError::BadRequest("items cannot be empty".into()));
    }
    let mut created = 0usize;
    let mut deduped = 0usize;
    let mut failed = 0usize;
    let mut results: Vec<Value> = Vec::with_capacity(body.items.len());
    for item in body.items {
        match validate_manifest_payload(&item.manifest)
            .and_then(|m| state.store.add_manifest(&id, &m, item.idempotency_key))
        {
            Ok((entry, c)) => {
                if c {
                    created += 1;
                } else {
                    deduped += 1;
                }
                results.push(json!({ "ok": true, "created": c, "manifest": entry }));
            }
            Err(e) => {
                failed += 1;
                results.push(json!({ "ok": false, "error": {
                    "message": e.to_string(),
                }}));
            }
        }
    }
    tracing::info!(
        target: "phd.workspaces",
        workspace = %id,
        created,
        deduplicated = deduped,
        failed,
        "batch.imported"
    );
    Ok(Json(json!({
        "summary": {
            "created": created,
            "deduplicated": deduped,
            "failed": failed,
        },
        "results": results,
    })))
}

async fn get_manifest(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path((id, mid)): Path<(String, String)>,
) -> WorkspaceResult<Json<Value>> {
    let manifest = state.store.get_manifest(&id, &mid)?;
    Ok(Json(json!({ "manifest": manifest })))
}

#[derive(Debug, Deserialize)]
struct RemoveQuery {
    #[serde(default)]
    delete_artifact: Option<u8>,
}

async fn remove_manifest(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path((id, mid)): Path<(String, String)>,
    Query(q): Query<RemoveQuery>,
) -> WorkspaceResult<impl IntoResponse> {
    state
        .store
        .remove_manifest(&id, &mid, q.delete_artifact.unwrap_or(0) != 0)?;
    tracing::info!(
        target: "phd.workspaces",
        workspace = %id,
        manifest = %mid,
        delete_artifact = q.delete_artifact.unwrap_or(0) != 0,
        "manifest.removed"
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn comparison_summary(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
) -> WorkspaceResult<Json<Value>> {
    let summaries: Vec<ManifestSummary> = state.store.comparison_summary(&id)?;
    Ok(Json(json!({ "summaries": summaries })))
}

async fn list_cohorts(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
) -> WorkspaceResult<Json<Value>> {
    let cohorts: Vec<CohortSummary> = state.store.list_cohorts(&id)?;
    Ok(Json(json!({ "cohorts": cohorts })))
}

async fn cohort_blocks(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path((id, cohort_key)): Path<(String, String)>,
) -> WorkspaceResult<Json<Value>> {
    let blocks: Vec<CohortBlockRow> = state.store.cohort_blocks(&id, &cohort_key)?;
    Ok(Json(json!({ "blocks": blocks })))
}

// ── schedule ingestion ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct IngestScheduleBody {
    schedule: Value,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IngestScheduleBatchBody {
    items: Vec<IngestScheduleBody>,
}

/// Build a [`Manifest`] from a self-contained schedule JSON (one that
/// carries embedded `schedule_metadata` and `schedule_metrics` fields,
/// as produced by `phd sweep`).
///
/// The resulting manifest does **not** populate `artifacts.schedule` —
/// that is filled in by [`ingest_schedule`] after persisting the
/// schedule body, so the URI/SHA-256 reflect the workspace store.
fn manifest_from_schedule(schedule: &Value) -> Result<Manifest, String> {
    let meta = schedule
        .get("schedule_metadata")
        .ok_or("schedule JSON missing `schedule_metadata`")?;

    let algorithm = meta
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let algorithm_config = meta.get("algorithm_config").cloned().unwrap_or(Value::Null);
    let dataset_id = meta
        .get("dataset_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let dataset_label = meta
        .get("dataset_label")
        .and_then(|v| v.as_str())
        .unwrap_or(dataset_id.as_str())
        .to_string();

    let (start_mjd, end_mjd) = if let Some(period) = meta.get("period") {
        let s = period
            .get("start_mjd_utc")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let e = period
            .get("end_mjd_utc")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        (s, e)
    } else {
        (0.0, 0.0)
    };

    let metrics: ScheduleMetrics = if let Some(m) = schedule.get("schedule_metrics") {
        serde_json::from_value(m.clone())
            .map_err(|e| format!("failed to parse `schedule_metrics`: {e}"))?
    } else {
        return Err(
            "schedule JSON missing `schedule_metrics` — rebuild with `phd sweep` (recent version)"
                .to_string(),
        );
    };

    let now = chrono::Utc::now().to_rfc3339();
    let manifest_id = uuid::Uuid::new_v4().to_string();

    Ok(Manifest {
        manifest_schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        manifest_id,
        created_at: now.clone(),
        producer: Producer {
            name: "phd-tsi-server".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: None,
            host: None,
        },
        dataset: DatasetRef {
            id: dataset_id,
            name: dataset_label,
            source_path: String::new(),
            sha256: String::new(),
            schema_version: "scheduling_problem/1".to_string(),
        },
        algorithm: AlgorithmRef {
            id: algorithm.clone(),
            label: algorithm.to_uppercase(),
            version: String::new(),
            config: algorithm_config,
        },
        run: RunInfo {
            run_id: "webapp-ingest".to_string(),
            kind: RunKind::MatrixCell,
            started_at: now.clone(),
            finished_at: now.clone(),
            status: RunStatus::Completed,
            exit_code: 0,
        },
        horizon: Horizon {
            start_mjd_utc: start_mjd,
            end_mjd_utc: end_mjd,
        },
        metrics,
        artifacts: Artifacts::default(),
        links: Links::default(),
        provenance: Provenance::default(),
        validation: Default::default(),
        extensions: Value::Null,
    })
}

/// Persist a schedule body, derive its manifest, fill
/// `artifacts.schedule` with the workspace-relative URI, and register
/// the manifest in the workspace.
fn ingest_one(
    store: &WorkspaceStore,
    workspace_id: &str,
    body: IngestScheduleBody,
) -> WorkspaceResult<(ManifestEntry, bool)> {
    let mut manifest =
        manifest_from_schedule(&body.schedule).map_err(WorkspaceError::BadRequest)?;
    let (sha, size, _stored_at) = store.put_schedule(workspace_id, &body.schedule)?;
    manifest.artifacts.schedule = Some(ArtifactRef {
        uri: format!("{}{}.json", WorkspaceStore::schedule_uri_prefix(), sha),
        size_bytes: size,
        sha256: sha,
        media_type: "application/json".to_string(),
    });
    // Derive workspace_context from the schedule when the manifest does
    // not already carry a fully populated one.
    let derived = crate::workspaces::store::workspace_context_from_schedule(&body.schedule);
    let existing = manifest.workspace_context();
    let merged = merge_workspace_context(existing, derived);
    if merged != schedulers::manifest::WorkspaceContext::default() {
        manifest.extensions = serde_json::json!({ "workspace_context": merged });
    }
    // Re-run the structural validator with the new artifact ref so the
    // stored manifest carries an up-to-date validation report.
    manifest.validation = manifest.validate();
    store.add_manifest(workspace_id, &manifest, body.idempotency_key)
}

fn merge_workspace_context(
    existing: Option<schedulers::manifest::WorkspaceContext>,
    derived: schedulers::manifest::WorkspaceContext,
) -> schedulers::manifest::WorkspaceContext {
    let mut out = existing.unwrap_or_default();
    if out.observatory_id.is_none() {
        out.observatory_id = derived.observatory_id;
    }
    if out.period.is_none() {
        out.period = derived.period;
    }
    if out.block_pool_hash.is_none() {
        out.block_pool_hash = derived.block_pool_hash;
    }
    if out.block_count.is_none() {
        out.block_count = derived.block_count;
    }
    out
}

async fn ingest_schedule(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
    Json(body): Json<IngestScheduleBody>,
) -> WorkspaceResult<(StatusCode, Json<Value>)> {
    let (entry, created) = ingest_one(&state.store, &id, body)?;
    tracing::info!(
        target: "phd.workspaces",
        workspace = %id,
        manifest = %entry.manifest_id,
        created,
        "schedule.ingested"
    );
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(json!({ "manifest": entry, "created": created })),
    ))
}

async fn ingest_schedule_batch(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path(id): Path<String>,
    Json(body): Json<IngestScheduleBatchBody>,
) -> WorkspaceResult<Json<Value>> {
    if body.items.is_empty() {
        return Err(WorkspaceError::BadRequest("items cannot be empty".into()));
    }
    let mut created = 0usize;
    let mut deduped = 0usize;
    let mut failed = 0usize;
    let mut results: Vec<Value> = Vec::with_capacity(body.items.len());
    for item in body.items {
        match ingest_one(&state.store, &id, item) {
            Ok((entry, c)) => {
                if c {
                    created += 1;
                } else {
                    deduped += 1;
                }
                results.push(json!({ "ok": true, "created": c, "manifest": entry }));
            }
            Err(e) => {
                failed += 1;
                results.push(json!({
                    "ok": false,
                    "error": { "message": e.to_string() }
                }));
            }
        }
    }
    tracing::info!(
        target: "phd.workspaces",
        workspace = %id,
        created,
        deduplicated = deduped,
        failed,
        "schedules.batch.ingested"
    );
    Ok(Json(json!({
        "summary": {
            "created": created,
            "deduplicated": deduped,
            "failed": failed,
        },
        "results": results,
    })))
}

async fn get_manifest_schedule(
    Extension(state): Extension<Arc<WorkspacesState>>,
    Path((id, mid)): Path<(String, String)>,
) -> WorkspaceResult<Json<Value>> {
    let schedule = state.store.get_schedule_for_manifest(&id, &mid)?;
    Ok(Json(json!({ "schedule": schedule })))
}
