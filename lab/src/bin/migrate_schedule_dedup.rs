//! One-off migration: upgrade pre-deduplication `runs.sqlite` databases.
//!
//! Databases created before the schedule-deduplication fix stored the full
//! export artifact (raw problem body + `schedule_metadata` + `schedule_metrics`)
//! under the shared semantic `schedule_hash`, and had no `runs.metadata_json`
//! column. This tool upgrades such databases in place so that exports become
//! correct per run:
//!
//! 1. Backfills `runs.metadata_json` by re-deriving each run's metadata from its
//!    stored identity (the scheduler is **not** re-run; only the dataset is
//!    loaded). Rows whose dataset file is missing or whose content hash no
//!    longer matches are reported and skipped.
//! 2. Strips any embedded `schedule_metadata` / `schedule_metrics` from
//!    `schedules.schedule_json`, leaving only the invariant body.
//!
//! This is intentionally a standalone binary, not a subcommand of the main CLI.
//!
//! Usage:
//!   lab-migrate-schedule-dedup <path/to/runs.sqlite>

use std::path::PathBuf;
use std::process::ExitCode;

use lab::registry::{Registry, RunIdentity};
use lab::runner::metadata_json_from_identity;
use rusqlite::{Connection, params};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(db_arg) = args.next() else {
        eprintln!("usage: lab-migrate-schedule-dedup <path/to/runs.sqlite>");
        return ExitCode::FAILURE;
    };
    let db_path = PathBuf::from(db_arg);
    if !db_path.is_file() {
        eprintln!("error: database `{}` not found", db_path.display());
        return ExitCode::FAILURE;
    }

    match migrate(&db_path) {
        Ok(report) => {
            println!(
                "migration complete: {} run(s) backfilled, {} skipped, {} schedule body/bodies stripped",
                report.backfilled, report.skipped, report.stripped
            );
            if report.skipped > 0 {
                println!(
                    "note: {} run(s) could not be backfilled (see warnings above); \
                     re-run those with `lab run --override` once the datasets are available",
                    report.skipped
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("migration failed: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Report {
    backfilled: usize,
    skipped: usize,
    stripped: usize,
}

fn migrate(db_path: &PathBuf) -> Result<Report, String> {
    // Ensure the schema (including the metadata_json column) is up to date.
    Registry::open(db_path)?;

    let conn = Connection::open(db_path)
        .map_err(|e| format!("failed to open {}: {e}", db_path.display()))?;

    let mut report = Report::default();
    backfill_metadata(&conn, &mut report)?;
    strip_schedule_bodies(&conn, &mut report)?;
    Ok(report)
}

fn backfill_metadata(conn: &Connection, report: &mut Report) -> Result<(), String> {
    let pending: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT run_key, identity_json FROM runs
                  WHERE schedule_hash IS NOT NULL AND metadata_json IS NULL",
            )
            .map_err(|e| format!("failed to query runs: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("failed to read runs: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("failed to collect runs: {e}"))?
    };

    for (run_key, identity_json) in pending {
        let identity: RunIdentity = match serde_json::from_str(&identity_json) {
            Ok(identity) => identity,
            Err(e) => {
                eprintln!(
                    "  warn: run {} has unparsable identity: {e}",
                    short(&run_key)
                );
                report.skipped += 1;
                continue;
            }
        };
        match metadata_json_from_identity(&identity) {
            Ok(metadata_json) => {
                conn.execute(
                    "UPDATE runs SET metadata_json = ?2 WHERE run_key = ?1",
                    params![run_key, metadata_json],
                )
                .map_err(|e| format!("failed to update run {}: {e}", short(&run_key)))?;
                report.backfilled += 1;
            }
            Err(e) => {
                eprintln!("  warn: run {} not backfilled: {e}", short(&run_key));
                report.skipped += 1;
            }
        }
    }
    Ok(())
}

fn strip_schedule_bodies(conn: &Connection, report: &mut Report) -> Result<(), String> {
    let schedules: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT schedule_hash, schedule_json FROM schedules")
            .map_err(|e| format!("failed to query schedules: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("failed to read schedules: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("failed to collect schedules: {e}"))?
    };

    for (schedule_hash, body) in schedules {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&body) else {
            eprintln!(
                "  warn: schedule {} has invalid JSON; left as-is",
                short(&schedule_hash)
            );
            continue;
        };
        let Some(obj) = value.as_object_mut() else {
            continue;
        };
        let had_metadata = obj.remove("schedule_metadata").is_some();
        let had_metrics = obj.remove("schedule_metrics").is_some();
        if !had_metadata && !had_metrics {
            continue;
        }
        let stripped = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("failed to serialize stripped body: {e}"))?;
        conn.execute(
            "UPDATE schedules SET schedule_json = ?2 WHERE schedule_hash = ?1",
            params![schedule_hash, stripped],
        )
        .map_err(|e| format!("failed to update schedule {}: {e}", short(&schedule_hash)))?;
        report.stripped += 1;
    }
    Ok(())
}

fn short(s: &str) -> &str {
    &s[..s.len().min(16)]
}
