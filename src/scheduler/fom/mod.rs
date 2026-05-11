pub mod composite;
pub mod kind;
pub mod soft_constraint;

pub use self::composite::CompositeFom;
pub use self::kind::{EstFomKind, FomKind};
pub use self::soft_constraint::SoftConstraintFom;

use crate::schedule::Schedule;
use crate::schedule::SchedulingProblem;
use std::sync::Arc;

/// Scores a schedule state. Higher values indicate better schedules.
///
/// The beam search keeps the K states with the *highest* FOM after each
/// expansion round.
pub trait ScheduleFom: std::fmt::Debug + Send + Sync {
    /// Return the scalar score used to rank one schedule state against another.
    fn evaluate(&self, schedule: &Schedule, problem: &SchedulingProblem) -> f64;
    /// Return a human-readable label for this FOM.
    fn label(&self) -> &'static str;
}

impl<T: ScheduleFom + ?Sized> ScheduleFom for Arc<T> {
    fn evaluate(&self, schedule: &Schedule, problem: &SchedulingProblem) -> f64 {
        (**self).evaluate(schedule, problem)
    }

    fn label(&self) -> &'static str {
        (**self).label()
    }
}
