use super::expr::ConstraintExpr;
use crate::error::ScheduleError;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

pub type HardConstraintExpr = ConstraintExpr;

#[derive(Debug)]
pub struct ConstraintBlocks {
    pub hard_static: HardConstraintExpr,
    pub hard_dynamic: HardConstraintExpr,
}

impl Default for ConstraintBlocks {
    fn default() -> Self {
        Self {
            hard_static: ConstraintExpr::Intersection(vec![]),
            hard_dynamic: ConstraintExpr::Intersection(vec![]),
        }
    }
}

impl ConstraintBlocks {
    pub fn new(
        hard_static: HardConstraintExpr,
        hard_dynamic: HardConstraintExpr,
    ) -> Self {
        Self {
            hard_static,
            hard_dynamic,
        }
    }

    pub fn check_hard(
        &self,
        timeline: &Period<MJD>,
        target: Option<&IcrsTarget>,
        location: Option<&Geodetic<ECEF>>,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        let static_feasible = self.hard_static.check(timeline, location, target);
        let dynamic_feasible = self.hard_dynamic.check(timeline, location, target);
        Ok(static_feasible.intersection(&dynamic_feasible))
    }
}

impl From<ConstraintExpr> for ConstraintBlocks {
    fn from(hard_expr: ConstraintExpr) -> Self {
        Self {
            hard_static: hard_expr,
            ..Self::default()
        }
    }
}
