//! The aggregate schedule: tasks, blocks, placements, and the interval index.
//!
//! [`Schedule`] is the central data structure.  It owns:
//! - a set of [`Task`]s keyed by [`TaskId`]
//! - a set of [`SchedulingBlock`]s keyed by [`SchedulingBlockId`]
//! - a set of [`TaskPlacement`]s keyed by [`TaskId`]
//! - a mutable interval index for fast overlap queries
//!
//! ## Interval index
//!
//! The interval index is a sorted `BTreeMap<OrderedFloat<f64>, TaskId>` keyed
//! by start time (JD).  A second `BTreeMap` keyed by end time allows the
//! overlap query in `O(k log n)` where `k` is the number of overlapping
//! placements.  This keeps the implementation dependency-free (no extra crate)
//! while being correct and efficient enough for v1.

use crate::error::ScheduleError;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{Period, PeriodSet, SchedulingBlockId, TaskId, TimeInterval, TimePoint};
use ordered_float::NotNan;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::{JulianDate, MJD};
use std::collections::{BTreeMap, HashMap};

// ─── TaskPlacement ───────────────────────────────────────────────────────────

/// The concrete scheduled slot for a task.
#[derive(Debug, Clone)]
pub struct TaskPlacement {
    pub task_id: TaskId,
    pub start: TimePoint,
    pub end: TimePoint,
    /// The scheduling block this placement belongs to, if any.
    pub block_id: Option<SchedulingBlockId>,
}

impl TaskPlacement {
    /// Convenience: return the interval `[start, end)`.
    pub fn interval(&self) -> TimeInterval {
        TimeInterval::new(self.start, self.end)
    }
}

// ─── Interval index helpers ──────────────────────────────────────────────────

/// Key type for the interval index maps.
type JdKey = NotNan<f64>;

fn jd_key(jd: JulianDate) -> JdKey {
    NotNan::new(jd.value()).expect("schedule time must be finite")
}

// ─── Schedule ────────────────────────────────────────────────────────────────

/// The aggregate schedule.
///
/// All mutating operations go through the high-level methods which keep
/// the placements and interval index consistent.
#[derive(Debug, Default)]
pub struct Schedule {
    pub tasks: HashMap<TaskId, Task>,
    pub blocks: HashMap<SchedulingBlockId, SchedulingBlock>,
    pub placements: HashMap<TaskId, TaskPlacement>,

    /// Sorted by placement **start** time → task id.
    /// Multiple tasks can start at the same JD, so the value is a `Vec`.
    starts: BTreeMap<JdKey, Vec<TaskId>>,
    /// Sorted by placement **end** time → task id (for efficient overlap check).
    ends: BTreeMap<JdKey, Vec<TaskId>>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Task registry ─────────────────────────────────────────────────────

    /// Register a task.  Replaces any existing task with the same id.
    pub fn add_task(&mut self, task: Task) {
        self.tasks.insert(task.id, task);
    }

    // ── Block registry ────────────────────────────────────────────────────

    /// Register a scheduling block.
    pub fn add_block(&mut self, block: SchedulingBlock) {
        self.blocks.insert(block.id, block);
    }

    // ── Overlap query ─────────────────────────────────────────────────────

    /// Return the IDs of all currently placed tasks whose interval overlaps
    /// `[start, end)`.
    ///
    /// Two intervals `[a, b)` and `[c, d)` overlap iff `a < d && c < b`.
    pub fn overlapping(&self, interval: &TimeInterval) -> Vec<TaskId> {
        let req_start = jd_key(interval.start);
        let req_end = jd_key(interval.end);

        // A placed task `[p_start, p_end)` overlaps `[req_start, req_end)` iff:
        //   p_start < req_end  AND  p_end > req_start
        //
        // We iterate all placed tasks whose start < req_end (thanks to BTreeMap),
        // then filter for p_end > req_start.
        let mut result = Vec::new();
        for (_start, ids) in self.starts.range(..req_end) {
            for &id in ids {
                if let Some(placement) = self.placements.get(&id) {
                    let p_end = jd_key(placement.end);
                    if p_end > req_start {
                        result.push(id);
                    }
                }
            }
        }
        result
    }

    // ── Validation ────────────────────────────────────────────────────────

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
        task_id: TaskId,
        candidate: &TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        // 1. Task must exist.
        let task = self.tasks.get(&task_id).ok_or(ScheduleError::TaskNotFound)?;

        // 2. Duration check: interval length in seconds must match task.duration.
        let interval_days = candidate.end.value() - candidate.start.value();
        let interval_secs = interval_days * 86_400.0;
        let required_secs = task.duration.value();
        if (interval_secs - required_secs).abs() > 1e-3 {
            return Err(ScheduleError::IntervalDurationMismatch);
        }

        // 3. Overlap check.
        if !self.overlapping(candidate).is_empty() {
            return Err(ScheduleError::OverlapConflict);
        }

        // 4. Constraint tree.
        let candidate_period = Period::new(candidate.start.to::<MJD>(), candidate.end.to::<MJD>());
        let hard_task_feasible = task
            .constraints
            .check_hard(&candidate_period, Some(&task.target), Some(site))?;
        let dependency_feasible = self.block_dependency_feasible_periods(task_id, candidate, block_id)?;
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

        // 7. Soft qualification is prepared for scoring/selection but does not
        // block placement decisions yet.
        let _ = task
            .constraints
            .qualify_soft(&candidate_period, Some(&task.target), Some(site), &feasible)?;

        Ok(())
    }

    /// Built-in hard-dynamic filter for intra-block dependencies.
    ///
    /// Returns the candidate interval as a singleton set if dependencies are
    /// satisfied, or an empty set if they are not.
    fn block_dependency_feasible_periods(
        &self,
        task_id: TaskId,
        candidate: &TimeInterval,
        block_id: Option<SchedulingBlockId>,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        let candidate_set = PeriodSet::from_periods(vec![Period::new(
            candidate.start.to::<MJD>(),
            candidate.end.to::<MJD>(),
        )]);

        let Some(bid) = block_id else {
            return Ok(candidate_set);
        };

        let block = self.blocks.get(&bid).ok_or(ScheduleError::BlockNotFound)?;
        if !block.contains_task(task_id) {
            return Ok(candidate_set);
        }

        let order = block.topological_order()?;
        let task_pos = order.iter().position(|&t| t == task_id).unwrap_or(0);

        for predecessor_id in order.iter().take(task_pos) {
            match self.placements.get(predecessor_id) {
                None => return Ok(PeriodSet::new()),
                Some(prev) => {
                    if jd_key(prev.end) > jd_key(candidate.start) {
                        return Ok(PeriodSet::new());
                    }
                }
            }
        }

        Ok(candidate_set)
    }

    // ── Placement ─────────────────────────────────────────────────────────

    /// Place `task_id` at `candidate` after successful validation.
    ///
    /// Calls [`can_place`](Self::can_place) internally; returns its error if
    /// validation fails.
    pub fn place_task(
        &mut self,
        task_id: TaskId,
        candidate: TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        self.can_place(task_id, &candidate, block_id, site)?;

        let placement = TaskPlacement {
            task_id,
            start: candidate.start,
            end: candidate.end,
            block_id,
        };

        self.index_insert(&placement);
        self.placements.insert(task_id, placement);
        Ok(())
    }

    /// Remove the current placement of `task_id`, if any.
    ///
    /// Returns `Ok(())` even if the task was not placed.
    pub fn unplace_task(&mut self, task_id: TaskId) -> Result<(), ScheduleError> {
        if let Some(placement) = self.placements.remove(&task_id) {
            self.index_remove(&placement);
        }
        Ok(())
    }

    /// Move `task_id` to a new `candidate` interval.
    ///
    /// Equivalent to `unplace_task` + `place_task`, but atomic: if
    /// `place_task` fails the original placement is restored.
    pub fn move_task(
        &mut self,
        task_id: TaskId,
        new_interval: TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        let old = self.placements.remove(&task_id);
        if let Some(ref p) = old {
            self.index_remove(p);
        }

        match self.place_task(task_id, new_interval, block_id, site) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Restore old placement on failure.
                if let Some(p) = old {
                    self.index_insert(&p);
                    self.placements.insert(task_id, p);
                }
                Err(e)
            }
        }
    }

    // ── Index helpers ─────────────────────────────────────────────────────

    fn index_insert(&mut self, p: &TaskPlacement) {
        self.starts.entry(jd_key(p.start)).or_default().push(p.task_id);
        self.ends.entry(jd_key(p.end)).or_default().push(p.task_id);
    }

    fn index_remove(&mut self, p: &TaskPlacement) {
        if let Some(v) = self.starts.get_mut(&jd_key(p.start)) {
            v.retain(|&id| id != p.task_id);
            if v.is_empty() {
                self.starts.remove(&jd_key(p.start));
            }
        }
        if let Some(v) = self.ends.get_mut(&jd_key(p.end)) {
            v.retain(|&id| id != p.task_id);
            if v.is_empty() {
                self.ends.remove(&jd_key(p.end));
            }
        }
    }
}
