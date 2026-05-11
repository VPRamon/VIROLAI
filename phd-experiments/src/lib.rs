//! `phd-experiments` — experiment runner for the PhD scheduling workspace.
//!
//! This crate provides the tooling to run parameter-sweep experiments against
//! the [`scheduler`] library crate.  It is deliberately decoupled from the
//! main crate so it can be evolved, tested, and versioned independently.
//!
//! # Crate structure
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`config`] | Immutable run-configuration types (`RunConfig`, `EstRunConfig`, `HapRunConfig`) and sweep-axis descriptors |
//! | [`spec`] | `ExperimentSpec` JSON format for the matrix runner |
//! | [`cell`] | `MatrixCell` and `resolve_cells` — Cartesian-product expansion |
//! | [`problem`] | `PreparedProblem` — dataset loading and prescheduling |
//! | [`run`] | `execute_run` — single-cell scheduler invocation |
//! | [`runner`] | `execute` — parallel matrix runner with checkpointing |
//! | [`output`] | Run-directory layout helpers |
//! | [`state`] | `state.jsonl` append-only checkpoint stream |
//! | [`migrate`] | Legacy run-directory migration |
//!
//! # Usage
//!
//! The `phd-experiments` binary exposes a `clap`-based CLI with the following
//! sub-commands:
//!
//! ```text
//! phd-experiments run    --spec <experiment.json>
//! phd-experiments run    --spec <experiment.json> --resume <existing_run_dir>
//! phd-experiments run    --spec <experiment.json> --dry-run
//! phd-experiments migrate <old_run_dir> [--output <new_dir>]
//! ```
//!
//! It is also wired as the target of `phd matrix` and `phd sweep` in the
//! `phd` unified CLI.

pub mod cell;
pub mod config;
pub mod migrate;
pub mod output;
pub mod problem;
pub mod run;
pub mod runner;
pub mod spec;
pub mod state;

// Re-export the most commonly used public types at the crate root for
// convenience when this library is used programmatically.
pub use cell::{MatrixCell, resolve_cells};
pub use config::{EstRunConfig, HapRunConfig, HapSurvivorMode, RunConfig};
pub use problem::{PreparedProblem, prepare_problem};
pub use runner::{RunSummary, execute};
pub use spec::ExperimentSpec;
