//! Earliest-Start-Time (EST) scheduler implementation.
//!
//! This module contains the beam-search EST variant used by the scheduler.
//! The code is split by concern so the decision rules stay explicit:
//! - [`config`]: EST tunable parameters and bounds.
//! - [`algorithm`]: outer search loop.
//! - [`candidate`]: per-task EST metadata.
//! - [`ordering`]: candidate queue ordering rules.
//! - [`queue`]: refresh and queue maintenance.
//! - [`schedule_state`]: one live beam state.
//! - [`validation`]: configuration and task pre-flight checks.

mod algorithm;
mod candidate;
mod config;
pub mod fom;
mod ordering;
mod queue;
mod schedule_state;
mod validation;

pub use algorithm::{EstScheduler, run_scheduler};
pub use candidate::{EstCandidate, IntoTaskPlacement};
pub use config::{EstConfig, MAX_K_BEAMS};
pub use fom::{CompositeFom, EstFomKind, ScheduleFom, SoftConstraintFom, TaskCountFom};
pub use ordering::{compare_candidates, sort_candidates};
pub use schedule_state::ScheduleState;

#[cfg(test)]
mod tests;
