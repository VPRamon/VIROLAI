//! Figure of Merit (FOM) for ranking beam search schedule states.
//!
//! Implement [`ScheduleFom`] to supply a custom scoring function. Built-in
//! implementations cover the two most common cases:
//! - [`SoftConstraintFom`]: maximise the sum of soft-constraint scores.
//! - [`CompositeFom`]: lexicographic combination of two FOMs.

use crate::schedule::Schedule;
use crate::schedule::SchedulingProblem;
use crate::task::Task;
use crate::time::{MJD, TaskId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

/// Prepared scoring context for FOM evaluation.
///
/// Build once per scheduler run to avoid rebuilding the task lookup map on
/// every call.
pub struct ScoringContext<'a> {
    problem: &'a SchedulingProblem,
}

impl<'a> ScoringContext<'a> {
    pub fn new(problem: &'a SchedulingProblem) -> Self {
        Self { problem }
    }

    pub fn tasks(&self) -> Vec<&Task> {
        self.problem.iter_tasks().collect()
    }

    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.problem.task(id)
    }
}

/// Scores a schedule state. Higher values indicate better schedules.
///
/// The beam search keeps the K states with the *highest* FOM after each
/// expansion round.
pub trait ScheduleFom: std::fmt::Debug + Send + Sync {
    /// Return the scalar score used to rank one schedule state against another.
    fn evaluate(&self, schedule: &Schedule, ctx: &ScoringContext) -> f64;
    /// Return a human-readable label for this FOM.
    fn label(&self) -> &'static str;
}

/// User-facing EST figure-of-merit selector.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum EstFomKind {
    #[default]
    SoftConstraint,
}

impl EstFomKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SoftConstraint => "soft_constraint",
        }
    }

    pub fn into_fom(self) -> Arc<dyn ScheduleFom> {
        match self {
            Self::SoftConstraint => Arc::new(crate::scheduler::fom::SoftConstraintFom),
        }
    }
}

impl fmt::Display for EstFomKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EstFomKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "soft_constraint" => Ok(Self::SoftConstraint),
            other => Err(format!(
                "invalid EST FOM '{other}' (expected 'soft_constraint')"
            )),
        }
    }
}
