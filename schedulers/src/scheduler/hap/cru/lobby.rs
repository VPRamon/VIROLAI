//! Lobby: LIFO stack of displaced tasks awaiting rescheduling.
//!
//! Displaced tasks are pushed and popped from the same end of a [`Vec`],
//! so the most recently evicted task is rescheduled first.  This keeps
//! related displacements together and reduces thrashing.

use crate::time::TaskId;

/// A LIFO stack of [`TaskId`]s displaced from the schedule and awaiting
/// rescheduling.
pub type Lobby = Vec<TaskId>;
