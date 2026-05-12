//! Persistent prescheduler cache.
//!
//! Caches `(SchedulingProblem, Horizon) → schedulable block ids` so the
//! comparison endpoint can compute block ratios without re-running the
//! prescheduler on every request. Cache files live under
//! `<workspaces_root>/.preschedule-cache/<sha>.json`.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use scheduler::manifest::Horizon;
use scheduler::time::{MJD, Period, Time};
use scheduler::{SchedulingProblem, Telescope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrescheduleCacheEntry {
    pub schedulable_block_ids: Vec<String>,
    pub total_block_count: u64,
}

#[derive(Debug)]
pub struct PrescheduleCache {
    root: PathBuf,
}

impl PrescheduleCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn key(problem: &SchedulingProblem, horizon: Horizon) -> String {
        let problem_bytes = serialize_problem_for_key(problem);
        let mut hasher = Sha256::new();
        hasher.update(&problem_bytes);
        hasher.update(b"|");
        let suffix = format!("{:.6}..{:.6}", horizon.start_mjd_utc, horizon.end_mjd_utc);
        hasher.update(suffix.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn get_or_compute(
        &self,
        problem: &SchedulingProblem,
        horizon: Horizon,
    ) -> io::Result<PrescheduleCacheEntry> {
        let key = Self::key(problem, horizon);
        let path = self.root.join(format!("{key}.json"));
        if let Ok(bytes) = fs::read(&path)
            && let Ok(entry) = serde_json::from_slice::<PrescheduleCacheEntry>(&bytes)
        {
            return Ok(entry);
        }

        let entry = compute_entry(problem, horizon).map_err(io::Error::other)?;

        fs::create_dir_all(&self.root)?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            let bytes = serde_json::to_vec(&entry).map_err(io::Error::other)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(entry)
    }
}

/// Approximate canonical form of the problem for use as a cache key.
/// Uses the raw block + task ids + durations + targets — enough to
/// detect "different problem" without full constraint serialisation
/// (which would require additional Serialize impls).
fn serialize_problem_for_key(problem: &SchedulingProblem) -> Vec<u8> {
    let mut s = String::new();
    for block in problem.blocks() {
        s.push_str(&format!("B:{}\n", block.id.0));
        for task in block.tasks() {
            s.push_str(&format!(
                "T:{}|d={:.6}|az={:.6}|pol={:.6}\n",
                task.id.0,
                task.duration.value(),
                task.target.azimuth.value(),
                task.target.polar.value(),
            ));
        }
    }
    s.into_bytes()
}

fn compute_entry(
    problem: &SchedulingProblem,
    horizon: Horizon,
) -> Result<PrescheduleCacheEntry, String> {
    let telescope: &Telescope = problem
        .telescope
        .as_ref()
        .ok_or_else(|| "scheduling problem has no telescope resource".to_string())?;
    let timeline: Period<MJD> = Period::new(
        Time::<MJD>::new(horizon.start_mjd_utc),
        Time::<MJD>::new(horizon.end_mjd_utc),
    );
    let task_periods = scheduler::preschedule(problem, &timeline, telescope)
        .map_err(|e| format!("prescheduler failed: {e:?}"))?;

    let total_block_count = problem.block_count() as u64;
    let mut schedulable_block_ids: Vec<String> = Vec::new();

    let task_has_window = |id: scheduler::time::TaskId| -> bool {
        task_periods
            .get(&id)
            .map(|p| !p.is_empty())
            .unwrap_or(false)
    };

    for block in problem.blocks() {
        let block_id = block.id.0.to_string();
        let satisfiable = match block.completion_branches() {
            Some(branches) if !branches.is_empty() => branches
                .iter()
                .any(|branch| branch.iter().all(|tid| task_has_window(*tid))),
            _ => block.iter().all(task_has_window),
        };
        if satisfiable {
            schedulable_block_ids.push(block_id);
        }
    }

    Ok(PrescheduleCacheEntry {
        schedulable_block_ids,
        total_block_count,
    })
}

#[allow(dead_code)]
fn _silence_unused_hashmap() -> HashMap<u32, u32> {
    HashMap::new()
}
