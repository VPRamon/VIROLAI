//! Constraint system for observation scheduling.
//!
//! Constraints are organized into hard feasibility expressions and soft
//! scoring expressions. Hard constraints are represented as
//! [`ConstraintExpr`] trees and produce feasible period sets. Soft
//! constraints are represented as [`SoftConstraintExpr`] trees that combine
//! scores using arithmetic operations.

pub mod hard;
pub mod soft;

mod expr;
mod soft_expr;
mod types;

pub use expr::{Constraint, ConstraintExpr};
pub use hard::{
    AltitudeConstraint, AzimuthConstraint, MoonAltitudeConstraint, MoonSeparationConstraint,
    NightConstraint, TimeConstraint, TimeWindowConstraint,
};
pub use soft::PrioritySoftConstraint;
pub use soft_expr::{SoftConstraint, SoftConstraintExpr};
pub use types::{ConstraintBlocks, HardConstraintExpr};
