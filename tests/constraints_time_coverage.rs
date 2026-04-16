use qtty::Degrees;
use scheduler::{
    Period,
    constraints::{
        AltitudeConstraint, AzimuthConstraint, Constraint, ConstraintBlocks, ConstraintExpr,
        MoonAltitudeConstraint, MoonSeparationConstraint, NightConstraint, PrioritySoftConstraint,
        SoftConstraint, SoftConstraintExpr, TimeConstraint,
    },
    task::IcrsTarget,
    time::{MJD, PeriodSet, Time},
};
use siderust::calculus::solar::Twilight;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::{ECEF, ICRS};
use siderust::coordinates::spherical::Direction;

fn p(start: f64, end: f64) -> Period<MJD> {
    Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
}

fn roque() -> Geodetic<ECEF> {
    Geodetic::<ECEF>::new(
        Degrees::new(-17.892),
        Degrees::new(28.762),
        qtty::Quantity::<qtty::Meter>::new(2396.0),
    )
}

fn sirius_target() -> IcrsTarget {
    Direction::<ICRS>::new_raw(Degrees::new(-16.716), Degrees::new(101.287))
}

#[derive(Debug, Clone)]
struct FixedConstraint {
    out: PeriodSet<MJD>,
}

impl Constraint for FixedConstraint {
    fn check(
        &self,
        _timeline: &Period<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> PeriodSet<MJD> {
        self.out.clone()
    }
}

#[derive(Debug, Clone)]
struct PanicConstraint;

impl Constraint for PanicConstraint {
    fn check(
        &self,
        _timeline: &Period<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> PeriodSet<MJD> {
        panic!("panic constraint was evaluated unexpectedly");
    }
}

#[derive(Debug, Clone, Copy)]
struct FixedSoft {
    value: f64,
}

impl SoftConstraint for FixedSoft {
    fn score(
        &self,
        _time: &Time<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> f64 {
        self.value
    }
}

#[test]
fn constraint_expr_atom_delegates_to_inner_constraint() {
    let expr = ConstraintExpr::atom(FixedConstraint {
        out: PeriodSet::from_periods(vec![p(1.0, 2.0)]),
    });

    let out = expr.check(&p(0.0, 3.0), None, None);
    assert_eq!(out.as_slice(), &[p(1.0, 2.0)]);
}

#[test]
fn constraint_expr_union_merges_children() {
    let expr = ConstraintExpr::Union(vec![
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(0.0, 2.0)]),
        }),
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(2.0, 4.0)]),
        }),
    ]);

    let out = expr.check(&p(0.0, 5.0), None, None);
    assert_eq!(out.as_slice(), &[p(0.0, 4.0)]);
}

#[test]
fn constraint_expr_union_empty_children_is_empty() {
    let expr = ConstraintExpr::Union(vec![]);
    let out = expr.check(&p(0.0, 10.0), None, None);
    assert!(out.is_empty());
}

#[test]
fn constraint_expr_intersection_empty_children_returns_timeline() {
    let timeline = p(10.0, 20.0);
    let expr = ConstraintExpr::Intersection(vec![]);
    let out = expr.check(&timeline, None, None);
    assert_eq!(out.as_slice(), &[timeline]);
}

#[test]
fn constraint_expr_intersection_short_circuits_when_empty() {
    let expr = ConstraintExpr::Intersection(vec![
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(0.0, 10.0)]),
        }),
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::new(),
        }),
        ConstraintExpr::atom(PanicConstraint),
    ]);

    let out = expr.check(&p(0.0, 10.0), None, None);
    assert!(out.is_empty());
}

#[test]
fn constraint_expr_intersection_combines_children() {
    let expr = ConstraintExpr::Intersection(vec![
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(0.0, 8.0)]),
        }),
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(2.0, 6.0)]),
        }),
    ]);

    let out = expr.check(&p(0.0, 10.0), None, None);
    assert_eq!(out.as_slice(), &[p(2.0, 6.0)]);
}

#[test]
fn constraint_blocks_default_allows_full_timeline() {
    let timeline = p(100.0, 120.0);
    let blocks = ConstraintBlocks::default();
    let out = blocks
        .check_hard(&timeline, None, None)
        .expect("default hard checks should succeed");
    assert_eq!(out.as_slice(), &[timeline]);
}

#[test]
fn constraint_blocks_new_intersects_static_and_dynamic() {
    let blocks = ConstraintBlocks::new(
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(0.0, 7.0)]),
        }),
        ConstraintExpr::atom(FixedConstraint {
            out: PeriodSet::from_periods(vec![p(3.0, 10.0)]),
        }),
    );

    let out = blocks
        .check_hard(&p(0.0, 10.0), None, None)
        .expect("hard check should succeed");
    assert_eq!(out.as_slice(), &[p(3.0, 7.0)]);
}

#[test]
fn constraint_blocks_from_sets_static_only() {
    let blocks = ConstraintBlocks::from(ConstraintExpr::atom(FixedConstraint {
        out: PeriodSet::from_periods(vec![p(4.0, 9.0)]),
    }));

    let out = blocks
        .check_hard(&p(0.0, 10.0), None, None)
        .expect("hard check should succeed");
    assert_eq!(out.as_slice(), &[p(4.0, 9.0)]);
}

#[test]
fn soft_constraint_expr_atom_delegates() {
    let expr = SoftConstraintExpr::atom(FixedSoft { value: 0.25 });
    let score = expr.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 0.25).abs() < 1e-12);
}

#[test]
fn soft_constraint_expr_add_handles_empty_and_non_empty() {
    let empty = SoftConstraintExpr::Add(vec![]);
    assert!((empty.score(&Time::<MJD>::new(60000.0), None, None) - 0.0).abs() < 1e-12);

    let expr = SoftConstraintExpr::Add(vec![
        SoftConstraintExpr::atom(FixedSoft { value: 0.2 }),
        SoftConstraintExpr::atom(FixedSoft { value: 0.3 }),
    ]);
    let score = expr.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 0.5).abs() < 1e-12);
}

#[test]
fn soft_constraint_expr_sub_handles_empty_and_non_empty() {
    let empty = SoftConstraintExpr::Sub(vec![]);
    assert!((empty.score(&Time::<MJD>::new(60000.0), None, None) - 0.0).abs() < 1e-12);

    let expr = SoftConstraintExpr::Sub(vec![
        SoftConstraintExpr::atom(FixedSoft { value: 1.0 }),
        SoftConstraintExpr::atom(FixedSoft { value: 0.25 }),
        SoftConstraintExpr::atom(FixedSoft { value: 0.15 }),
    ]);
    let score = expr.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 0.6).abs() < 1e-12);
}

#[test]
fn soft_constraint_expr_mul_handles_empty_and_non_empty() {
    let empty = SoftConstraintExpr::Mul(vec![]);
    assert!((empty.score(&Time::<MJD>::new(60000.0), None, None) - 1.0).abs() < 1e-12);

    let expr = SoftConstraintExpr::Mul(vec![
        SoftConstraintExpr::atom(FixedSoft { value: 2.0 }),
        SoftConstraintExpr::atom(FixedSoft { value: 0.5 }),
    ]);
    let score = expr.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 1.0).abs() < 1e-12);
}

#[test]
fn soft_constraint_expr_div_handles_empty_nonzero_and_zero_denominator() {
    let empty = SoftConstraintExpr::Div(vec![]);
    assert!((empty.score(&Time::<MJD>::new(60000.0), None, None) - 1.0).abs() < 1e-12);

    let nonzero = SoftConstraintExpr::Div(vec![
        SoftConstraintExpr::atom(FixedSoft { value: 6.0 }),
        SoftConstraintExpr::atom(FixedSoft { value: 3.0 }),
    ]);
    let score = nonzero.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 2.0).abs() < 1e-12);

    let zero_denominator = SoftConstraintExpr::Div(vec![
        SoftConstraintExpr::atom(FixedSoft { value: 5.0 }),
        SoftConstraintExpr::atom(FixedSoft { value: 0.0 }),
    ]);
    let score = zero_denominator.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 0.0).abs() < 1e-12);
}

#[test]
fn priority_soft_constraint_uses_priority_value() {
    let c = PrioritySoftConstraint::new(0.9);
    let score = c.score(&Time::<MJD>::new(60000.0), None, None);
    assert!((score - 0.9).abs() < 1e-12);
}

#[test]
fn time_constraint_returns_overlap_period() {
    let c = TimeConstraint {
        window: p(2.0, 6.0),
    };
    let out = c.check(&p(4.0, 8.0), None, None);
    assert_eq!(out.as_slice(), &[p(4.0, 6.0)]);
}

#[test]
fn time_constraint_returns_empty_when_disjoint() {
    let c = TimeConstraint {
        window: p(2.0, 6.0),
    };
    let out = c.check(&p(6.0, 8.0), None, None);
    assert!(out.is_empty());
}

#[test]
#[should_panic(expected = "missing location for night constraint evaluation")]
fn night_constraint_panics_when_location_missing() {
    let c = NightConstraint {
        twilight: Twilight::Astronomical,
    };
    let _ = c.check(&p(60000.0, 60001.0), None, None);
}

#[test]
fn night_constraint_returns_periods_with_location() {
    let c = NightConstraint {
        twilight: Twilight::Astronomical,
    };
    let timeline = p(60000.0, 60002.0);
    let out = c.check(&timeline, Some(&roque()), None);

    assert!(!out.is_empty());
    assert!(out.iter().all(|period| {
        period.start >= timeline.start && period.end <= timeline.end && period.start < period.end
    }));
}

#[test]
#[should_panic(expected = "invalid altitude range")]
fn altitude_constraint_panics_for_invalid_range() {
    let c = AltitudeConstraint {
        min: Degrees::new(80.0),
        max: Degrees::new(20.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), Some(&roque()), Some(&sirius_target()));
}

#[test]
#[should_panic(expected = "missing target for altitude constraint evaluation")]
fn altitude_constraint_panics_when_target_missing() {
    let c = AltitudeConstraint {
        min: Degrees::new(-90.0),
        max: Degrees::new(90.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), Some(&roque()), None);
}

#[test]
#[should_panic(expected = "missing location for altitude constraint evaluation")]
fn altitude_constraint_panics_when_location_missing() {
    let c = AltitudeConstraint {
        min: Degrees::new(-90.0),
        max: Degrees::new(90.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), None, Some(&sirius_target()));
}

#[test]
fn altitude_constraint_wide_band_contains_midpoint() {
    let c = AltitudeConstraint {
        min: Degrees::new(-90.0),
        max: Degrees::new(90.0),
    };
    let timeline = p(60000.0, 60001.0);
    let out = c.check(&timeline, Some(&roque()), Some(&sirius_target()));
    assert!(out.contains_point(Time::<MJD>::new(60000.5)));
}

#[test]
#[should_panic(expected = "missing target for azimuth constraint evaluation")]
fn azimuth_constraint_panics_when_target_missing() {
    let c = AzimuthConstraint {
        min: Degrees::new(0.0),
        max: Degrees::new(360.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), Some(&roque()), None);
}

#[test]
#[should_panic(expected = "missing location for azimuth constraint evaluation")]
fn azimuth_constraint_panics_when_location_missing() {
    let c = AzimuthConstraint {
        min: Degrees::new(0.0),
        max: Degrees::new(360.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), None, Some(&sirius_target()));
}

#[test]
fn azimuth_constraint_wide_band_has_some_feasible_time() {
    let c = AzimuthConstraint {
        min: Degrees::new(0.0),
        max: Degrees::new(360.0),
    };
    let out = c.check(&p(60000.0, 60003.0), Some(&roque()), Some(&sirius_target()));
    assert!(!out.is_empty());
}

#[test]
#[should_panic(expected = "invalid moon altitude range")]
fn moon_altitude_constraint_panics_for_invalid_range() {
    let c = MoonAltitudeConstraint {
        min: Degrees::new(20.0),
        max: Degrees::new(10.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), Some(&roque()), None);
}

#[test]
#[should_panic(expected = "missing location for moon-altitude constraint evaluation")]
fn moon_altitude_constraint_panics_when_location_missing() {
    let c = MoonAltitudeConstraint {
        min: Degrees::new(-90.0),
        max: Degrees::new(90.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), None, None);
}

#[test]
fn moon_altitude_constraint_wide_band_has_some_feasible_time() {
    let c = MoonAltitudeConstraint {
        min: Degrees::new(-90.0),
        max: Degrees::new(90.0),
    };
    let out = c.check(&p(60000.0, 60001.0), Some(&roque()), None);
    assert!(!out.is_empty());
}

#[test]
#[should_panic(expected = "missing target for moon-separation constraint evaluation")]
fn moon_separation_constraint_panics_when_target_missing() {
    let c = MoonSeparationConstraint {
        min_separation: Degrees::new(1.0),
    };
    let _ = c.check(&p(60000.0, 60001.0), None, None);
}

#[test]
fn moon_separation_constraint_accepts_zero_minimum() {
    let c = MoonSeparationConstraint {
        min_separation: Degrees::new(0.0),
    };
    let timeline = p(60000.0, 60001.0);
    let out = c.check(&timeline, None, Some(&sirius_target()));
    assert_eq!(out.as_slice(), &[timeline]);
}

#[test]
fn moon_separation_constraint_rejects_threshold_over_180_degrees() {
    let c = MoonSeparationConstraint {
        min_separation: Degrees::new(181.0),
    };
    let out = c.check(&p(60000.0, 60001.0), None, Some(&sirius_target()));
    assert!(out.is_empty());
}
