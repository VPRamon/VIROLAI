//! `lab`: experiment runner and registry for VIROLAI.
//!
//! This crate provides parameter-sweep execution against the `schedulers`
//! library and persists results in the SQLite registry.
//!
//! # Crate structure
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`experiment`] | Sweep specs, run configs, matrix cells, dataset preparation, and execution |
//! | [`registry`] | SQLite run registry, identity hashing, queries, sorting, and stored rows |
//!
//! Compatibility aliases such as [`cell`], [`config`], and [`runner`] remain
//! exported at the crate root so existing programmatic users do not need to
//! change imports.
//!
//! # Usage
//!
//! ```text
//! lab run --spec <experiment.json> [--run-db <path>] [--override]
//! ```
//!
//! The user-facing `virolai sweep` command delegates to this runner.

pub mod experiment;
pub mod registry;

pub use experiment::{cell, config, output, problem, runner, spec, state};

pub use experiment::{
    EstRunConfig, ExperimentSpec, HapRunConfig, HapSurvivorMode, MatrixCell, PreparedProblem,
    RunConfig, RunOptions, RunSummary, execute, prepare_problem, resolve_cells,
};
