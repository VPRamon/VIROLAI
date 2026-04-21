use crate::schedule::{Schedule, ScheduleOutput};
use crate::time::TaskId;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::EstRunConfig;
use super::problem::PreparedProblem;
use super::stats::percentile;

/// The result of a single scheduler run.
#[derive(Debug, Clone)]
pub(crate) struct RunOutcome {
    pub(crate) config: EstRunConfig,
    pub(crate) schedule_path: PathBuf,
    pub(crate) metrics: RunMetrics,
}

/// Per-run performance metrics derived from the produced schedule.
#[derive(Debug, Clone)]
pub(crate) struct RunMetrics {
    pub(crate) scheduled_task_count: usize,
    pub(crate) fitness_priority_sum: f64,
    pub(crate) scheduled_priority_p25: f64,
    pub(crate) scheduled_priority_p50: f64,
    pub(crate) scheduled_priority_p75: f64,
    pub(crate) scheduled_priority_p90: f64,
}

/// Runs the scheduler, writes the schedule JSON to `schedule_path`, and computes metrics.
pub(crate) fn execute_run(
    run: &EstRunConfig,
    prepared_problem: &PreparedProblem,
    schedule_path: &Path,
) -> Result<RunOutcome, String> {
    let scheduler = run.build_scheduler()?;
    let schedule = scheduler
        .run_scheduler(
            &prepared_problem.tasks,
            &prepared_problem.possible_periods,
            &prepared_problem.horizon,
        )
        .map_err(|e| format!("EST run {} failed: {e}", run.slug()))?;

    let output = ScheduleOutput::new(prepared_problem.raw_json.clone(), &schedule);
    let output_text = serde_json::to_string_pretty(&output)
        .map_err(|e| format!("failed to serialize schedule output {}: {e}", run.slug()))?;
    fs::write(schedule_path, output_text).map_err(|e| {
        format!(
            "failed to write schedule output {}: {e}",
            schedule_path.display()
        )
    })?;

    let metrics = compute_run_metrics(&schedule, &prepared_problem.priority_by_task);

    Ok(RunOutcome {
        config: *run,
        schedule_path: schedule_path.to_path_buf(),
        metrics,
    })
}

pub(crate) fn compute_run_metrics(
    schedule: &Schedule,
    priority_by_task: &HashMap<TaskId, f64>,
) -> RunMetrics {
    let scheduled_priorities: Vec<f64> = schedule
        .placements
        .keys()
        .map(|task_id| *priority_by_task.get(task_id).unwrap_or(&0.0))
        .collect();

    let fitness_priority_sum: f64 = scheduled_priorities.iter().sum();

    RunMetrics {
        scheduled_task_count: schedule.placements.len(),
        fitness_priority_sum,
        scheduled_priority_p25: percentile(&scheduled_priorities, 0.25),
        scheduled_priority_p50: percentile(&scheduled_priorities, 0.50),
        scheduled_priority_p75: percentile(&scheduled_priorities, 0.75),
        scheduled_priority_p90: percentile(&scheduled_priorities, 0.90),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::TaskPlacement;
    use crate::time::{JD, MJD, TaskId, Time};

    fn schedule_with_slots(slots: &[(u64, f64, f64)]) -> Schedule {
        let mut schedule = Schedule::new();
        for (task_id, start, end) in slots {
            schedule.insert_placement(TaskPlacement {
                task_id: TaskId(*task_id),
                start: Time::<MJD>::new(*start).to::<JD>(),
                end: Time::<MJD>::new(*end).to::<JD>(),
                block_id: None,
            });
        }
        schedule
    }

    #[test]
    fn compute_run_metrics_reports_compact_priority_stats() {
        let schedule =
            schedule_with_slots(&[(1, 0.0, 0.1), (2, 0.2, 0.3), (3, 0.5, 0.6), (4, 0.7, 0.8)]);
        let priorities = HashMap::from([
            (TaskId(1), 10.0),
            (TaskId(2), 20.0),
            (TaskId(3), 30.0),
            (TaskId(4), 40.0),
        ]);

        let metrics = compute_run_metrics(&schedule, &priorities);

        assert_eq!(metrics.scheduled_task_count, 4);
        assert!((metrics.fitness_priority_sum - 100.0).abs() < 1e-9);
        assert!((metrics.scheduled_priority_p25 - 17.5).abs() < 1e-9);
        assert!((metrics.scheduled_priority_p50 - 25.0).abs() < 1e-9);
        assert!((metrics.scheduled_priority_p75 - 32.5).abs() < 1e-9);
        assert!((metrics.scheduled_priority_p90 - 37.0).abs() < 1e-9);
    }

    #[test]
    fn compute_run_metrics_defaults_missing_priorities_to_zero() {
        let schedule = schedule_with_slots(&[(10, 0.0, 0.1), (11, 0.2, 0.3)]);
        let priorities = HashMap::from([(TaskId(10), 10.0)]);

        let metrics = compute_run_metrics(&schedule, &priorities);

        assert_eq!(metrics.scheduled_task_count, 2);
        assert!((metrics.fitness_priority_sum - 10.0).abs() < 1e-9);
        assert!((metrics.scheduled_priority_p50 - 5.0).abs() < 1e-9);
    }
}
