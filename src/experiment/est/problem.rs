use crate::prescheduler::{TaskPeriodMap, preschedule};
use crate::schedule::SchedulingProblem;
use crate::task::Task;
use crate::time::{MJD, Period, TaskId, Time};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::config::HorizonOverride;

/// A scheduling problem loaded from disk and preprocessed, shared across all runs.
///
/// The prescheduling step (computing feasible windows per task) is expensive and
/// independent of scheduler configuration, so it runs once here and is reused.
#[derive(Debug)]
pub(crate) struct PreparedProblem {
    /// Original JSON, preserved for embedding in schedule output files.
    pub(crate) raw_json: Value,
    pub(crate) tasks: Vec<Task>,
    pub(crate) possible_periods: TaskPeriodMap,
    pub(crate) horizon: Period<MJD>,
    pub(crate) priority_by_task: HashMap<TaskId, f64>,
}

/// Loads `input_path`, runs the prescheduler, and returns a [`PreparedProblem`].
pub(crate) fn prepare_problem(
    input_path: &Path,
    horizon_override: Option<HorizonOverride>,
) -> Result<PreparedProblem, String> {
    let text = fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;
    let raw_json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse JSON {}: {e}", input_path.display()))?;
    let problem: SchedulingProblem = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    if problem.tasks.is_empty() {
        return Err("input JSON contains no tasks".to_string());
    }

    let priority_by_task = extract_task_priorities(&raw_json);

    let SchedulingProblem {
        tasks,
        blocks,
        detected_horizon,
        telescope,
    } = problem;

    let horizon = build_horizon(detected_horizon, horizon_override)?;
    let telescope = telescope.ok_or_else(|| {
        "missing observing site in input; expected resources[0] or legacy location".to_string()
    })?;
    let blocks: Vec<_> = blocks.into_values().collect();

    let possible_periods = preschedule(&blocks, &tasks, &horizon, &telescope)
        .map_err(|e| format!("prescheduling failed: {e}"))?;

    let mut tasks: Vec<_> = tasks.into_values().collect();
    tasks.sort_by_key(|task| task.id.0);

    Ok(PreparedProblem {
        raw_json,
        tasks,
        possible_periods,
        horizon,
        priority_by_task,
    })
}

fn build_horizon(
    detected: Option<Period<MJD>>,
    override_range: Option<HorizonOverride>,
) -> Result<Period<MJD>, String> {
    if let Some(override_range) = override_range {
        return period_from_mjd(override_range.start_mjd, override_range.end_mjd);
    }
    detected.ok_or_else(|| {
        "missing schedule_time_window in input and no horizon override was provided".to_string()
    })
}

fn period_from_mjd(start_mjd: f64, end_mjd: f64) -> Result<Period<MJD>, String> {
    if !start_mjd.is_finite() || !end_mjd.is_finite() {
        return Err("horizon bounds must be finite numbers".to_string());
    }
    if start_mjd >= end_mjd {
        return Err(format!(
            "invalid horizon: start ({start_mjd}) must be before end ({end_mjd})"
        ));
    }
    Ok(Period::new(
        Time::<MJD>::new(start_mjd),
        Time::<MJD>::new(end_mjd),
    ))
}

/// Extracts `soft_constraints.priority` for every task in the input JSON.
fn extract_task_priorities(json: &Value) -> HashMap<TaskId, f64> {
    let mut priorities = HashMap::new();
    if let Some(blocks) = json.get("scheduling_blocks").and_then(Value::as_array) {
        for block in blocks {
            collect_block_priorities(block, &mut priorities);
        }
    } else if let Some(blocks) = json.as_array() {
        for block in blocks {
            collect_block_priorities(block, &mut priorities);
        }
    }
    priorities
}

fn collect_block_priorities(block: &Value, priorities: &mut HashMap<TaskId, f64>) {
    let Some(tasks) = block.get("tasks").and_then(Value::as_array) else {
        return;
    };
    for task in tasks {
        let Some(id) = task.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let priority = task
            .get("soft_constraints")
            .and_then(|soft| soft.get("priority"))
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        priorities.insert(TaskId(id), priority);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_task_priorities_defaults_missing_priority_to_zero() {
        let json: Value = serde_json::from_str(
            r#"{
                "scheduling_blocks": [
                    {
                        "tasks": [
                            {
                                "id": 1,
                                "soft_constraints": {
                                    "priority": 10.0
                                }
                            },
                            {
                                "id": 2,
                                "soft_constraints": {}
                            }
                        ]
                    }
                ]
            }"#,
        )
        .expect("fixture should parse");

        let priorities = extract_task_priorities(&json);

        assert_eq!(priorities.get(&TaskId(1)), Some(&10.0));
        assert_eq!(priorities.get(&TaskId(2)), Some(&0.0));
    }
}
