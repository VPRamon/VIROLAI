//! Figure of Merit (FOM) for ranking beam search schedule states.
//!
//! Implement [`ScheduleFom`] to supply a custom scoring function. Built-in
//! implementations cover the two most common cases:
//! - [`TaskCountFom`]: maximise the number of placed tasks.
//! - [`SoftConstraintFom`]: maximise the sum of soft-constraint scores.
//! - [`CompositeFom`]: lexicographic combination of two FOMs.

use crate::schedule::Schedule;
use crate::task::Task;
use crate::time::MJD;
use std::collections::HashMap;

/// Scores a schedule state. Higher values indicate better schedules.
///
/// The beam search keeps the K states with the *highest* FOM after each
/// expansion round.
pub trait ScheduleFom: std::fmt::Debug + Send + Sync {
    fn evaluate(&self, schedule: &Schedule, tasks: &[Task]) -> f64;
}

/// FOM: number of scheduled tasks (maximize).
#[derive(Debug, Clone, Default)]
pub struct TaskCountFom;

impl ScheduleFom for TaskCountFom {
    fn evaluate(&self, schedule: &Schedule, _tasks: &[Task]) -> f64 {
        schedule.placements.len() as f64
    }
}

/// FOM: sum of soft-constraint scores of all placed tasks (maximize).
///
/// Each placed task is evaluated at its scheduled start time. Tasks without
/// soft constraints contribute 0.0.
#[derive(Debug, Clone, Default)]
pub struct SoftConstraintFom;

impl ScheduleFom for SoftConstraintFom {
    fn evaluate(&self, schedule: &Schedule, tasks: &[Task]) -> f64 {
        let task_map: HashMap<_, _> = tasks.iter().map(|t| (t.id, t)).collect();
        schedule
            .placements
            .iter()
            .map(|(id, placement)| {
                let Some(task) = task_map.get(id) else {
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
    pub fn new(primary: Box<dyn ScheduleFom>, secondary: Box<dyn ScheduleFom>) -> Self {
        Self { primary, secondary }
    }
}

impl ScheduleFom for CompositeFom {
    fn evaluate(&self, schedule: &Schedule, tasks: &[Task]) -> f64 {
        self.primary.evaluate(schedule, tasks) * 1.0e9
            + self.secondary.evaluate(schedule, tasks)
    }
}
