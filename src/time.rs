//! Core domain time types used throughout the scheduler.
//!
//! `tempoch` is the canonical time backend for this crate and is re-exported
//! here so callers can consistently import time primitives from a single
//! module.

pub use tempoch;
pub use tempoch::{JD, MJD, Period, Time, TimeScale};

/// A point in time expressed as Julian Date.
pub type TimePoint = Time<JD>;

/// A half-open interval `[start, end)` in Julian Date.
pub type TimeInterval = Period<JD>;

/// Opaque identifier for a [`Task`](crate::task::Task).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);

/// Opaque identifier for a [`SchedulingBlock`](crate::scheduling_block::SchedulingBlock).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchedulingBlockId(pub u64);
