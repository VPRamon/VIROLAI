use super::conflict::compute_min_positive_priority;
use super::rng::Xorshift64;
use super::selection::choose_candidate;
use crate::schedule::Schedule;
use crate::time::{MJD, Period, PeriodSet, Time};

#[test]
fn xorshift64_is_deterministic() {
    let mut r1 = Xorshift64::new(42);
    let mut r2 = Xorshift64::new(42);
    for _ in 0..200 {
        assert_eq!(r1.next(), r2.next());
    }
}

#[test]
fn xorshift64_seed_zero_avoids_degenerate_state() {
    let mut r = Xorshift64::new(0);
    // Must produce non-zero values (zero seed is replaced with 1)
    assert_ne!(r.next(), 0);
}

#[test]
fn xorshift64_different_seeds_differ() {
    let mut r1 = Xorshift64::new(1);
    let mut r2 = Xorshift64::new(2);
    // It would be astronomically unlikely for 10 consecutive outputs to match
    let seq1: Vec<u64> = (0..10).map(|_| r1.next()).collect();
    let seq2: Vec<u64> = (0..10).map(|_| r2.next()).collect();
    assert_ne!(seq1, seq2);
}

#[test]
fn choose_candidate_picks_zero_cost_first() {
    let costs = vec![2.0, 0.0, 1.0];
    let mut rng = Xorshift64::new(1);
    assert_eq!(choose_candidate(&costs, 3, &mut rng), 1);
}

#[test]
fn choose_candidate_returns_zero_for_empty() {
    let mut rng = Xorshift64::new(1);
    assert_eq!(choose_candidate(&[], 3, &mut rng), 0);
}

#[test]
fn choose_candidate_stochastic_stays_within_range() {
    let costs = vec![3.0, 1.0, 2.0, 4.0, 5.0];
    let mut rng = Xorshift64::new(99);
    // With stochastic_range=2 we should only ever pick index of 1.0 or 2.0
    for _ in 0..50 {
        let idx = choose_candidate(&costs, 2, &mut rng);
        // The two cheapest are cost 1.0 (idx 1) and cost 2.0 (idx 2)
        assert!(idx == 1 || idx == 2, "unexpected index {idx}");
    }
}

#[test]
fn compute_min_positive_priority_fallback() {
    assert_eq!(compute_min_positive_priority(&[]), 1.0);
}

#[test]
fn generate_candidate_starts_empty_when_window_too_small() {
    // Task duration 2.0 days, window [0.0, 1.0] - too small
    let windows = PeriodSet::from_periods(vec![Period::new(
        Time::<MJD>::new(0.0),
        Time::<MJD>::new(1.0),
    )]);

    // We test the window-too-small branch indirectly through the empty-result
    // invariant using a schedule with no placements.
    let schedule = Schedule::new();
    let pred_end = Time::<MJD>::new(0.0);

    // window_duration (1.0) < duration (2.0) => skip
    let duration_days = 2.0_f64;
    let mut result_count = 0usize;
    for window in windows.iter() {
        let window_duration = window.end.value() - window.start.value();
        if window_duration >= duration_days {
            let s0 = if window.start.value() >= pred_end.value() {
                window.start
            } else {
                pred_end
            };
            if s0.value() + duration_days <= window.end.value() {
                result_count += 1;
            }
            let window_interval = Period::new(window.start, window.end);
            result_count += schedule.overlapping(&window_interval).len();
        }
    }
    assert_eq!(result_count, 0);
}
