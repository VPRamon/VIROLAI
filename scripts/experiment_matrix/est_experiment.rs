//! Re-export of the `est_experiment` binary's config and problem modules,
//! shared by the new `experiment_matrix` runner so there is a single
//! source of truth for sweep axes and dataset preparation.

#[allow(dead_code)]
#[path = "../est_experiment/config.rs"]
pub mod config;

#[allow(dead_code)]
#[path = "../est_experiment/problem.rs"]
pub mod problem;
