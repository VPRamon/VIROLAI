//! Experiment definitions, matrix expansion, and execution.
//!
//! This module owns the concepts needed to turn a JSON sweep specification into
//! concrete scheduler runs, prepare input datasets, and execute cells.

pub mod cell;
pub mod config;
pub mod output;
pub mod problem;
pub mod runner;
pub mod spec;
pub mod state;

pub use cell::{MatrixCell, resolve_cells};
pub use config::{EstRunConfig, HapRunConfig, HapSurvivorMode, RunConfig};
pub use problem::{PreparedProblem, prepare_problem};
pub use runner::{RunOptions, RunSummary, execute};
pub use spec::ExperimentSpec;
