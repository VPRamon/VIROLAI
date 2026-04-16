//! Error types for the scheduler.

use thiserror::Error;

/// All errors that can arise from scheduling operations.
#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("task not found")]
    TaskNotFound,

    #[error("scheduling block not found")]
    BlockNotFound,

    #[error("duration must be positive")]
    InvalidDuration,

    #[error("invalid task: {0}")]
    InvalidTask(String),

    #[error("placement interval length does not match task duration")]
    IntervalDurationMismatch,

    #[error("placement overlaps with an existing task in the schedule")]
    OverlapConflict,

    #[error("dependency graph contains a cycle")]
    DependencyCycle,

    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("constraint violated: {0}")]
    ConstraintViolation(String),
}
