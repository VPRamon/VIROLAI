//! Earliest-Start-Time (EST) scheduler implementation.
//!
//! `EstScheduler` is a thin wrapper that preconfigures the shared cursor
//! engine as a single forward cursor over the whole horizon. The actual
//! beam-search execution lives in the shared cursor-engine module.
//!
//! The sub-modules in this crate are shared utilities consumed by the cursor
//! engine:
//! - `context`: problem-aware dependency helpers (`check_block_dependencies`)
//! - `configuration`: EST tunable parameters
//! - `algorithm`: scheduler public entry points
//! - `candidate`: per-task EST metadata
//! - `ordering`: candidate queue ordering rules
//! - `queue`: test-only queue maintenance helpers

mod algorithm;
mod candidate;
mod configuration;
mod context;
mod ordering;
#[cfg(test)]
mod queue;

pub use crate::scheduler::fom::{CompositeFom, FomKind, ScheduleFom, SoftConstraintFom};
pub use algorithm::{EstScheduler, run_scheduler};
pub use candidate::{Candidate, IntoTaskPlacement};
pub use configuration::Configuration;
pub use ordering::{compare_candidates, sort_candidates};

pub(crate) use context::check_block_dependencies;

#[cfg(test)]
mod tests;
