//! Constraint expression tree.

use crate::error::ScheduleError;
use crate::period::Period;
use crate::period_set::PeriodSet;
use crate::task::IcrsTarget;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The outcome of a single constraint check.
pub type ConstraintResult = Result<PeriodSet<MJD>, ScheduleError>;

/// An atomic, composable scheduling constraint.
///
/// Implementors should be `Send + Sync` so that constraint trees can be
/// shared across threads in future parallel scheduling loops.
pub trait Constraint: Send + Sync + std::fmt::Debug {
    /// Return the feasible periods in MJD for this constraint.
    fn check(
        &self,
        timeline: &Period<MJD>,
        location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> ConstraintResult;
}

/// A logical constraint expression tree.
///
/// The tree evaluates bottom-up:
/// - `Atom` delegates to the inner [`Constraint`] implementation.
/// - `Union` merges feasible periods from all children.
/// - `Intersection` keeps only periods feasible in every child.
#[derive(Debug)]
pub enum ConstraintExpr {
    /// A leaf node wrapping a single constraint.
    Atom(Box<dyn Constraint>),
    /// Logical OR: at least one child must be satisfied.
    Union(Vec<ConstraintExpr>),
    /// Logical AND: every child must be satisfied.
    Intersection(Vec<ConstraintExpr>),
}

impl ConstraintExpr {
    /// Wrap a concrete [`Constraint`] in a leaf node.
    pub fn atom<C: Constraint + 'static>(c: C) -> Self {
        ConstraintExpr::Atom(Box::new(c))
    }

    /// Evaluate the expression tree over a candidate timeline and optional
    /// contextual data required by some constraints.
    pub fn check(
        &self,
        timeline: &Period<MJD>,
        location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> ConstraintResult {
        match self {
        location: Option<&Geodetic<ECEF>>,
            ConstraintExpr::Atom(c) => c.check(timeline, location, target),

            ConstraintExpr::Union(children) => {
                let mut out = PeriodSet::new();
                for child in children {
                    let set = child.check(timeline, location, target)?;
                    out = out.union(&set);
                }
                Ok(out)
            }

            ConstraintExpr::Intersection(children) => {
                if children.is_empty() {
                    return Ok(PeriodSet::from_periods(vec![Period::new(
                        timeline.start,
                        timeline.end,
                    )]));
                }

                let mut iter = children.iter();
                let mut out = iter
                    .next()
                    .expect("checked non-empty")
                    .check(timeline, location, target)?;

                for child in iter {
                    let set = child.check(timeline, location, target)?;
                    out = out.intersection(&set);
                    if out.is_empty() {
                        break;
                    }
                }

                Ok(out)
            }
        }
    }
}
