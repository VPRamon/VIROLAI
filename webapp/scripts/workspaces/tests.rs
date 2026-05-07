//! Integration tests for the workspaces domain.
//!
//! The router is exercised end-to-end via `tower::ServiceExt::oneshot`
//! against a temporary filesystem root so the assertions cover routing,
//! validation, persistence and lookups together.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::workspaces::routes::{WorkspacesState, workspaces_router};
use crate::workspaces::store::WorkspaceStore;

fn build_app() -> (TempDir, Router) {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(WorkspaceStore::open(tmp.path().to_path_buf()).unwrap());
    let state = Arc::new(WorkspacesState { store });
    let app = Router::<()>::new()
        .nest("/v1", workspaces_router::<()>(state))
        .with_state(());
    (tmp, app)
}

async fn json_call(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn sample_manifest(manifest_id: &str, algorithm: &str, dataset: &str) -> Value {
    json!({
        "manifest_schema_version": "1.0.0",
        "manifest_id": manifest_id,
        "created_at": "2026-05-07T10:00:00Z",
        "producer": { "name": "phd", "version": "0.1.0" },
        "dataset": {
            "id": dataset,
            "name": dataset,
            "source_path": "/tmp/dataset.json",
            "sha256": "0".repeat(64),
            "schema_version": "scheduling_problem/1"
        },
        "algorithm": {
            "id": algorithm,
            "label": algorithm.to_uppercase(),
            "version": "0.1.0",
            "config": {}
        },
        "run": {
            "run_id": "run-1",
            "kind": "single",
            "started_at": "2026-05-07T09:59:00Z",
            "finished_at": "2026-05-07T10:00:00Z",
            "status": "completed",
            "exit_code": 0
        },
        "horizon": { "start_mjd_utc": 60000.0, "end_mjd_utc": 60001.0 },
        "metrics": {
            "scheduled_task_count": 5,
            "total_task_count": 10,
            "completion_ratio": 0.5,
            "priority": {"count":5,"sum":15.0,"min":1.0,"max":5.0,"mean":3.0,"std":1.0,"p25":2.0,"p50":3.0,"p75":4.0,"p90":4.5},
            "fragmentation": {"gap_count":1,"gap_total_sec":300.0,"largest_gap_sec":300.0,"fragmentation_index":0.05},
            "total_horizon_sec": 86400.0,
            "available_time_sec": 86400.0,
            "scheduled_time_sec": 50000.0,
            "utilization": 0.58,
            "per_resource": [],
            "composite_rank_score": 0.7,
            "ranking_weights": {"completion":1.0,"priority":1.0,"utilization":1.0,"fragmentation":1.0},
            "scheduled_priority_stair": {
                "metric": "scheduled_priority_stair",
                "sort": "priority",
                "direction": "descending",
                "stairs": [
                    {"priority": 3.0, "start_index": 0, "end_index": 1, "count": 2},
                    {"priority": 1.0, "start_index": 2, "end_index": 4, "count": 3}
                ],
                "total_scheduled_items": 5
            }
        },
        "provenance": {}
    })
}

#[tokio::test]
async fn create_list_get_workspace() {
    let (_tmp, app) = build_app();

    let (status, body) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "Paper Comparisons" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["workspace"]["id"], "paper-comparisons");
    assert_eq!(body["workspace"]["status"], "active");

    let (status, body) = json_call(&app, Method::GET, "/v1/workspaces", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["workspaces"].as_array().unwrap().len(), 1);

    let (status, body) =
        json_call(&app, Method::GET, "/v1/workspaces/paper-comparisons", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["workspace"]["name"], "Paper Comparisons");
    assert_eq!(body["manifests"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn create_workspace_rejects_empty_name() {
    let (_tmp, app) = build_app();
    let (status, _) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn duplicate_workspace_returns_conflict() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "Same Name" })),
    )
    .await;
    let (status, _) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "Same Name" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn add_remove_manifest_lifecycle() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "ws" })),
    )
    .await;

    let m = sample_manifest("m-1", "est", "ds-1");
    let (status, body) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/manifests",
        Some(json!({ "manifest": m })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["created"], true);
    assert_eq!(body["manifest"]["manifest_id"], "m-1");

    // Re-add same idempotency key → no duplicate, 200 OK with created=false.
    let (status, body) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/manifests",
        Some(json!({ "manifest": sample_manifest("m-1", "est", "ds-1") })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"], false);

    // List shows exactly one entry.
    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces/ws/manifests", None).await;
    assert_eq!(body["manifests"].as_array().unwrap().len(), 1);

    // Comparison summary has no schedule loading and reports stair_block_count.
    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces/ws/comparison", None).await;
    let summary = &body["summaries"][0];
    assert_eq!(summary["stair_block_count"], 2);
    assert_eq!(summary["has_full_schedule"], false);

    // Remove and verify.
    let (status, _) = json_call(
        &app,
        Method::DELETE,
        "/v1/workspaces/ws/manifests/m-1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces/ws/manifests", None).await;
    assert_eq!(body["manifests"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn batch_import_summary_counts() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "ws" })),
    )
    .await;

    let body = json!({
        "items": [
            { "manifest": sample_manifest("a", "est", "d1") },
            { "manifest": sample_manifest("b", "hap", "d1") },
            // duplicate of the first → deduped:
            { "manifest": sample_manifest("a", "est", "d1") },
            // bogus payload → counted as failed but does not abort:
            { "manifest": json!({"manifest_schema_version": "1.0.0"}) },
        ]
    });
    let (status, body) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/manifests/batch",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["summary"]["created"], 2);
    assert_eq!(body["summary"]["deduplicated"], 1);
    assert_eq!(body["summary"]["failed"], 1);
}

#[tokio::test]
async fn invalid_manifest_returns_unprocessable() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "ws" })),
    )
    .await;
    let (status, _) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/manifests",
        Some(json!({ "manifest": { "wrong": "shape" } })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn archive_then_filter_lists() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "ws" })),
    )
    .await;

    let (status, body) = json_call(
        &app,
        Method::PATCH,
        "/v1/workspaces/ws",
        Some(json!({ "status": "archived" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["workspace"]["status"], "archived");

    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces", None).await;
    assert_eq!(body["workspaces"].as_array().unwrap().len(), 0);

    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces?include_archived=1", None).await;
    assert_eq!(body["workspaces"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_workspace_removes_storage() {
    let (tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "ws" })),
    )
    .await;
    assert!(tmp.path().join("ws").exists());

    let (status, _) = json_call(&app, Method::DELETE, "/v1/workspaces/ws", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!tmp.path().join("ws").exists());

    let (status, _) = json_call(&app, Method::GET, "/v1/workspaces/ws", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
