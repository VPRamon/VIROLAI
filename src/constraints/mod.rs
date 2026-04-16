//! Constraint system for observation scheduling.
//!
//! Constraints are organized in four blocks per task:
//! - Hard-Static
//! - Hard-Dynamic
//! - Soft-Static
//! - Soft-Dynamic
//!
//! Hard blocks are represented as [`ConstraintExpr`] trees and produce feasible
//! period sets. Soft blocks are represented as ordered lists of qualifiers that
//! can transform or rank feasible periods (scoring policy is intentionally TBD).

mod azimuth_constraint;
mod altitude_constraint;
mod expr;
mod moon_altitude_constraint;
mod moon_separation_constraint;
mod night_constraint;
mod time_window_constraint;
mod types;

pub use azimuth_constraint::AzimuthConstraint;
pub use altitude_constraint::AltitudeConstraint;
pub use expr::{Constraint, ConstraintExpr, ConstraintResult};
pub use moon_altitude_constraint::MoonAltitudeConstraint;
pub use moon_separation_constraint::MoonSeparationConstraint;
pub use night_constraint::NightConstraint;
pub use time_window_constraint::{TimeConstraint, TimeWindowConstraint};
pub use types::{
	ConstraintBlocks, HardConstraintExpr, SoftConstraint, SoftConstraintSet,
};
