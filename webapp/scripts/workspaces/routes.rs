//! REST endpoints for the workspaces domain. Mounted under `/v1`.
//!
//! Endpoints (all JSON unless noted):
//!
//! ```text
//! GET    /v1/workspaces                       list (active by default; ?include_archived=1)
//! POST   /v1/workspaces                       create   { name, description? }
//! GET    /v1/workspaces/{id}                  detail
//! PATCH  /v1/workspaces/{id}                  rename / archive
//! DELETE /v1/workspaces/{id}                  delete (cascades manifests in this ws only)
//! GET    /v1/workspaces/{id}/manifests        list manifest entries
//! POST   /v1/workspaces/{id}/manifests        add one  ({manifest, idempotency_key?})
//! POST   /v1/workspaces/{id}/manifests/batch  add many ({items: [{manifest, idempotency_key?}, ...]})
//! GET    /v1/workspaces/{id}/manifests/{mid}  full manifest payload
//! DELETE /v1/workspaces/{id}/manifests/{mid}  remove (?delete_artifact=1 to drop the file)
//! GET    /v1/workspaces/{id}/comparison       lightweight summary across all manifests (no schedules)
//! ```

use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::workspaces::errors::{WorkspaceError, WorkspaceResult};
use crate::workspaces::store::{
    ManifestEntry, ManifestSummary, WorkspaceRecord, WorkspaceStatus, WorkspaceStore,
    validate_manifest_payload,
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
        .route("/workspaces/{id}/comparison", get(comparison_summary))
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
