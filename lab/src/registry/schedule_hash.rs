//! Canonical result-level schedule hashing.

use schedulers::schedule::Schedule;
use schedulers::time::MJD;
use serde::Serialize;
use sha2::{Digest, Sha256};

const MICROS_PER_DAY: f64 = 86_400_000_000.0;

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalPlacement<'a> {
    task_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_id: Option<&'a str>,
    start: i64,
    end: i64,
}

/// Computes the SHA-256 hex digest of the semantic schedule placements.
///
/// The canonical payload intentionally excludes schedule metadata, metrics,
/// provenance, and JSON formatting. Placements are sorted before hashing so
/// equivalent schedules produce the same hash regardless of map iteration
/// order.
pub fn canonical_schedule_hash(
    schedule: &Schedule,
    resource_id: Option<&str>,
) -> Result<String, String> {
    let canonical = canonical_json(schedule, resource_id)?;
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(format!("{digest:x}"))
}

fn canonical_json(schedule: &Schedule, resource_id: Option<&str>) -> Result<String, String> {
    let mut placements = schedule
        .placements()
        .map(|placement| {
            Ok(CanonicalPlacement {
                task_id: placement.task_id.0,
                resource_id,
                start: mjd_micros(placement.start.to::<MJD>().value())?,
                end: mjd_micros(placement.end.to::<MJD>().value())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    placements.sort();
    serde_json::to_string(&placements)
        .map_err(|e| format!("failed to serialize canonical schedule placements: {e}"))
}

fn mjd_micros(mjd: f64) -> Result<i64, String> {
    if !mjd.is_finite() {
        return Err("schedule placement time is not finite".to_string());
    }
    let micros = (mjd * MICROS_PER_DAY).round();
    if micros < i64::MIN as f64 || micros > i64::MAX as f64 {
        return Err(format!(
            "schedule placement time {mjd} is out of i64 microsecond range"
        ));
    }
    Ok(micros as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use schedulers::schedule::{Schedule, TaskPlacement};
    use schedulers::time::{MJD, TaskId, Time};

    fn placement(task_id: u64, start: f64, end: f64) -> TaskPlacement {
        TaskPlacement {
            task_id: TaskId(task_id),
            start: Time::<MJD>::new(start),
            end: Time::<MJD>::new(end),
        }
    }

    fn schedule(placements: &[TaskPlacement]) -> Schedule {
        let mut schedule = Schedule::new();
        for placement in placements {
            schedule.insert_placement(placement.clone());
        }
        schedule
    }

    #[test]
    fn canonical_hash_is_stable_across_placement_order() {
        let a = schedule(&[
            placement(2, 62000.2, 62000.3),
            placement(1, 62000.1, 62000.2),
        ]);
        let b = schedule(&[
            placement(1, 62000.1, 62000.2),
            placement(2, 62000.2, 62000.3),
        ]);

        assert_eq!(
            canonical_schedule_hash(&a, Some("MST-N")).unwrap(),
            canonical_schedule_hash(&b, Some("MST-N")).unwrap()
        );
    }

    #[test]
    fn canonical_hash_changes_when_semantics_change() {
        let base = schedule(&[placement(1, 62000.1, 62000.2)]);
        let different_task = schedule(&[placement(2, 62000.1, 62000.2)]);
        let different_start = schedule(&[placement(1, 62000.100001, 62000.2)]);
        let different_end = schedule(&[placement(1, 62000.1, 62000.200001)]);

        let base_hash = canonical_schedule_hash(&base, Some("MST-N")).unwrap();

        assert_ne!(
            base_hash,
            canonical_schedule_hash(&different_task, Some("MST-N")).unwrap()
        );
        assert_ne!(
            base_hash,
            canonical_schedule_hash(&base, Some("MST-S")).unwrap()
        );
        assert_ne!(
            base_hash,
            canonical_schedule_hash(&different_start, Some("MST-N")).unwrap()
        );
        assert_ne!(
            base_hash,
            canonical_schedule_hash(&different_end, Some("MST-N")).unwrap()
        );
    }
}
