use crate::schedule::Schedule;
use crate::time::MJD;
use super::{ScheduleFom, ScoringContext};

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

    fn label(&self) -> &'static str {
        "soft_constraint"
    }
}
