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
            Self::SoftConstraint => Arc::new(SoftConstraintFom),
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

/// FOM: sum of soft-constraint scores of all placed tasks (maximize).
///
/// Each placed task is evaluated at its scheduled start time. Tasks without
/// soft constraints contribute 0.0.
#[derive(Debug, Clone, Default)]
pub struct SoftConstraintFom;

impl ScheduleFom for SoftConstraintFom {
    /// Score by summing the soft-constraint score of every placed task.
    fn evaluate(&self, schedule: &Schedule, ctx: &ScoringContext) -> f64 {
        schedule
            .placements()
            .map(|placement| {
                let Some(task) = ctx.task(placement.task_id) else {
                    // Missing task metadata should not panic during ranking; it
                    // simply contributes no additional soft score.
                    return 0.0;
                };
                let start = placement.start.to::<MJD>();
                task.soft_constraints
                    .as_ref()
                    .map(|expr| expr.score(&start, None, Some(&task.target)))
                    .unwrap_or(0.0)
            })
            .sum()
    }
}

/// FOM: lexicographic combination of two FOMs.
///
/// `score = primary * 1e9 + secondary`. The primary always dominates; the
/// secondary breaks ties. This is valid as long as secondary scores are
/// bounded well below 1e9.
#[derive(Debug)]
pub struct CompositeFom {
    pub primary: Box<dyn ScheduleFom>,
    pub secondary: Box<dyn ScheduleFom>,
}

impl CompositeFom {
    /// Combine two FOMs into one lexicographic score.
    pub fn new(primary: Box<dyn ScheduleFom>, secondary: Box<dyn ScheduleFom>) -> Self {
        Self { primary, secondary }
    }
}

impl ScheduleFom for CompositeFom {
    /// Score first by `primary`, then by `secondary` as a tie-breaker.
    fn evaluate(&self, schedule: &Schedule, ctx: &ScoringContext) -> f64 {
        self.primary.evaluate(schedule, ctx) * 1.0e9 + self.secondary.evaluate(schedule, ctx)
    }
}
