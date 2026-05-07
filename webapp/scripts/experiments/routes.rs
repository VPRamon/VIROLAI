//! REST + SSE endpoints for the experiments domain. Mounted under `/v1`.

use axum::body::Body;
use axum::extract::{Extension, Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use scheduler::metrics::RankingWeights;
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::experiments::catalog::{Catalog, CellListFilter};
use crate::experiments::errors::{ExperimentError, ExperimentResult};
use crate::experiments::orchestrator::ExperimentRunner;
use crate::experiments::state_events;

#[derive(Clone)]
pub struct ExperimentsState {
    pub root: PathBuf,
    pub catalog: Arc<Catalog>,
    pub runner: Arc<ExperimentRunner>,
}

/// Build the `/v1/experiments/...` router. The router is parameterised
/// over `tsi_rust::http::AppState` so it merges cleanly into the TSI
/// backend; per-handler dependencies are injected via `Extension`.
pub fn experiments_router<S>(state: Arc<ExperimentsState>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::<S>::new()
        .route(
            "/v1/experiments",
            get(list_experiments).post(submit_experiment),
        )
        .route("/v1/experiments/{slug}/runs/{run_id}", get(get_experiment))
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/cancel",
            post(cancel_run),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/resume",
            post(resume_run),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/cells",
            get(list_cells),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/cells/bulk",
            post(bulk_cells),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/cells/{cell_id}",
            get(get_cell),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/cells/{cell_id}/schedule",
            get(get_cell_schedule),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/cells/{cell_id}/trace",
            get(get_cell_trace),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/summary.csv",
            get(get_summary_csv),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/pareto",
            get(get_pareto),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/ranking",
            get(get_ranking),
        )
        .route(
            "/v1/experiments/{slug}/runs/{run_id}/events",
            get(stream_events),
        )
        .layer(Extension(state))
}

// ── handlers ───────────────────────────────────────────────────────────────

async fn list_experiments(
    Extension(state): Extension<Arc<ExperimentsState>>,
) -> ExperimentResult<Json<Value>> {
    let entries = state.catalog.list_experiments()?;
    Ok(Json(json!({ "experiments": entries })))
}

async fn submit_experiment(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Json(spec): Json<Value>,
) -> ExperimentResult<(StatusCode, Json<Value>)> {
    let result = state.runner.submit(spec).await?;
    Ok((StatusCode::ACCEPTED, Json(json!(result))))
}

async fn get_experiment(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
) -> ExperimentResult<Json<Value>> {
    let detail = state.catalog.get_experiment(&slug, &run_id)?;
    let live_status = state.runner.status(&slug, &run_id).await;
    Ok(Json(json!({
        "experiment": detail,
        "live_status": live_status,
    })))
}

async fn cancel_run(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
) -> ExperimentResult<Json<Value>> {
    state.runner.cancel(&slug, &run_id).await?;
    Ok(Json(
        json!({ "cancelled": true, "slug": slug, "run_id": run_id }),
    ))
}

async fn resume_run(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
) -> ExperimentResult<(StatusCode, Json<Value>)> {
    let result = state.runner.resume(&slug, &run_id).await?;
    Ok((StatusCode::ACCEPTED, Json(json!(result))))
}

async fn list_cells(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
    Query(filter): Query<CellListFilter>,
) -> ExperimentResult<Json<Value>> {
    let cells = state.catalog.list_cells(&slug, &run_id, &filter)?;
    Ok(Json(json!({ "cells": cells })))
}

#[derive(Debug, Deserialize)]
struct BulkBody {
    cell_ids: Vec<String>,
}

async fn bulk_cells(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
    Json(body): Json<BulkBody>,
) -> ExperimentResult<Json<Value>> {
    if body.cell_ids.is_empty() {
        return Err(ExperimentError::BadRequest(
            "cell_ids cannot be empty".into(),
        ));
    }
    let pairs = state.catalog.bulk_metrics(&slug, &run_id, &body.cell_ids)?;
    let mut items = Vec::with_capacity(pairs.len());
    for (cell_id, res) in pairs {
        match res {
            Ok(m) => items.push(json!({ "cell_id": cell_id, "metrics": m })),
            Err(e) => items.push(json!({
                "cell_id": cell_id,
                "error": e.to_string(),
            })),
        }
    }
    Ok(Json(json!({ "items": items })))
}

async fn get_cell(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id, cell_id)): Path<(String, String, String)>,
) -> ExperimentResult<Json<Value>> {
    let detail = state.catalog.get_cell_detail(&slug, &run_id, &cell_id)?;
    Ok(Json(json!({ "cell": detail })))
}

async fn get_cell_schedule(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id, cell_id)): Path<(String, String, String)>,
) -> ExperimentResult<Response> {
    let path = state
        .catalog
        .get_cell_schedule_path(&slug, &run_id, &cell_id)?;
    serve_file(path, "application/json").await
}

async fn get_cell_trace(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id, cell_id)): Path<(String, String, String)>,
) -> ExperimentResult<Response> {
    let path = state
        .catalog
        .get_cell_trace_path(&slug, &run_id, &cell_id)?;
    serve_file(path, "application/x-ndjson").await
}

async fn get_summary_csv(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
) -> ExperimentResult<Response> {
    let path = state.catalog.get_summary_csv_path(&slug, &run_id)?;
    serve_file(path, "text/csv").await
}

#[derive(Debug, Deserialize)]
struct ParetoQuery {
    x: String,
    y: String,
    #[serde(default = "default_true")]
    xmax: bool,
    #[serde(default)]
    ymax: bool,
}

fn default_true() -> bool {
    true
}

async fn get_pareto(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
    Query(q): Query<ParetoQuery>,
) -> ExperimentResult<Json<Value>> {
    let front = state
        .catalog
        .pareto_front(&slug, &run_id, &q.x, &q.y, q.xmax, q.ymax)?;
    Ok(Json(json!({
        "x_field": q.x,
        "y_field": q.y,
        "maximize_x": q.xmax,
        "maximize_y": q.ymax,
        "front": front,
    })))
}

#[derive(Debug, Deserialize)]
struct RankingQuery {
    #[serde(default = "default_rank_by")]
    by: String,
    #[serde(default)]
    completion: Option<f64>,
    #[serde(default)]
    priority: Option<f64>,
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    fragmentation: Option<f64>,
}

fn default_rank_by() -> String {
    "dataset".into()
}

async fn get_ranking(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
    Query(q): Query<RankingQuery>,
) -> ExperimentResult<Json<Value>> {
    let weights = if q.completion.is_some()
        || q.priority.is_some()
        || q.utilization.is_some()
        || q.fragmentation.is_some()
    {
        Some(RankingWeights {
            completion: q.completion.unwrap_or(1.0),
            priority: q.priority.unwrap_or(1.0),
            utilization: q.utilization.unwrap_or(1.0),
            fragmentation: q.fragmentation.unwrap_or(1.0),
        })
    } else {
        None
    };
    let ranking = match q.by.as_str() {
        "dataset" => state.catalog.dataset_ranking(&slug, &run_id, weights)?,
        "algorithm" => state.catalog.algorithm_ranking(&slug, &run_id, weights)?,
        other => {
            return Err(ExperimentError::BadRequest(format!(
                "unknown `by` value `{other}`; expected `dataset` or `algorithm`"
            )));
        }
    };
    Ok(Json(json!({
        "by": q.by,
        "weights": weights,
        "entries": ranking,
    })))
}

async fn stream_events(
    Extension(state): Extension<Arc<ExperimentsState>>,
    Path((slug, run_id)): Path<(String, String)>,
) -> ExperimentResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    // Validate run exists upfront so we can return 404 instead of an SSE
    // that only surfaces empty events.
    state.catalog.get_experiment_index(&slug, &run_id)?;
    let path = state.catalog.state_path(&slug, &run_id);

    let stream = async_stream::stream! {
        let mut last_seen = 0usize;
        // Replay loop: read whole file, yield new events, sleep, repeat.
        // The matrix is single-writer append-only, so re-reading byte 0
        // each tick is correct (and cheap for thousands of cells).
        loop {
            match state_events::read_events(&path) {
                Ok(events) => {
                    for ev in events.iter().skip(last_seen) {
                        let payload = serde_json::to_string(ev).unwrap_or_default();
                        let event = Event::default()
                            .event("state")
                            .data(payload);
                        yield Ok::<_, Infallible>(event);
                    }
                    last_seen = events.len();
                }
                Err(e) => {
                    let event = Event::default()
                        .event("error")
                        .data(format!("{e}"));
                    yield Ok::<_, Infallible>(event);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30))))
}

// ── helpers ────────────────────────────────────────────────────────────────

async fn serve_file(path: PathBuf, content_type: &'static str) -> ExperimentResult<Response> {
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(ExperimentError::Io)?;
    let stream = tokio_util_compat::ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(name) = path.file_name().and_then(|n| n.to_str())
        && let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{name}\""))
    {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok((StatusCode::OK, headers, body).into_response())
}

// We can't easily depend on tokio-util's ReaderStream without adding the
// dep; provide a tiny adapter here. (`tokio_util` IS pulled in
// transitively by axum, but its `io` feature isn't guaranteed to be on.)
mod tokio_util_compat {
    use axum::body::Bytes;
    use futures::Stream;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, ReadBuf};

    pub struct ReaderStream<R> {
        reader: R,
        buf: Vec<u8>,
        done: bool,
    }

    impl<R: AsyncRead + Unpin> ReaderStream<R> {
        pub fn new(reader: R) -> Self {
            Self {
                reader,
                buf: vec![0u8; 8192],
                done: false,
            }
        }
    }

    impl<R: AsyncRead + Unpin> Stream for ReaderStream<R> {
        type Item = std::io::Result<Bytes>;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.done {
                return Poll::Ready(None);
            }
            let mut tmp = std::mem::take(&mut self.buf);
            let result = {
                let mut read_buf = ReadBuf::new(&mut tmp);
                let pinned = Pin::new(&mut self.reader);
                pinned
                    .poll_read(cx, &mut read_buf)
                    .map(|r| r.map(|()| read_buf.filled().len()))
            };
            self.buf = tmp;
            match result {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(0)) => {
                    self.done = true;
                    Poll::Ready(None)
                }
                Poll::Ready(Ok(n)) => {
                    let chunk = Bytes::copy_from_slice(&self.buf[..n]);
                    Poll::Ready(Some(Ok(chunk)))
                }
                Poll::Ready(Err(e)) => {
                    self.done = true;
                    Poll::Ready(Some(Err(e)))
                }
            }
        }
    }
}
