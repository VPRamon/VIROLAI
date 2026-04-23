use super::{Schedule, TaskPlacement};
use crate::error::ScheduleError;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::telescope::Telescope;
use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, TaskId, TimeInterval};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
struct TaskLocation {
    block_idx: usize,
    task_idx: usize,
}

/// The scheduling problem definition: owned blocks, their owned tasks, and
/// optional horizon/telescope metadata parsed from input.
#[derive(Debug, Default)]
pub struct SchedulingProblem {
    blocks: Vec<SchedulingBlock>,
    block_index: HashMap<SchedulingBlockId, usize>,
    task_index: HashMap<TaskId, TaskLocation>,
    pub detected_horizon: Option<Period<MJD>>,
    pub telescope: Option<Telescope>,
}

impl SchedulingProblem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a problem from fully-populated blocks.
    pub fn from_blocks(blocks: Vec<SchedulingBlock>) -> Result<Self, ScheduleError> {
        let mut problem = Self::new();
        for block in blocks {
            problem.push_block(block)?;
        }
        Ok(problem)
    }

    /// Append one block, validating block-id and task-id uniqueness.
    pub fn push_block(&mut self, block: SchedulingBlock) -> Result<(), ScheduleError> {
        if self.block_index.contains_key(&block.id) {
            return Err(ScheduleError::InvalidTask(format!(
                "duplicate scheduling block id {}",
                block.id.0
            )));
        }

        for (task_idx, task) in block.iter_tasks().enumerate() {
            if self.task_index.contains_key(&task.id) {
                return Err(ScheduleError::InvalidTask(format!(
                    "duplicate task id {} across scheduling blocks",
                    task.id.0
                )));
            }
            self.task_index.insert(
                task.id,
                TaskLocation {
                    block_idx: self.blocks.len(),
                    task_idx,
                },
            );
        }

        self.block_index.insert(block.id, self.blocks.len());
        self.blocks.push(block);
        Ok(())
    }

    /// Borrow all blocks in input order.
    pub fn blocks(&self) -> &[SchedulingBlock] {
        &self.blocks
    }

    /// Count blocks in the problem.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Count tasks across all blocks.
    pub fn task_count(&self) -> usize {
        self.task_index.len()
    }

    /// Iterate all tasks in block/input order.
    pub fn iter_tasks(&self) -> impl Iterator<Item = &Task> {
        self.blocks.iter().flat_map(SchedulingBlock::iter_tasks)
    }

    /// Look up a task by stable identifier.
    pub fn task(&self, task_id: TaskId) -> Option<&Task> {
        let location = self.task_index.get(&task_id)?;
        self.blocks
            .get(location.block_idx)
            .and_then(|block| block.tasks().get(location.task_idx))
    }

    /// Look up a block by stable identifier.
    pub fn block(&self, block_id: SchedulingBlockId) -> Option<&SchedulingBlock> {
        self.block_index
            .get(&block_id)
            .and_then(|&idx| self.blocks.get(idx))
    }

    /// Return the block id that owns `task_id`, if any.
    pub fn task_block_id(&self, task_id: TaskId) -> Option<SchedulingBlockId> {
        self.task_index
            .get(&task_id)
            .map(|location| self.blocks[location.block_idx].id)
    }

    /// Validate that `task_id` can be placed at `candidate`.
    pub fn can_place(
        &self,
        schedule: &Schedule,
        task_id: TaskId,
        candidate: &TimeInterval,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        let task = self.task(task_id).ok_or(ScheduleError::TaskNotFound)?;

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
            self.block_dependency_feasible_periods(schedule, task_id, candidate)?;
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
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        self.can_place(schedule, task_id, &candidate, site)?;

        schedule.insert_placement(TaskPlacement {
            task_id,
            start: candidate.start,
            end: candidate.end,
        });

        Ok(())
    }

    /// Move `task_id` to a new `candidate` interval.
    pub fn move_task(
        &self,
        schedule: &mut Schedule,
        task_id: TaskId,
        new_interval: TimeInterval,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        let old = schedule.placements.remove(&task_id);
        if let Some(ref placement) = old {
            schedule.index_remove(placement);
        }

        match self.place_task(schedule, task_id, new_interval, site) {
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
    fn block_dependency_feasible_periods(
        &self,
        schedule: &Schedule,
        task_id: TaskId,
        candidate: &TimeInterval,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        let candidate_set = PeriodSet::from_periods(vec![Period::new(
            candidate.start.to::<MJD>(),
            candidate.end.to::<MJD>(),
        )]);

        let Some(block_id) = self.task_block_id(task_id) else {
            return Ok(candidate_set);
        };
        let block = self.block(block_id).ok_or(ScheduleError::BlockNotFound)?;

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
