use super::ScheduleFom;
use crate::schedule::{Schedule, SchedulingProblem};

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
    fn evaluate(&self, schedule: &Schedule, problem: &SchedulingProblem) -> f64 {
        self.primary.evaluate(schedule, problem) * 1.0e9 + self.secondary.evaluate(schedule, problem)
    }

    fn label(&self) -> &'static str {
        "composite"
    }
}
