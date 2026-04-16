use super::{Period, PeriodExt, Time, TimeScale};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct PeriodSet<S: TimeScale> {
	periods: Vec<Period<S>>,
}

impl<S: TimeScale> Default for PeriodSet<S> {
	fn default() -> Self {
		Self { periods: Vec::new() }
	}
}

impl<S: TimeScale> PeriodSet<S> {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn from_periods(periods: Vec<Period<S>>) -> Self {
		Self {
			periods: Self::normalize(periods),
		}
	}

	pub fn as_slice(&self) -> &[Period<S>] {
		&self.periods
	}

	pub fn iter(&self) -> std::slice::Iter<'_, Period<S>> {
		self.periods.iter()
	}

	pub fn is_empty(&self) -> bool {
		self.periods.is_empty()
	}

	pub fn len(&self) -> usize {
		self.periods.len()
	}

	pub fn contains_point(&self, t: Time<S>) -> bool {
		let idx = self.periods.partition_point(|p| p.start <= t);
		if idx == 0 {
			return false;
		}

		let p = self.periods[idx - 1];
		t < p.end
	}

	pub fn overlaps(&self, period: Period<S>) -> bool {
		let idx = self.periods.partition_point(|p| p.start < period.end);
		if idx == 0 {
			return false;
		}

		self.periods[idx - 1].overlaps(&period)
	}

	pub fn insert(&mut self, period: Period<S>) {
		let mut all = std::mem::take(&mut self.periods);
		all.push(period);
		self.periods = Self::normalize(all);
	}

	pub fn intersection(&self, other: &Self) -> Self {
		let mut out = Vec::new();
		let mut i = 0;
		let mut j = 0;

		while i < self.periods.len() && j < other.periods.len() {
			let a = self.periods[i];
			let b = other.periods[j];

			if let Some(intersection) = a.intersects(&b) {
				out.push(intersection);
			}

			if a.end <= b.end {
				i += 1;
			} else {
				j += 1;
			}
		}

		Self { periods: out }
	}

	pub fn union(&self, other: &Self) -> Self {
		let mut out = Vec::with_capacity(self.periods.len() + other.periods.len());
		let mut i = 0;
		let mut j = 0;

		while i < self.periods.len() && j < other.periods.len() {
			let next = if self.periods[i].start <= other.periods[j].start {
				let p = self.periods[i];
				i += 1;
				p
			} else {
				let p = other.periods[j];
				j += 1;
				p
			};

			Self::push_merged(&mut out, next);
		}

		while i < self.periods.len() {
			let p = self.periods[i];
			i += 1;
			Self::push_merged(&mut out, p);
		}

		while j < other.periods.len() {
			let p = other.periods[j];
			j += 1;
			Self::push_merged(&mut out, p);
		}

		Self { periods: out }
	}

	fn normalize(mut periods: Vec<Period<S>>) -> Vec<Period<S>> {
		if periods.is_empty() {
			return periods;
		}

		periods.sort_by(|a, b| {
			let by_start = a.start.partial_cmp(&b.start).unwrap_or(Ordering::Equal);
			if by_start != Ordering::Equal {
				return by_start;
			}

			a.end.partial_cmp(&b.end).unwrap_or(Ordering::Equal)
		});

		let mut out = Vec::with_capacity(periods.len());
		for period in periods {
			Self::push_merged(&mut out, period);
		}

		out
	}

	fn push_merged(out: &mut Vec<Period<S>>, period: Period<S>) {
		if let Some(last) = out.last_mut() {
			if period.start <= last.end {
				if period.end > last.end {
					last.end = period.end;
				}
				return;
			}
		}

		out.push(period);
	}
}

impl<'a, S: TimeScale> IntoIterator for &'a PeriodSet<S> {
	type Item = &'a Period<S>;
	type IntoIter = std::slice::Iter<'a, Period<S>>;

	fn into_iter(self) -> Self::IntoIter {
		self.periods.iter()
	}
}

impl<S: TimeScale> IntoIterator for PeriodSet<S> {
	type Item = Period<S>;
	type IntoIter = std::vec::IntoIter<Period<S>>;

	fn into_iter(self) -> Self::IntoIter {
		self.periods.into_iter()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::time::MJD;

	fn p(start: f64, end: f64) -> Period<MJD> {
		Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
	}

	#[test]
	fn normalize_merges_overlapping_and_adjacent() {
		let set = PeriodSet::<MJD>::from_periods(vec![
			p(10.0, 20.0),
			p(15.0, 25.0),
			p(25.0, 30.0),
			p(40.0, 50.0),
		]);

		assert_eq!(set.as_slice(), &[p(10.0, 30.0), p(40.0, 50.0)]);
	}

	#[test]
	fn intersection_works() {
		let a = PeriodSet::<MJD>::from_periods(vec![p(10.0, 20.0), p(30.0, 50.0)]);
		let b = PeriodSet::<MJD>::from_periods(vec![p(15.0, 35.0), p(40.0, 60.0)]);

		let result = a.intersection(&b);

		assert_eq!(
			result.as_slice(),
			&[p(15.0, 20.0), p(30.0, 35.0), p(40.0, 50.0)]
		);
	}

	#[test]
	fn union_works() {
		let a = PeriodSet::<MJD>::from_periods(vec![p(10.0, 20.0), p(30.0, 40.0)]);
		let b = PeriodSet::<MJD>::from_periods(vec![p(15.0, 35.0), p(50.0, 60.0)]);

		let result = a.union(&b);

		assert_eq!(result.as_slice(), &[p(10.0, 40.0), p(50.0, 60.0)]);
	}

	#[test]
	fn contains_point_works() {
		let set = PeriodSet::<MJD>::from_periods(vec![p(10.0, 20.0), p(30.0, 40.0)]);

		assert!(set.contains_point(Time::<MJD>::new(15.0)));
		assert!(!set.contains_point(Time::<MJD>::new(20.0)));
		assert!(!set.contains_point(Time::<MJD>::new(25.0)));
	}

	#[test]
	fn iterates_in_order() {
		let set = PeriodSet::<MJD>::from_periods(vec![p(30.0, 40.0), p(10.0, 20.0)]);

		let starts: Vec<f64> = set.iter().map(|period| period.start.value()).collect();
		assert_eq!(starts, vec![10.0, 30.0]);
	}
}