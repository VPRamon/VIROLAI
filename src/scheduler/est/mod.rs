//! Earliest-Start-Time (EST) scheduler implementation.
//!
//! This module contains the beam-search EST variant used by the scheduler.
//! The code is split by concern so the decision rules stay explicit:
//! - [`context`]: problem-aware beam-search helpers.
//! - [`config`]: EST tunable parameters and bounds.
//! - [`algorithm`]: scheduler setup and entry points.
//! - [`beam`]: beam-search expansion and pruning loop.
//! - [`candidate`]: per-task EST metadata.
//! - [`ordering`]: candidate queue ordering rules.
//! - [`queue`]: refresh and queue maintenance.
//! - [`schedule_state`]: one live beam state.
//! - [`validation`]: configuration and task pre-flight checks.

mod algorithm;
mod beam;
mod candidate;
mod configuration;
mod context;
pub mod fom;
mod ordering;
mod queue;
mod schedule_state;
pub mod trace;
mod validation;

pub use algorithm::{EstScheduler, run_scheduler};
pub use candidate::{EstCandidate, IntoTaskPlacement};
pub use configuration::{Configuration, MAX_K_BEAMS};
pub use fom::{CompositeFom, EstFomKind, ScheduleFom, ScoringContext, SoftConstraintFom};
pub use ordering::{compare_candidates, sort_candidates};
pub use schedule_state::ScheduleState;
pub use trace::{EstTraceEvent, EstTraceSink, JsonlTraceSink, NoopTraceSink};

#[cfg(test)]
mod tests;
