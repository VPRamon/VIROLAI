//! Soft constraint expression tree.

use crate::task::IcrsTarget;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::{MJD, Time};
use std::any::Any;

/// A soft scoring constraint that produces a numeric score.
///
/// Soft constraints evaluate to a floating-point score, typically in the
/// range [0.0, 1.0], where higher values indicate better feasibility.
/// Implementors should be `Send + Sync` so that constraint trees can be
/// shared across threads.
pub trait SoftConstraint: Send + Sync + std::fmt::Debug + Any {
    /// Return a score for the given context.
    fn score(
        &self,
        time: &Time<MJD>,
        location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> f64;
}

impl dyn SoftConstraint {
    pub fn downcast_ref<T: SoftConstraint + 'static>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

/// A soft constraint expression tree with arithmetic operations.
///
/// The tree evaluates bottom-up:
/// - `Atom` delegates to the inner [`SoftConstraint`] implementation.
/// - `Add` sums the scores of all children.
/// - `Sub` subtracts subsequent children from the first.
/// - `Mul` multiplies all scores.
/// - `Div` divides the first score by subsequent children.
#[derive(Debug)]
pub enum SoftConstraintExpr {
    /// A leaf node wrapping a single soft constraint.
    Atom(Box<dyn SoftConstraint>),
    /// Arithmetic addition: sum all child scores.
    Add(Vec<SoftConstraintExpr>),
    /// Arithmetic subtraction: first - rest.
    Sub(Vec<SoftConstraintExpr>),
    /// Arithmetic multiplication: product of all child scores.
    Mul(Vec<SoftConstraintExpr>),
    /// Arithmetic division: first / (rest multiplied).
    Div(Vec<SoftConstraintExpr>),
}

impl SoftConstraintExpr {
    /// Wrap a concrete [`SoftConstraint`] in a leaf node.
    pub fn atom<C: SoftConstraint + 'static>(c: C) -> Self {
        SoftConstraintExpr::Atom(Box::new(c))
    }

    /// Evaluate the expression tree and return a numeric score.
    pub fn score(
        &self,
        time: &Time<MJD>,
        location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> f64 {
        match self {
            SoftConstraintExpr::Atom(c) => c.score(time, location, target),

            SoftConstraintExpr::Add(children) => {
                if children.is_empty() {
                    return 0.0;
                }
                children
                    .iter()
                    .map(|child| child.score(time, location, target))
                    .sum()
            }

            SoftConstraintExpr::Sub(children) => {
                if children.is_empty() {
                    return 0.0;
                }
                let mut iter = children.iter();
                let first = iter.next().unwrap().score(time, location, target);
                let rest_sum: f64 = iter.map(|child| child.score(time, location, target)).sum();
                first - rest_sum
            }

            SoftConstraintExpr::Mul(children) => {
                if children.is_empty() {
                    return 1.0;
                }
                children
                    .iter()
                    .map(|child| child.score(time, location, target))
                    .product()
            }

            SoftConstraintExpr::Div(children) => {
                if children.is_empty() {
                    return 1.0;
                }
                let mut iter = children.iter();
                let first = iter.next().unwrap().score(time, location, target);
                let denominator: f64 = iter
                    .map(|child| child.score(time, location, target))
                    .product();
                if denominator == 0.0 {
                    return 0.0;
                }
                first / denominator
            }
        }
    }
}
