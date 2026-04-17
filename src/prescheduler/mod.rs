//! Pre-scheduling pass that computes per-task feasibility windows.
//!
//! Telescope-level hard constraints (e.g. night-time, Moon-below-horizon) are
//! evaluated **once** over the scheduling horizon. The resulting feasibility
//! set is then intersected with each task's own hard constraints (time
//! window, altitude, azimuth, …) to produce a final per-task window set.
//!
//! Tasks frequently share the same dynamic constraint shape and pointing
//! target, so dynamic results are memoized on a `(signature, target)` key.

use crate::error::ScheduleError;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::telescope::Telescope;
use crate::time::{MJD, Period, PeriodSet, TaskId};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

pub type TaskPeriodMap = HashMap<TaskId, PeriodSet<MJD>>;

const PARALLEL_TASKS_PER_WORKER_THRESHOLD: usize = 16;

#[derive(Debug, Default)]
pub struct Prescheduler;

fn should_parallelize(task_count: usize) -> bool {
    let workers = rayon::current_num_threads();
    task_count >= workers.saturating_mul(PARALLEL_TASKS_PER_WORKER_THRESHOLD)
}

fn quantize_degrees(value: f64) -> i64 {
    (value * 1_000_000.0).round() as i64
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DynamicKey {
    signature: String,
    target_azimuth_microdeg: i64,
    target_polar_microdeg: i64,
}

impl DynamicKey {
    fn from_task(task: &Task) -> Self {
        Self {
            signature: format!("{:?}", task.hard_constraints.hard_dynamic),
            target_azimuth_microdeg: quantize_degrees(task.target.azimuth.value()),
            target_polar_microdeg: quantize_degrees(task.target.polar.value()),
        }
    }
}

/// Evaluate a task's dynamic constraints within a candidate set of windows,
/// memoizing results for identical `(dynamic-signature, target)` pairs.
fn dynamic_feasible(
    task: &Task,
    telescope: &Telescope,
    candidate_windows: &PeriodSet<MJD>,
    cache: &Mutex<HashMap<(DynamicKey, PeriodSetKey), PeriodSet<MJD>>>,
) -> PeriodSet<MJD> {
    if task.hard_constraints.hard_dynamic.is_unconstrained() {
        return candidate_windows.clone();
    }

    let key = (
        DynamicKey::from_task(task),
        PeriodSetKey::from(candidate_windows),
    );
    if let Some(cached) = cache.lock().unwrap().get(&key) {
        return cached.clone();
    }

    let mut out = PeriodSet::new();
    for window in candidate_windows.iter() {
        let dynamic = task.hard_constraints.hard_dynamic.check(
            window,
            Some(&telescope.location),
            Some(&task.target),
        );
        out = out.union(&dynamic);
    }

    cache.lock().unwrap().insert(key, out.clone());
    out
}

/// Hashable fingerprint of a `PeriodSet` (treated by start/end value pairs).
///
/// Tasks with different static constraints project onto different candidate
/// window sets, so caching by the candidate set ensures correctness.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PeriodSetKey(Vec<(i64, i64)>);

impl PeriodSetKey {
    fn from(set: &PeriodSet<MJD>) -> Self {
        let mut bounds = Vec::with_capacity(set.as_slice().len());
        for period in set.iter() {
            bounds.push((
                period.start.value().to_bits() as i64,
                period.end.value().to_bits() as i64,
            ));
        }
        Self(bounds)
    }
}

impl Prescheduler {
    /// Compute feasible period sets per task.
    ///
    /// `telescope.hard_constraints` are evaluated once over `timeline`; the
    /// resulting periods are then intersected with each task's static
    /// constraints and any dynamic constraints are evaluated only on those
    /// narrowed windows.
    pub fn run(
        blocks: &[SchedulingBlock],
        tasks: &HashMap<TaskId, Task>,
        timeline: &Period<MJD>,
        telescope: &Telescope,
    ) -> Result<TaskPeriodMap, ScheduleError> {
        log::info!(
            "prescheduler: starting — blocks={}, tasks={}, timeline=[{:.4}, {:.4}]",
            blocks.len(),
            tasks.len(),
            timeline.start.value(),
            timeline.end.value(),
        );

        let telescope_feasible =
            telescope
                .hard_constraints
                .check_hard(timeline, None, Some(&telescope.location))?;

        log::info!(
            "prescheduler: telescope feasibility — {} window(s)",
            telescope_feasible.as_slice().len(),
        );

        let task_ids: Vec<TaskId> = blocks.iter().flat_map(SchedulingBlock::iter).collect();

        if telescope_feasible.is_empty() {
            log::warn!("prescheduler: telescope has no feasibility on this horizon");
            let mut out = TaskPeriodMap::with_capacity(task_ids.len());
            for task_id in &task_ids {
                out.insert(*task_id, PeriodSet::new());
            }
            return Ok(out);
        }

        let parallel = should_parallelize(task_ids.len());
        let cache = Mutex::new(HashMap::new());

        log::debug!(
            "prescheduler: evaluating {} task ids ({})",
            task_ids.len(),
            if parallel { "parallel" } else { "sequential" },
        );

        let eval = |task_id: &TaskId| -> Result<(TaskId, PeriodSet<MJD>), ScheduleError> {
            let task = tasks.get(task_id).ok_or(ScheduleError::TaskNotFound)?;
            let task_static = task.hard_constraints.hard_static.check(
                timeline,
                Some(&telescope.location),
                Some(&task.target),
            );
            let candidate = task_static.intersection(&telescope_feasible);
            if candidate.is_empty() {
                return Ok((*task_id, candidate));
            }

            let feasible = dynamic_feasible(task, telescope, &candidate, &cache);
            Ok((*task_id, feasible))
        };

        let evaluated: Vec<Result<(TaskId, PeriodSet<MJD>), ScheduleError>> = if parallel {
            task_ids.par_iter().map(eval).collect()
        } else {
            task_ids.iter().map(eval).collect()
        };

        let mut out = TaskPeriodMap::with_capacity(task_ids.len());
        for item in evaluated {
            let (task_id, feasible) = item?;
            if feasible.is_empty() {
                log::warn!("prescheduler: task {} has no feasible windows", task_id.0);
            }
            if out.insert(task_id, feasible).is_some() {
                return Err(ScheduleError::InvalidTask(format!(
                    "duplicate task id {} across scheduling blocks",
                    task_id.0
                )));
            }
        }

        let total_windows: usize = out.values().map(|p| p.as_slice().len()).sum();
        log::info!(
            "prescheduler: done — {} tasks, {} total feasible windows",
            out.len(),
            total_windows,
        );

        Ok(out)
    }
}

pub fn preschedule(
    blocks: &[SchedulingBlock],
    tasks: &HashMap<TaskId, Task>,
    timeline: &Period<MJD>,
    telescope: &Telescope,
) -> Result<TaskPeriodMap, ScheduleError> {
    Prescheduler::run(blocks, tasks, timeline, telescope)
}
