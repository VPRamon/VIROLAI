use crate::schedule::TaskPlacement;
use crate::task::Task;
use crate::time::{JD, MJD, Period, PeriodSet, SchedulingBlockId, TaskId, Time};
use qtty::Day;

/// Convert a refreshed EST candidate into a concrete scheduled placement.
pub trait IntoTaskPlacement {
    /// Build a placement starting at the candidate's current EST.
    ///
    /// Callers must ensure the candidate is schedulable before invoking this.
    fn into_task_placement(self, horizon_end: Time<MJD>) -> TaskPlacement;
}

/// Mutable EST work item used during candidate ordering and placement.
#[derive(Debug, Clone)]
pub struct EstCandidate<'a> {
    task: &'a Task,
    windows: &'a PeriodSet<MJD>,
    pub(crate) next_window_idx: usize,
    pub est: Option<Time<MJD>>,
    pub deadline: Option<Time<MJD>>,
    pub flexibility: f64,
    /// Block this task belongs to, used for dependency-aware placement.
    pub(super) block_id: Option<SchedulingBlockId>,
}

impl<'a> EstCandidate<'a> {
    /// Create a candidate and immediately compute its first EST metadata.
    pub fn new(task: &'a Task, windows: &'a PeriodSet<MJD>, horizon: &Period<MJD>) -> Self {
        let mut candidate = Self {
            task,
            windows,
            next_window_idx: 0,
            est: None,
            deadline: None,
            flexibility: 0.0,
            block_id: None,
        };
        candidate.refresh(horizon);
        candidate
    }

    /// Return the task identifier carried by this candidate.
    pub const fn task_id(&self) -> TaskId {
        self.task.id
    }

    /// Return the task duration in days, matching the EST window arithmetic.
    pub fn duration(&self) -> qtty::Quantity<Day> {
        self.task.duration.to::<Day>()
    }

    /// Return the soft-priority score used as a tie-breaker in EST ordering.
    pub fn priority(&self, at: Time<MJD>) -> f64 {
        Self::task_score(self.task, at)
    }

    /// Recompute the candidate's EST metadata within the current horizon slice.
    ///
    /// Decision flow:
    /// 1. Skip windows already ending before the beam cursor.
    /// 2. Stop once future windows start beyond the current horizon.
    /// 3. Ignore intersections shorter than the task duration.
    /// 4. Use the first valid overlap as the EST.
    /// 5. Update the latest feasible start (`deadline`) and aggregate
    ///    flexibility from every valid overlap.
    pub fn refresh(&mut self, horizon: &Period<MJD>) {
        self.est = None;
        self.deadline = None;
        self.flexibility = 0.0;
        let windows = self.windows.as_slice();
        let duration = self.duration();
        let duration_days = duration.value();

        // Discard windows that are entirely behind the current beam cursor.
        while self.next_window_idx < windows.len()
            && windows[self.next_window_idx].end <= horizon.start
        {
            self.next_window_idx += 1;
        }

        for window in &windows[self.next_window_idx..] {
            // The remaining windows are ordered, so once a window starts after
            // the active horizon there can be no more relevant windows.
            if window.start >= horizon.end {
                break;
            }

            // Windows may partially overlap the beam horizon after prior
            // placements; only the intersection matters for feasibility.
            let Some(overlap) = window.intersection(horizon) else {
                continue;
            };

            let overlap_duration_days = overlap.duration().to::<Day>().value();
            // A window that cannot fit the whole task contributes nothing to EST
            // feasibility or flexibility.
            if overlap_duration_days < duration_days {
                continue;
            }

            // The first valid overlap determines the earliest feasible start.
            self.est.get_or_insert(overlap.start);
            // The latest feasible start is the end of the overlap minus the
            // required duration.
            self.deadline = Some(overlap.end - duration);
            // Flexibility is expressed as "how many task-length units" of
            // feasible time remain across all usable overlaps.
            self.flexibility += overlap_duration_days / duration_days;
        }

        log::trace!(
            "est: candidate task={} refreshed — est={}, deadline={}, flexibility={:.2}",
            self.task.id.0,
            self.est
                .map_or("none".to_string(), |t| format!("{:.4}", t.value())),
            self.deadline
                .map_or("none".to_string(), |t| format!("{:.4}", t.value())),
            self.flexibility,
        );
    }

    #[inline]
    /// Score a task using its soft constraints at the provided instant.
    fn task_score(task: &Task, at: Time<MJD>) -> f64 {
        task.soft_constraints
            .as_ref()
            .map(|expr| expr.score(&at, None, Some(&task.target)))
            .unwrap_or(0.0)
    }

    /// Return `true` when the remaining feasible time is less than one task.
    ///
    /// Such candidates cannot be scheduled from the current beam cursor.
    pub const fn is_impossible(&self) -> bool {
        self.flexibility < 1.0
    }
}

impl IntoTaskPlacement for EstCandidate<'_> {
    /// Materialise the candidate into a placement occupying `[est, est + duration)`.
    fn into_task_placement(self, horizon_end: Time<MJD>) -> TaskPlacement {
        let start = self
            .est
            .expect("EST invariant violated: candidate has no start time");
        let end = start + self.duration();
        assert!(
            end <= horizon_end,
            "EST invariant violated: candidate placement exceeds horizon"
        );

        TaskPlacement {
            task_id: self.task_id(),
            start: start.to::<JD>(),
            end: end.to::<JD>(),
            block_id: self.block_id,
        }
    }
}
