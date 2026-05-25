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
        "manifest_schema_version": "2.0.0",
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
            "scheduled_task_ratio": 0.5,
            "scheduled_priority": {"count":5,"sum":15.0,"min":1.0,"max":5.0,"mean":3.0,"std":1.0,"p25":2.0,"p50":3.0,"p75":4.0,"p90":4.5},
            "scheduled_priority_sum": 15.0,
            "total_priority_sum": 30.0,
            "scheduled_priority_ratio": 0.5,
            "priority_density": 1.0,
            "fragmentation": {"gap_count":1,"gap_total_sec":300.0,"largest_gap_sec":300.0,"fragmentation_index":0.05},
            "total_horizon_sec": 86400.0,
            "available_time_sec": 86400.0,
            "scheduled_time_sec": 50000.0,
            "utilization": 0.58,
            "per_resource": [],
            "composite_rank_score": 0.7,
            "ranking_weights": {"scheduled_task":1.0,"scheduled_priority":1.0,"utilization":1.0,"fragmentation":1.0},
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
            { "manifest": json!({"manifest_schema_version": "2.0.0"}) },
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

// ── schedule ingestion / persistence / GC ────────────────────────────

fn sample_schedule(algorithm: &str, dataset: &str, completion: f64) -> Value {
    json!({
        "schedule_metadata": {
            "algorithm": algorithm,
            "algorithm_config": {},
            "dataset_id": dataset,
            "dataset_label": dataset,
            "period": { "start_mjd_utc": 60000.0, "end_mjd_utc": 60001.0 }
        },
        "schedule_metrics": {
            "scheduled_task_count": 5,
            "total_task_count": 10,
            "scheduled_task_ratio": completion,
            "scheduled_priority": {"count":5,"sum":15.0,"min":1.0,"max":5.0,"mean":3.0,"std":1.0,"p25":2.0,"p50":3.0,"p75":4.0,"p90":4.5},
            "scheduled_priority_sum": 15.0,
            "total_priority_sum": 30.0,
            "scheduled_priority_ratio": 0.5,
            "priority_density": 1.0,
            "fragmentation": {"gap_count":1,"gap_total_sec":300.0,"largest_gap_sec":300.0,"fragmentation_index":0.05},
            "total_horizon_sec": 86400.0,
            "available_time_sec": 86400.0,
            "scheduled_time_sec": 50000.0,
            "utilization": 0.58,
            "per_resource": [],
            "composite_rank_score": 0.7,
            "ranking_weights": {"scheduled_task":1.0,"scheduled_priority":1.0,"utilization":1.0,"fragmentation":1.0},
            "scheduled_priority_stair": {
                "metric": "scheduled_priority_stair",
                "sort": "priority",
                "direction": "descending",
                "stairs": [],
                "total_scheduled_items": 5
            }
        },
        "events": []
    })
}

#[tokio::test]
async fn ingest_schedule_persists_and_drill_down() {
    let (tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({"name":"ws"})),
    )
    .await;

    let sched = sample_schedule("est", "ds-1", 0.5);
    let (status, body) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/schedules",
        Some(json!({ "schedule": sched })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = body["manifest"]["manifest_id"]
        .as_str()
        .unwrap()
        .to_string();

    // The schedule file lives under workspaces/ws/schedules/<sha>.json
    let schedules_dir = tmp.path().join("ws").join("schedules");
    let count = std::fs::read_dir(&schedules_dir).unwrap().count();
    assert_eq!(count, 1);

    // Comparison surfaces `has_full_schedule = true`.
    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces/ws/comparison", None).await;
    assert_eq!(body["summaries"][0]["has_full_schedule"], true);

    // Drill-down endpoint returns the schedule body.
    let (status, body) = json_call(
        &app,
        Method::GET,
        &format!("/v1/workspaces/ws/manifests/{mid}/schedule"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["schedule"]["schedule_metadata"]["algorithm"], "est");

    // The stored manifest carries the workspace-relative URI.
    let (_, body) = json_call(
        &app,
        Method::GET,
        &format!("/v1/workspaces/ws/manifests/{mid}"),
        None,
    )
    .await;
    let uri = body["manifest"]["artifacts"]["schedule"]["uri"]
        .as_str()
        .unwrap();
    assert!(uri.starts_with("ws:///schedules/"));
}

#[tokio::test]
async fn ingest_schedule_dedupes_by_sha() {
    let (tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({"name":"ws"})),
    )
    .await;

    let sched = sample_schedule("est", "ds-1", 0.5);
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/schedules",
        Some(json!({ "schedule": sched.clone(), "idempotency_key": "k1" })),
    )
    .await;
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/schedules",
        Some(json!({ "schedule": sched, "idempotency_key": "k2" })),
    )
    .await;

    // Two manifests, one shared schedule file.
    let schedules_dir = tmp.path().join("ws").join("schedules");
    assert_eq!(std::fs::read_dir(&schedules_dir).unwrap().count(), 1);
    let (_, body) = json_call(&app, Method::GET, "/v1/workspaces/ws/manifests", None).await;
    assert_eq!(body["manifests"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn schedule_gc_when_last_manifest_deleted() {
    let (tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({"name":"ws"})),
    )
    .await;

    let sched = sample_schedule("est", "ds-1", 0.5);
    let (_, body_a) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/schedules",
        Some(json!({ "schedule": sched.clone(), "idempotency_key": "ka" })),
    )
    .await;
    let (_, body_b) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/schedules",
        Some(json!({ "schedule": sched, "idempotency_key": "kb" })),
    )
    .await;
    let mid_a = body_a["manifest"]["manifest_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mid_b = body_b["manifest"]["manifest_id"]
        .as_str()
        .unwrap()
        .to_string();

    let schedules_dir = tmp.path().join("ws").join("schedules");
    assert_eq!(std::fs::read_dir(&schedules_dir).unwrap().count(), 1);

    // Delete the first manifest with delete_artifact: schedule still
    // referenced by the second manifest, so it must survive.
    let (status, _) = json_call(
        &app,
        Method::DELETE,
        &format!("/v1/workspaces/ws/manifests/{mid_a}?delete_artifact=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(std::fs::read_dir(&schedules_dir).unwrap().count(), 1);

    // Delete the second one too → schedule file should be GC'd.
    let (status, _) = json_call(
        &app,
        Method::DELETE,
        &format!("/v1/workspaces/ws/manifests/{mid_b}?delete_artifact=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(std::fs::read_dir(&schedules_dir).unwrap().count(), 0);
}

#[tokio::test]
async fn schedules_batch_summary_counts() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({"name":"ws"})),
    )
    .await;

    let body = json!({
        "items": [
            { "schedule": sample_schedule("est", "d1", 0.5), "idempotency_key": "a" },
            { "schedule": sample_schedule("hap", "d1", 0.7), "idempotency_key": "b" },
            // duplicate idempotency key → counted as deduplicated
            { "schedule": sample_schedule("est", "d1", 0.5), "idempotency_key": "a" },
            // missing schedule_metrics → failed
            { "schedule": json!({ "schedule_metadata": {} }) },
        ]
    });
    let (status, body) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/schedules/batch",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["summary"]["created"], 2);
    assert_eq!(body["summary"]["deduplicated"], 1);
    assert_eq!(body["summary"]["failed"], 1);
}

#[tokio::test]
async fn schedule_drill_down_404_when_not_stored() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({"name":"ws"})),
    )
    .await;

    // A plain manifest (no workspace-stored schedule) should yield 404
    // on the drill-down endpoint.
    let m = sample_manifest("m-x", "est", "ds");
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/ws/manifests",
        Some(json!({ "manifest": m })),
    )
    .await;
    let (status, _) = json_call(
        &app,
        Method::GET,
        "/v1/workspaces/ws/manifests/m-x/schedule",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── cohorts ────────────────────────────────────────────────────────────

fn manifest_with_cohort(
    manifest_id: &str,
    algorithm: &str,
    dataset: &str,
    observatory: Option<&str>,
    pool: Option<&str>,
) -> Value {
    let mut m = sample_manifest(manifest_id, algorithm, dataset);
    let mut ctx = serde_json::Map::new();
    if let Some(o) = observatory {
        ctx.insert("observatory_id".into(), Value::String(o.to_string()));
    }
    ctx.insert(
        "period".into(),
        json!({ "start_mjd_utc": 60000.0, "end_mjd_utc": 60001.0 }),
    );
    if let Some(p) = pool {
        ctx.insert("block_pool_hash".into(), Value::String(p.to_string()));
    }
    m["extensions"] = json!({ "workspace_context": Value::Object(ctx) });
    m
}

#[tokio::test]
async fn cohorts_group_manifests_by_workspace_context() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "Cohorts" })),
    )
    .await;
    // Two manifests share the same cohort (same dataset, observatory, pool).
    let m1 = manifest_with_cohort("m-1", "est", "ds-A", Some("CTA-N"), Some("pool-1"));
    let m2 = manifest_with_cohort("m-2", "fom", "ds-A", Some("CTA-N"), Some("pool-1"));
    // Third differs by observatory.
    let m3 = manifest_with_cohort("m-3", "est", "ds-A", Some("CTA-S"), Some("pool-1"));
    for m in [&m1, &m2, &m3] {
        let (status, _) = json_call(
            &app,
            Method::POST,
            "/v1/workspaces/cohorts/manifests",
            Some(json!({ "manifest": m })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, body) = json_call(&app, Method::GET, "/v1/workspaces/cohorts/cohorts", None).await;
    assert_eq!(status, StatusCode::OK);
    let cohorts = body["cohorts"].as_array().unwrap();
    assert_eq!(cohorts.len(), 2);
    let counts: Vec<u64> = cohorts
        .iter()
        .map(|c| c["manifest_count"].as_u64().unwrap())
        .collect();
    assert!(counts.contains(&2));
    assert!(counts.contains(&1));
}

#[tokio::test]
async fn cohort_fallback_groups_legacy_manifests() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "Legacy" })),
    )
    .await;
    // No workspace_context → fallback cohort by (dataset, horizon).
    let m1 = sample_manifest("m-1", "est", "ds-A");
    let m2 = sample_manifest("m-2", "fom", "ds-A");
    for m in [&m1, &m2] {
        let (status, _) = json_call(
            &app,
            Method::POST,
            "/v1/workspaces/legacy/manifests",
            Some(json!({ "manifest": m })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, body) = json_call(&app, Method::GET, "/v1/workspaces/legacy/cohorts", None).await;
    assert_eq!(status, StatusCode::OK);
    let cohorts = body["cohorts"].as_array().unwrap();
    assert_eq!(cohorts.len(), 1);
    assert_eq!(cohorts[0]["manifest_count"], 2);
}

#[tokio::test]
async fn cohort_blocks_only_uses_persisted_schedules() {
    let (_tmp, app) = build_app();
    let _ = json_call(
        &app,
        Method::POST,
        "/v1/workspaces",
        Some(json!({ "name": "Blocks" })),
    )
    .await;
    // A manifest with no stored schedule.
    let bare = manifest_with_cohort("m-bare", "est", "ds-A", Some("CTA-N"), Some("pool-1"));
    let (status, _) = json_call(
        &app,
        Method::POST,
        "/v1/workspaces/blocks/manifests",
        Some(json!({ "manifest": bare })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cohort_key = json_call(&app, Method::GET, "/v1/workspaces/blocks/cohorts", None)
        .await
        .1["cohorts"][0]["cohort_key"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = json_call(
        &app,
        Method::GET,
        &format!("/v1/workspaces/blocks/cohorts/{cohort_key}/blocks"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["blocks"].as_array().unwrap().is_empty());
}
