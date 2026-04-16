//! Soft constraints for observation scheduling.
//!
//! Soft constraints are qualifiers that score feasibility on a numeric scale.
//! They use arithmetic expressions to combine multiple scoring constraints.

mod priority_constraint;

pub use priority_constraint::PrioritySoftConstraint;
