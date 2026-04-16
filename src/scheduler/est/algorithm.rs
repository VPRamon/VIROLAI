use super::candidate::IntoTaskPlacement;
use super::queue::CandidateQueue;
use super::validation;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::task::Task;
use crate::time::{MJD, Period};

/// EST configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstConfig {
    /// Candidate class split between endangered and flexible.
    pub endangered_threshold: u32,
}

impl Default for EstConfig {
    fn default() -> Self {
        Self {
            endangered_threshold: 1,
        }
    }
}

/// EST scheduler implementation.
#[derive(Debug, Clone, Default)]
pub struct EstScheduler {
    pub config: EstConfig,
}

impl EstScheduler {
    pub fn new(config: EstConfig) -> Result<Self, ScheduleError> {
        let scheduler = Self { config };
        validation::validate_scheduler(&scheduler)?;
        Ok(scheduler)
    }

    /// Run EST on `task_ids` using the provided feasible windows.
    pub fn run_scheduler(
        &self,
        tasks: Vec<Task>,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — tasks={}, endangered_threshold={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.endangered_threshold,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_tasks(&tasks)?;
        let tasks = validation::filter_tasks(tasks, possible_periods);

        log::debug!("est: {} tasks remain after feasibility filter", tasks.len());

        let mut schedule = Schedule::new();

        let mut candidates = CandidateQueue::build(
            &tasks,
            possible_periods,
            horizon,
            self.config.endangered_threshold,
        );
        let mut cursor = horizon.start;
        let mut iteration: u32 = 0;

        while cursor < horizon.end {
            candidates.refresh(
                &Period::new(cursor, horizon.end),
                self.config.endangered_threshold,
            );
            let Some(candidate) = candidates.pop_next() else {
                log::warn!("est: no schedulable candidates remain at cursor={:.4}", cursor.value());
                break;
            };

            let task_id = candidate.task_id();
            let placement = candidate.into_task_placement(horizon.end);

            log::debug!(
                "est: iteration={} placed task={} at [{:.4}, {:.4}]",
                iteration,
                task_id.0,
                placement.start.value(),
                placement.end.value(),
            );

            cursor = placement.end.to::<MJD>();
            schedule.insert_placement(placement);
            iteration += 1;
        }

        log::info!(
            "est: done — scheduled {} task(s) in {} iteration(s)",
            schedule.placements.len(),
            iteration,
        );

        Ok(schedule)
    }
}

pub fn run_scheduler(
    tasks: Vec<Task>,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    EstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}
