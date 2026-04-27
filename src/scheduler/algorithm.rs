use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Period};

/// Common interface for scheduling algorithms that operate on a pre-scheduled problem.
pub trait SchedulingAlgorithm {
    fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError>;
}
