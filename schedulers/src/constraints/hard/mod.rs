//! Hard constraints for observation scheduling.
//!
//! Hard constraints are represented as [`ConstraintExpr`] trees and produce
//! feasible period sets. All periods that violate a hard constraint are
//! completely infeasible and excluded from consideration.

mod altitude_constraint;
mod azimuth_constraint;
mod moon_altitude_constraint;
mod moon_separation_constraint;
mod night_constraint;
mod time_window_constraint;

pub use altitude_constraint::AltitudeConstraint;
pub use azimuth_constraint::AzimuthConstraint;
pub use moon_altitude_constraint::MoonAltitudeConstraint;
pub use moon_separation_constraint::MoonSeparationConstraint;
pub use night_constraint::NightConstraint;
pub use time_window_constraint::{TimeConstraint, TimeWindowConstraint};
