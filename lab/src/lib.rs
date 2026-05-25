//! `lab` — experiment runner for the PhD scheduling workspace.
//!
//! This crate provides the tooling to run parameter-sweep experiments
//! against the [`scheduler`] library crate.  It is intentionally
//! decoupled from the main crate so it can evolve and be tested
//! independently.
//!
//! # Crate structure
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`config`] | Immutable run-configuration types (`RunConfig`, `EstRunConfig`, `HapRunConfig`) and sweep-axis descriptors |
//! | [`spec`] | `ExperimentSpec` JSON format for the matrix runner |
//! | [`cell`] | `MatrixCell` and `resolve_cells` — Cartesian-product expansion |
//! | [`problem`] | `PreparedProblem` — dataset loading and prescheduling |
//! | [`runner`] | `execute` — parallel matrix runner with checkpointing; embeds metrics into each schedule |
//! | [`output`] | Run-directory layout helpers |
//! | [`state`] | `state.jsonl` append-only checkpoint stream |
//!
//! # Usage
//!
//! The `lab` binary exposes a single sub-command:
//!
//! ```text
//! lab run --spec <experiment.json>
//!                 [--resume <existing_run_dir>]
//!                 [--output-dir <dir>]
//!                 [--dry-run] [--no-state]
//! ```
//!
//! It is also wired as the target of `phd matrix` and `phd sweep` in
//! the `phd` unified CLI.

pub mod cell;
pub mod config;
pub mod output;
pub mod problem;
pub mod registry;
pub mod runner;
pub mod spec;
pub mod state;

// Re-export the most commonly used public types at the crate root for
// convenience when this library is used programmatically.
pub use cell::{MatrixCell, resolve_cells};
pub use config::{EstRunConfig, HapRunConfig, HapSurvivorMode, RunConfig};
pub use problem::{PreparedProblem, prepare_problem};
pub use runner::{RunOptions, RunSummary, execute, execute_with_options};
pub use spec::ExperimentSpec;
