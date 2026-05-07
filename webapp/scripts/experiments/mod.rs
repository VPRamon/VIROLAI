//! Experiments backend domain.
//!
//! Filesystem-backed catalog + child-process orchestrator + REST/SSE
//! endpoints for the `experiment_matrix` workflow. Designed to be mounted
//! into the TSI HTTP backend via [`tsi_rust::http::BackendExtensions`].
//!
//! Module layout:
//!
//! - [`state_events`]: minimal mirror of `state.jsonl` event shapes (the
//!   matrix runner is a separate binary so we can't depend on its types).
//! - [`catalog`]: in-memory index over `<root>/<slug>/run-*/` directories
//!   plus on-demand readers for metrics/schedules/traces and aggregations.
//! - [`orchestrator`]: spawns the `experiment_matrix` child process and
//!   tracks its lifecycle.
//! - [`errors`]: HTTP-friendly error type.
//! - [`routes`]: axum router exposing `/v1/experiments/...` endpoints.

pub mod catalog;
pub mod errors;
pub mod orchestrator;
pub mod routes;
pub mod state_events;

use std::sync::Arc;

pub use catalog::Catalog;
pub use orchestrator::ExperimentRunner;
#[allow(unused_imports)]
pub use orchestrator::RunHandleStatus;
pub use routes::{ExperimentsState, experiments_router};

/// Initialize a shared experiments state from the environment.
pub fn state_from_env() -> std::io::Result<Arc<ExperimentsState>> {
    let root = std::env::var("PHD_EXPERIMENTS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("./experiments"));
    std::fs::create_dir_all(&root)?;

    let max_concurrent = std::env::var("PHD_EXPERIMENTS_MAX_CONCURRENT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.max(1))
        .unwrap_or(1);

    let bin = std::env::var("PHD_EXPERIMENT_MATRIX_BIN").ok();

    let catalog = Arc::new(Catalog::new(root.clone()));
    let runner = Arc::new(ExperimentRunner::new(
        root.clone(),
        bin,
        max_concurrent,
        catalog.clone(),
    ));

    Ok(Arc::new(ExperimentsState {
        root,
        catalog,
        runner,
    }))
}

#[cfg(test)]
mod tests;
