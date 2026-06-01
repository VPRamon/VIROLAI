//! `lab` — experiment runner for the PhD scheduling workspace.
//!
//! This crate provides the tooling to run parameter-sweep experiments
//! against the `schedulers` library crate. It is intentionally
//! decoupled from the main crate so it can evolve and be tested
//! independently.
//!
//! # Crate structure
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`experiment`] | Sweep specs, run configs, matrix cells, dataset preparation, and execution |
//! | [`registry`] | SQLite run registry, identity hashing, query options, sort helpers, and rows |
//!
//! Compatibility aliases such as [`cell`], [`config`], and [`runner`] remain
//! exported at the crate root so existing programmatic users do not need to
//! change imports.
//!
//! # Usage
//!
//! The `lab` binary exposes a single sub-command:
//!
//! ```text
//! lab run --spec <experiment.json> [--run-db <path>] [--override]
//! ```
//!
//! It is also wired as the target of `phd sweep` in the `phd` unified CLI.

pub mod experiment;
pub mod registry;

pub use experiment::{cell, config, output, problem, runner, spec, state};

// Re-export the most commonly used public types at the crate root for
// convenience when this library is used programmatically.
pub use experiment::{
    EstRunConfig, ExperimentSpec, HapRunConfig, HapSurvivorMode, MatrixCell, PreparedProblem,
    RunConfig, RunOptions, RunSummary, execute, prepare_problem, resolve_cells,
};
