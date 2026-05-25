//! Earliest-Start-Time (EST) scheduler implementation.
//!
//! This module contains the beam-search EST variant used by the scheduler.
//! The code is split by concern so the decision rules stay explicit:
//! - [`context`]: problem-aware beam-search helpers.
//! - [`configuration`]: EST tunable parameters.
//! - [`algorithm`]: scheduler setup and entry points.
//! - [`beam`]: beam-search expansion and pruning loop.
//! - [`candidate`]: per-task EST metadata.
//! - [`ordering`]: candidate queue ordering rules.
//! - [`queue`]: refresh and queue maintenance.
//! - [`schedule_state`]: one live beam state.

mod algorithm;
mod beam;
mod candidate;
mod configuration;
mod context;
mod ordering;
mod queue;
mod schedule_state;

pub use crate::scheduler::fom::{CompositeFom, FomKind, ScheduleFom, SoftConstraintFom};
pub use algorithm::{EstScheduler, run_scheduler};
pub use candidate::{Candidate, IntoTaskPlacement};
pub use configuration::Configuration;
pub use ordering::{compare_candidates, sort_candidates};
pub use schedule_state::ScheduleState;

#[cfg(test)]
mod tests;
