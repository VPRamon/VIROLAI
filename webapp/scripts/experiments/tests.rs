//! Integration tests for the experiments domain.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::experiments::catalog::{Catalog, RunStatus};
use crate::experiments::orchestrator::ExperimentRunner;
use crate::experiments::routes::{ExperimentsState, experiments_router};

const SAMPLE_METRICS: &str = r#"{
  "scheduled_task_count": 8,
  "total_task_count": 10,
  "completion_ratio": 0.8,
  "priority": {"count":8,"sum":40.0,"min":1.0,"max":9.0,"mean":5.0,"std":2.0,"p25":3.0,"p50":5.0,"p75":7.0,"p90":8.5},
  "fragmentation": {"gap_count":3,"gap_total_sec":900.0,"largest_gap_sec":500.0,"fragmentation_index":0.1},
  "total_horizon_sec": 86400.0,
  "available_time_sec": 86400.0,
  "scheduled_time_sec": 60000.0,
  "utilization": 0.7,
  "per_resource": [{"resource_id":"r","scheduled_task_count":8,"scheduled_time_sec":60000.0,"priority_sum":40.0,"utilization":0.7}],
  "composite_rank_score": 0.65,
  "ranking_weights": {"completion":1.0,"priority":1.0,"utilization":1.0,"fragmentation":1.0}
}"#;

fn write_metrics(dir: &Path, cell_id: &str, completion: f64, frag: f64) {
    let mut v: Value = serde_json::from_str(SAMPLE_METRICS).unwrap();
    v["completion_ratio"] = json!(completion);
    v["fragmentation"]["fragmentation_index"] = json!(frag);
    v["composite_rank_score"] = json!(completion - frag);
    std::fs::write(
        dir.join("metrics").join(format!("{cell_id}.json")),
        serde_json::to_string(&v).unwrap(),
    )
    .unwrap();
}

fn build_fixture() -> (TempDir, String, String) {
    let tmp = TempDir::new().unwrap();
    let slug = "demo";
    let run_id = "run-20240101T000000-000000000Z";
    let run_dir = tmp.path().join(slug).join(run_id);
    std::fs::create_dir_all(run_dir.join("metrics")).unwrap();
    std::fs::create_dir_all(run_dir.join("schedules")).unwrap();
    std::fs::create_dir_all(run_dir.join("traces")).unwrap();

    // Write an experiment.json with two cells.
    let manifest = json!({
        "spec": { "name": "Demo Experiment", "datasets": [], "algorithms": [], "output_dir": "" },
        "cells": [
            { "cell_id": "ds1__est__k1", "dataset_id": "ds1", "algorithm": "est" },
            { "cell_id": "ds2__hap__r2", "dataset_id": "ds2", "algorithm": "hap" }
        ]
    });
    std::fs::write(
        run_dir.join("experiment.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Per-cell artefacts.
    write_metrics(&run_dir, "ds1__est__k1", 0.9, 0.1);
    write_metrics(&run_dir, "ds2__hap__r2", 0.7, 0.4);
    std::fs::write(
        run_dir.join("schedules").join("ds1__est__k1.json"),
        r#"{"schedule":[]}"#,
    )
    .unwrap();
    std::fs::write(
        run_dir.join("traces").join("ds1__est__k1.jsonl"),
        "{\"step\":1}\n{\"step\":2}\n",
    )
    .unwrap();
    std::fs::write(run_dir.join("summary.csv"), "cell_id\nds1__est__k1\n").unwrap();

    // state.jsonl with two completed events.
    let mut state = String::new();
    state.push_str(
        &serde_json::to_string(&json!({
            "cell_id": "ds1__est__k1",
            "status": "completed",
            "schedule_path": "schedules/ds1__est__k1.json",
            "metrics_path": "metrics/ds1__est__k1.json",
            "started_at": "2024-01-01T00:00:00Z",
            "finished_at": "2024-01-01T00:00:01Z"
        }))
        .unwrap(),
    );
    state.push('\n');
    state.push_str(
        &serde_json::to_string(&json!({
            "cell_id": "ds2__hap__r2",
            "status": "completed",
            "metrics_path": "metrics/ds2__hap__r2.json",
            "started_at": "2024-01-01T00:00:00Z",
            "finished_at": "2024-01-01T00:00:02Z"
        }))
        .unwrap(),
    );
    state.push('\n');
    std::fs::write(run_dir.join("state.jsonl"), state).unwrap();

    (tmp, slug.to_string(), run_id.to_string())
}

fn build_router(root: &Path) -> Router {
    let catalog = Arc::new(Catalog::new(root.to_path_buf()));
    let runner = Arc::new(ExperimentRunner::new(
        root.to_path_buf(),
        Some("/usr/bin/true".to_string()),
        1,
        catalog.clone(),
    ));
    let state = Arc::new(ExperimentsState {
        root: root.to_path_buf(),
        catalog,
        runner,
    });
    experiments_router::<()>(state).with_state(())
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("body not JSON: {e}: {:?}", bytes))
}

#[tokio::test]
async fn catalog_discovers_fixture() {
    let (tmp, slug, run_id) = build_fixture();
    let cat = Catalog::new(tmp.path().to_path_buf());
    let entries = cat.list_experiments().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.experiment_slug, slug);
    assert_eq!(entry.run_id, run_id);
    assert_eq!(entry.total_cells, 2);
    assert_eq!(entry.completed_cells, 2);
    assert!(matches!(
        entry.status,
        RunStatus::Completed | RunStatus::Running
    ));
}

#[tokio::test]
async fn list_experiments_endpoint() {
    let (tmp, _, _) = build_fixture();
    let app = build_router(tmp.path());
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/experiments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["experiments"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn bulk_metrics_endpoint() {
    let (tmp, slug, run_id) = build_fixture();
    let app = build_router(tmp.path());
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/experiments/{slug}/runs/{run_id}/cells/bulk"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"cell_ids": ["ds1__est__k1", "ds2__hap__r2", "missing"]}).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    // first two have metrics, last has error
    assert!(items[0].get("metrics").is_some());
    assert!(items[1].get("metrics").is_some());
    assert!(items[2].get("error").is_some());
}

#[tokio::test]
async fn pareto_endpoint_picks_dominating_cell() {
    let (tmp, slug, run_id) = build_fixture();
    let app = build_router(tmp.path());
    // ds1 has higher completion AND lower fragmentation → dominates ds2.
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/v1/experiments/{slug}/runs/{run_id}/pareto?x=completion_ratio&y=fragmentation_index&xmax=true&ymax=false"
        ))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let front = body["front"].as_array().unwrap();
    assert_eq!(front.len(), 1);
    assert_eq!(front[0]["cell_id"], "ds1__est__k1");
}

#[tokio::test]
async fn ranking_endpoint_groups_by_dataset() {
    let (tmp, slug, run_id) = build_fixture();
    let app = build_router(tmp.path());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/v1/experiments/{slug}/runs/{run_id}/ranking?by=dataset"
        ))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    // ds1 has higher composite_rank_score (0.8 vs 0.3) → first.
    assert_eq!(entries[0]["key"], "ds1");
}

#[tokio::test]
async fn list_cells_endpoint_returns_summaries() {
    let (tmp, slug, run_id) = build_fixture();
    let app = build_router(tmp.path());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/experiments/{slug}/runs/{run_id}/cells"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let cells = body["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 2);
}

#[tokio::test]
async fn submit_invalid_spec_yields_422() {
    let tmp = TempDir::new().unwrap();
    let app = build_router(tmp.path());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/experiments")
        .header("content-type", "application/json")
        .body(Body::from(json!({}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn submit_with_dummy_binary_times_out_cleanly() {
    // Use `/bin/true` as a stand-in matrix binary: it ignores --spec and
    // exits immediately without creating a run dir, so submit() should
    // surface a clean "did not create a run directory" error rather than
    // hanging or panicking.
    let tmp = TempDir::new().unwrap();
    let catalog = Arc::new(Catalog::new(tmp.path().to_path_buf()));
    let runner = ExperimentRunner::new(
        tmp.path().to_path_buf(),
        Some("/bin/true".to_string()),
        1,
        catalog.clone(),
    );

    let spec = json!({
        "name": "Smoke",
        "datasets": [
            { "id": "ds_smoke", "path": "/nonexistent/does_not_matter.json" }
        ],
        "algorithms": [
            { "kind": "est", "axes": { "k_beams": [1] } }
        ],
        "output_dir": tmp.path().to_string_lossy()
    });

    // Shorten the wait window to keep the test fast: the orchestrator's
    // built-in 10s deadline applies here, so we put it inside a tokio
    // timeout to fail loudly if something regresses.
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(15), runner.submit(spec))
        .await
        .expect("submit completed within timeout");
    assert!(
        outcome.is_err(),
        "expected submit to error when matrix never produces a run dir"
    );
}

#[tokio::test]
async fn sse_replay_emits_existing_events() {
    use futures::StreamExt;
    let (tmp, slug, run_id) = build_fixture();
    let app = build_router(tmp.path());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/experiments/{slug}/runs/{run_id}/events"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body().into_data_stream();
    // Read until we see two `event: state` lines or 2 seconds elapse.
    let mut text = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let chunk = tokio::time::timeout(std::time::Duration::from_millis(500), stream.next())
            .await
            .ok()
            .and_then(|x| x);
        let Some(Ok(b)) = chunk else { continue };
        text.push_str(&String::from_utf8_lossy(&b));
        let n = text.matches("event: state").count();
        if n >= 2 {
            break;
        }
    }
    assert!(
        text.matches("event: state").count() >= 2,
        "expected ≥2 state events in SSE stream, got: {text}"
    );
}
