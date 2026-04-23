//! HAP (Hybrid Asynchronous Proposal) scheduler.
//!
//! Each [`crate::scheduling_block::SchedulingBlock`] becomes one proposal.
//! HAP maintains a pool of `num_crus` survivor schedules and runs parallel
//! CRU (Constraint Repair Unit) workers to place one proposal's tasks at a
//! time, merging the best outcomes after each round.  The algorithm terminates
//! after a full queue rotation produces no improvement.

pub mod configuration;
mod cru;
pub mod proposal;
pub mod ranking;
pub mod scheduler;

pub use configuration::Configuration;
pub use scheduler::HapScheduler;
