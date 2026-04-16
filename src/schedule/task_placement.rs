use crate::time::{SchedulingBlockId, TaskId, TimeInterval, TimePoint};

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
    /// Convenience: return the interval `[start, end)` as a [`Period<JD>`].
    pub fn interval(&self) -> TimeInterval {
        TimeInterval::new(self.start, self.end)
    }
}