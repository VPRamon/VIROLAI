//! Latest-Start-Time (LST) scheduler implementation.
//!
//! This module wraps the EST beam search by mirroring the scheduling horizon:
//! tasks are reflected about the midpoint of the horizon before being passed
//! to EST, and the resulting schedule is reflected back.  The net effect is
//! that tasks are placed as *late* as possible rather than as early as
//! possible.
//!
//! - [`transform`]: pure mirroring / unmirroring utilities.
//! - [`algorithm`]: `LstScheduler` entry points and `MirroredFom` wrapper.

mod algorithm;
pub mod transform;

pub use algorithm::{LstScheduler, MirroredFom, run_scheduler};

#[cfg(test)]
mod tests;
