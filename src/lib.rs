pub mod constraints;
pub mod error;
pub mod metrics;
pub mod prescheduler;
pub mod schedule;
pub mod scheduler;
pub mod scheduling_block;
pub mod telescope;
pub mod time;

pub(crate) mod serde_repr;

pub use prescheduler::{Prescheduler, TaskPeriodMap, preschedule};
pub use schedule::{
    LocationMeta, PeriodMeta, Schedule, ScheduleMetadata, ScheduleOutput, SchedulingProblem,
    TaskPlacement,
};
pub use scheduling_block::task;
pub use telescope::Telescope;
pub use time::IntervalTree;

pub use time::{Period, PeriodError, PeriodSet};
