//! Latest-Start-Time (LST) scheduler implementation.
//!
//! `LstScheduler` is a preconfigured **single-backward cursor** wrapper around
//! the shared cursor engine (see [`crate::scheduler::cursor`]).  Tasks are
//! placed as *late* as possible; the backward direction is handled internally
//! by the engine via `CursorFrame::Mirrored`.
//!
//! - [`transform`]: pure mirroring / unmirroring utilities (used by tests and
//!   the crate-internal `MirroredFom`).
//! - [`algorithm`]: `LstScheduler` entry points.

mod algorithm;
pub mod transform;

pub use algorithm::{LstScheduler, run_scheduler};

#[cfg(test)]
mod tests;
