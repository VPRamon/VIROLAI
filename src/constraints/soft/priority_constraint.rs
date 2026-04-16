//! Priority-based soft constraint.

use super::super::soft_expr::SoftConstraint;
use crate::task::IcrsTarget;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::{MJD, Time};

/// A soft constraint that applies a priority weight.
///
/// Higher values indicate higher priority. The score returned is the priority
/// value itself.
#[derive(Debug, Clone)]
pub struct PrioritySoftConstraint {
    /// Free priority value.
    pub priority: f64,
}

impl PrioritySoftConstraint {
    /// Create a new priority soft constraint.
    pub fn new(priority: f64) -> Self {
        Self { priority }
    }
}

impl SoftConstraint for PrioritySoftConstraint {
    fn score(
        &self,
        _time: &Time<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> f64 {
        self.priority
    }
}
