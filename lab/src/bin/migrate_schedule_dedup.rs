//! One-off disposable migration for registry schedule deduplication.
//!
//! This is intentionally not part of the long-term `lab registry` CLI. Make a
//! backup of the target `.sqlite` before running it.

use clap::Parser;
use lab::registry::{Registry, RunIdentity};
use lab::runner::regenerate_from_identity;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Parser)]
struct Args {
    /// Path to the existing registry SQLite database.
    #[arg(long, value_name = "PATH")]
    run_db: PathBuf,

    /// Recompute schedules from runs.identity_json.
    #[arg(long)]
    recompute: bool,

    /// Reserved for old schedule-file migrations; not implemented.
    #[arg(long, value_name = "DIR")]
    schedules_root: Option<PathBuf>,

    /// Compute and report without writing.
    #[arg(long)]
    dry_run: bool,

    /// Stop after N candidate rows.
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Recompute rows even when runs.schedule_hash is already present.
    #[arg(long)]
    force: bool,
}

#[derive(Debug)]
struct MigrationRow {
    run_key: String,
    identity_json: String,
    schedule_hash: Option<String>,
}

#[derive(Default)]
struct Summary {
    total: usize,
    skipped_existing: usize,
    migrated: usize,
    unique_schedules_inserted: usize,
    duplicate_schedules_detected: usize,
    failed: usize,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("migrate-schedule-dedup: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse();
    if !args.run_db.exists() {
        return Err(format!(
            "target DB does not exist: {}",
            args.run_db.display()
        ));
    }
    if !args.recompute {
        let hint = if args.schedules_root.is_some() {
            "migration from --schedules-root is not implemented; use --recompute"
        } else {
            "pass --recompute; historical schedule-file lookup is intentionally not guessed"
        };
        return Err(hint.to_string());
    }

    eprintln!("DB: {}", args.run_db.display());
    eprintln!("Make sure this DB has been backed up before writing.");
    if args.dry_run {
        eprintln!("dry run: no database writes will be made");
    }

    let _registry = Registry::open(&args.run_db)?;
    let mut conn = Connection::open(&args.run_db)
        .map_err(|e| format!("failed to open {}: {e}", args.run_db.display()))?;
    let rows = load_rows(&conn, args.limit)?;
    let mut summary = Summary {
        total: rows.len(),
        ..Summary::default()
    };
    let mut seen_hashes = HashSet::new();

    for (idx, row) in rows.iter().enumerate() {
        let prefix = &row.run_key[..row.run_key.len().min(16)];
        if row.schedule_hash.is_some() && !args.force {
            summary.skipped_existing += 1;
            eprintln!(
                "[{}/{}] skip {}: schedule_hash already set",
                idx + 1,
                rows.len(),
                prefix
            );
            continue;
        }

        match migrate_one(&mut conn, row, args.dry_run, &mut seen_hashes) {
            Ok(inserted_unique) => {
                summary.migrated += 1;
                if inserted_unique {
                    summary.unique_schedules_inserted += 1;
                } else {
                    summary.duplicate_schedules_detected += 1;
                }
                eprintln!("[{}/{}] migrated {}", idx + 1, rows.len(), prefix);
            }
            Err(e) => {
                summary.failed += 1;
                eprintln!("[{}/{}] failed {}: {e}", idx + 1, rows.len(), prefix);
            }
        }
    }

    println!("total rows: {}", summary.total);
    println!("skipped existing: {}", summary.skipped_existing);
    println!("migrated: {}", summary.migrated);
    println!(
        "unique schedules inserted: {}",
        summary.unique_schedules_inserted
    );
    println!(
        "duplicate schedules detected: {}",
        summary.duplicate_schedules_detected
    );
    println!("failed rows: {}", summary.failed);
    println!();
    println!("Duplicate report SQL:");
    println!(
        "SELECT
  substr(schedule_hash, 1, 16) AS schedule,
  COUNT(*) AS n,
  GROUP_CONCAT(substr(run_key, 1, 16), ', ') AS runs,
  GROUP_CONCAT(dataset_id || ':' || algorithm || ':' || config_slug, ' | ') AS configs
FROM runs
WHERE schedule_hash IS NOT NULL
GROUP BY schedule_hash
HAVING COUNT(*) > 1
ORDER BY n DESC;"
    );

    if summary.failed > 0 {
        return Err(format!("{} row(s) failed", summary.failed));
    }
    Ok(())
}

fn load_rows(conn: &Connection, limit: Option<usize>) -> Result<Vec<MigrationRow>, String> {
    let limit_sql = limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
    let sql = format!(
        "SELECT run_key, identity_json, schedule_hash FROM runs ORDER BY run_key{limit_sql}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("failed to query runs: {e}"))?;
    stmt.query_map([], |row| {
        Ok(MigrationRow {
            run_key: row.get(0)?,
            identity_json: row.get(1)?,
            schedule_hash: row.get(2)?,
        })
    })
    .map_err(|e| format!("failed to query runs: {e}"))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("failed to read run row: {e}"))
}

fn migrate_one(
    conn: &mut Connection,
    row: &MigrationRow,
    dry_run: bool,
    seen_hashes: &mut HashSet<String>,
) -> Result<bool, String> {
    let identity: RunIdentity = serde_json::from_str(&row.identity_json)
        .map_err(|e| format!("failed to parse identity_json: {e}"))?;
    let regenerated = regenerate_from_identity(&identity)?;
    let already_in_batch = !seen_hashes.insert(regenerated.schedule_hash.clone());
    let already_in_db: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM schedules WHERE schedule_hash = ?1",
            params![regenerated.schedule_hash],
            |db_row| db_row.get(0),
        )
        .optional()
        .map_err(|e| format!("failed to check existing schedule: {e}"))?;
    let inserted_unique = !already_in_batch && already_in_db.is_none();

    if !dry_run {
        let tx = conn
            .transaction()
            .map_err(|e| format!("failed to start write transaction: {e}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO schedules (
                schedule_hash, dataset_hash, schedule_json, created_at
            ) VALUES (?1, ?2, ?3, datetime('now'))",
            params![
                regenerated.schedule_hash,
                identity.dataset_hash,
                regenerated.schedule_json
            ],
        )
        .map_err(|e| format!("failed to insert schedule: {e}"))?;
        tx.execute(
            "UPDATE runs SET schedule_hash = ?2 WHERE run_key = ?1",
            params![row.run_key, regenerated.schedule_hash],
        )
        .map_err(|e| format!("failed to update run schedule_hash: {e}"))?;
        tx.commit()
            .map_err(|e| format!("failed to commit write transaction: {e}"))?;
    }

    Ok(inserted_unique)
}
