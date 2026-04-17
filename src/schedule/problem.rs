use super::{Schedule, TaskPlacement};
use crate::error::ScheduleError;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::telescope::Telescope;
use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, TaskId, TimeInterval};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use std::collections::HashMap;

/// The scheduling problem definition: available tasks and blocks.
///
/// When deserialized from JSON, `detected_horizon` is populated from the union
/// of all task `time_window` hard constraints found in the input. It is `None`
/// when the problem is constructed programmatically or when no time windows
/// are present in the input.
///
/// `telescope` is populated when input JSON provides an observing resource via
/// the envelope `resources[0]` entry (preferred) or a legacy top-level
/// `location` (in which case telescope hard constraints default to empty).
#[derive(Debug, Default)]
pub struct SchedulingProblem {
    pub tasks: HashMap<TaskId, Task>,
    pub blocks: HashMap<SchedulingBlockId, SchedulingBlock>,
    pub detected_horizon: Option<Period<MJD>>,
    pub telescope: Option<Telescope>,
}

impl SchedulingProblem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a task. Replaces any existing task with the same id.
    pub fn add_task(&mut self, task: Task) {
        self.tasks.insert(task.id, task);
    }

    /// Register a scheduling block.
    pub fn add_block(&mut self, block: SchedulingBlock) {
        self.blocks.insert(block.id, block);
    }

    /// Validate that `task_id` can be placed at `candidate`.
    ///
    /// Checks (in order):
    /// 1. Task exists.
    /// 2. Interval length matches task duration (within 1 ms tolerance).
    /// 3. No overlap with existing placements.
    /// 4. Hard-Static and Hard-Dynamic task constraints.
    /// 5. Built-in Hard-Dynamic dependency filtering (if a block is provided).
    /// 6. Candidate interval is fully covered by resulting feasible periods.
    /// 7. Soft-Static and Soft-Dynamic qualification (currently non-blocking).
    pub fn can_place(
        &self,
        schedule: &Schedule,
        task_id: TaskId,
        candidate: &TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        let task = self
            .tasks
            .get(&task_id)
            .ok_or(ScheduleError::TaskNotFound)?;

        let interval_days = candidate.end.value() - candidate.start.value();
        let interval_secs = interval_days * 86_400.0;
        let required_secs = task.duration.value();
        if (interval_secs - required_secs).abs() > 1e-3 {
            return Err(ScheduleError::IntervalDurationMismatch);
        }

        if !schedule.overlapping(candidate).is_empty() {
            return Err(ScheduleError::OverlapConflict);
        }

        let candidate_period = Period::new(candidate.start.to::<MJD>(), candidate.end.to::<MJD>());
        let hard_task_feasible =
            task.hard_constraints
                .check_hard(&candidate_period, Some(&task.target), Some(site))?;
        let dependency_feasible =
            self.block_dependency_feasible_periods(schedule, task_id, candidate, block_id)?;
        let feasible = hard_task_feasible.intersection(&dependency_feasible);

        let candidate_start = candidate.start.to::<MJD>();
        let candidate_end = candidate.end.to::<MJD>();
        let fully_covered = feasible
            .as_slice()
            .iter()
            .any(|period| period.start <= candidate_start && period.end >= candidate_end);

        if !fully_covered {
            return Err(ScheduleError::ConstraintViolation(
                "candidate interval is not fully covered by constraint periods".into(),
            ));
        }

        if let Some(soft_constraints) = &task.soft_constraints {
            let _ = soft_constraints.score(&candidate_start, Some(site), Some(&task.target));
        }

        Ok(())
    }

    /// Place `task_id` at `candidate` after successful validation.
    pub fn place_task(
        &self,
        schedule: &mut Schedule,
        task_id: TaskId,
        candidate: TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        self.can_place(schedule, task_id, &candidate, block_id, site)?;

        schedule.insert_placement(TaskPlacement {
            task_id,
            start: candidate.start,
            end: candidate.end,
            block_id,
        });

        Ok(())
    }

    /// Move `task_id` to a new `candidate` interval.
    ///
    /// Equivalent to `unplace_task` + `place_task`, but atomic: if
    /// `place_task` fails the original placement is restored.
    pub fn move_task(
        &self,
        schedule: &mut Schedule,
        task_id: TaskId,
        new_interval: TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        let old = schedule.placements.remove(&task_id);
        if let Some(ref placement) = old {
            schedule.index_remove(placement);
        }

        match self.place_task(schedule, task_id, new_interval, block_id, site) {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Some(placement) = old {
                    schedule.index_insert(&placement);
                    schedule.placements.insert(task_id, placement);
                }
                Err(error)
            }
        }
    }

    /// Built-in hard-dynamic filter for intra-block dependencies.
    ///
    /// Returns the candidate interval as a singleton set if dependencies are
    /// satisfied, or an empty set if they are not.
    fn block_dependency_feasible_periods(
        &self,
        schedule: &Schedule,
        task_id: TaskId,
        candidate: &TimeInterval,
        block_id: Option<SchedulingBlockId>,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        let candidate_set = PeriodSet::from_periods(vec![Period::new(
            candidate.start.to::<MJD>(),
            candidate.end.to::<MJD>(),
        )]);

        let Some(block_id) = block_id else {
            return Ok(candidate_set);
        };

        let block = self
            .blocks
            .get(&block_id)
            .ok_or(ScheduleError::BlockNotFound)?;
        if !block.contains_task(task_id) {
            return Ok(candidate_set);
        }

        let order = block.topological_order()?;
        let task_pos = order.iter().position(|&task| task == task_id).unwrap_or(0);

        for predecessor_id in order.iter().take(task_pos) {
            match schedule.placements.get(predecessor_id) {
                None => return Ok(PeriodSet::new()),
                Some(previous) if previous.end > candidate.start => return Ok(PeriodSet::new()),
                Some(_) => {}
            }
        }

        Ok(candidate_set)
    }
}
