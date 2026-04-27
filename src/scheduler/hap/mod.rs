//! HAP / AP — Accumulative Planner family.
//!
//! Two top-level entry points configured around a single accumulative core:
//!
//! - [`ap::run`] — deterministic greedy, single output schedule.
//! - [`hap::run`] — stochastic multi-start, set of output schedules.
//!
//! Both reuse the [`cru`] module (Conflict Resolution Unit + variants) for
//! per-block candidate generation and the shared [`eval`] / [`selection`]
//! helpers for fitness and survivor selection.

pub mod accumulative;
pub mod ap;
pub mod configuration;
pub mod cru;
pub mod eval;
#[allow(clippy::module_inception)]
pub mod hap;
pub mod selection;

pub use configuration::{Configuration, PlannerConfig, Selector, SurvivorSelector};
