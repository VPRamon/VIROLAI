use crate::time::{TaskId, TimeInterval, TimePoint};

/// The concrete scheduled slot for a task.
#[derive(Debug, Clone)]
pub struct TaskPlacement {
    pub task_id: TaskId,
    pub start: TimePoint,
    pub end: TimePoint,
}

impl TaskPlacement {
    /// Convenience: return the interval `[start, end)` as a [`Period<MJD>`].
    pub fn interval(&self) -> TimeInterval {
        TimeInterval::new(self.start, self.end)
    }
}
