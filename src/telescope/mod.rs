//! Telescope resource: observing site plus site-level hard constraints.
//!
//! A [`Telescope`] bundles the geodetic location of an observing site with
//! the hard constraints that apply to every task scheduled on that site
//! (e.g. night-time and Moon-below-horizon requirements). Telescope-level
//! constraints are evaluated once by the prescheduler and intersected into
//! each task's feasibility window.

pub(crate) mod serde_impl;

use crate::constraints::ConstraintBlocks;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;

#[derive(Debug)]
pub struct Telescope {
    pub id: u64,
    pub name: String,
    pub location: Geodetic<ECEF>,
    pub hard_constraints: ConstraintBlocks,
}

impl Telescope {
    pub fn new(
        id: u64,
        name: impl Into<String>,
        location: Geodetic<ECEF>,
        hard_constraints: ConstraintBlocks,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            location,
            hard_constraints,
        }
    }
}
