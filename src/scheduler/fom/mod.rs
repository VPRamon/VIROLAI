pub mod composite;
pub mod est_kind;
pub mod soft_constraint;

pub use self::composite::CompositeFom;
pub use self::est_kind::EstFomKind;
pub use self::soft_constraint::SoftConstraintFom;

use crate::schedule::Schedule;
use crate::schedule::SchedulingProblem;
use crate::task::Task;
use crate::time::TaskId;

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
