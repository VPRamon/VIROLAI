//! `lab` binary entry point.
//!
//! Provides a `clap`-based CLI for running parameter-sweep lab runs
//! against the `schedulers` library.
//!
//! # Commands
//!
//! ## `run`
//!
//! ```text
//! lab run --spec <experiment.json>
//!                 [--run-db <PATH>]
//!                 [--override]
//! ```
//!
//! Loads an experiment spec, resolves the Cartesian product of cells,
//! skips cells already in the registry, and runs the rest in parallel.
//! With `--override`, existing registry rows are re-executed and updated.
//! All results (metrics and schedule JSON) are stored in SQLite.
//!
//! ## `registry`
//!
//! ```text
//! lab registry list   [--dataset <ID>] [--algorithm <NAME>] [--metric <NAME>]
//!                     [--min <VAL>] [--max <VAL>]
//!                     [--sort <METRIC:DIR>]... [--limit <N>]
//!                     [--run-db <PATH>] [--format json]
//! lab registry best   --dataset <ID> [--algorithm <NAME>]
//!                     [--sort <METRIC:DIR>]...
//!                     [--limit <N>] [--run-db <PATH>]
//! lab registry rank   [--dataset <ID>] [--algorithm <NAME>]
//!                     --weight <METRIC=WEIGHT>...
//! lab registry pareto [--dataset <ID>] [--algorithm <NAME>]
//!                     [--maximize <METRIC>]... [--minimize <METRIC>]...
//! lab registry inspect --run <KEY_OR_PREFIX> [--run-db <PATH>]
//! lab registry export  --run <KEY_OR_PREFIX> --out <FILE> [--force]
//!                      [--run-db <PATH>]
//! lab registry export  --out-dir <DIR>
//!                      [--dataset <ID>] [--algorithm <NAME>]
//!                      [--metric <NAME> --min <VAL> --max <VAL>]
//!                      [--sort <METRIC:DIR>]... [--limit <N>]
//!                      [--force] [--run-db <PATH>]
//! ```

use clap::{Parser, Subcommand};
use lab::cell::resolve_cells;
use lab::registry::{
    BestOpts, ListOpts, Registry, RunRow, SortKey, default_sort_keys, parse_sort_key, registry_path,
};
use lab::runner::{RunOptions, execute};
use lab::spec::ExperimentSpec;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ── Top-level CLI ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "lab",
    version,
    about = "Run parameter-sweep lab jobs against the PhD schedulers library"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run an experiment matrix.
    Run(RunArgs),
    /// Query the SQLite run registry.
    Registry(RegistryArgs),
}

// ── `run` sub-command ─────────────────────────────────────────────────────────

/// Arguments for `lab run`.
#[derive(Parser, Debug)]
struct RunArgs {
    /// Path to the experiment spec JSON.
    #[arg(long, value_name = "FILE")]
    spec: PathBuf,

    /// Path to the SQLite registry file.
    /// Defaults to `.lab/runs.sqlite`.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,

    /// Re-execute cells that already have a row in the registry and update
    /// their stored metrics and schedule JSON.
    #[arg(long = "override")]
    override_existing: bool,
}

// ── `registry` sub-command ────────────────────────────────────────────────────

#[derive(Parser, Debug)]
struct RegistryArgs {
    #[command(subcommand)]
    cmd: RegistryCmd,
}

#[derive(Subcommand, Debug)]
enum RegistryCmd {
    /// List run records (with optional filters).
    List(RegistryListArgs),
    /// Sort registry records by query-time metric keys.
    Sort(RegistrySortArgs),
    /// Show the best runs for a dataset.
    Best(RegistryBestArgs),
    /// Compute a weighted query-time score and rank matching records.
    Rank(RegistryRankArgs),
    /// Compute a Pareto front from objective metrics.
    Pareto(RegistryParetoArgs),
    /// Inspect a single run record.
    Inspect(RegistryInspectArgs),
    /// Export stored schedule JSON from the registry.
    Export(RegistryExportArgs),
}

#[derive(Parser, Debug)]
struct RegistryListArgs {
    /// Filter by dataset ID.
    #[arg(long, value_name = "ID")]
    dataset: Option<String>,

    /// Filter by algorithm name (`est` or `hap`).
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Metric column for --min / --max filtering.
    #[arg(long, value_name = "NAME")]
    metric: Option<String>,

    /// Minimum metric value (inclusive).
    #[arg(long, value_name = "VAL")]
    min: Option<f64>,

    /// Maximum metric value (inclusive).
    #[arg(long, value_name = "VAL")]
    max: Option<f64>,

    /// Sort key in `metric:asc` or `metric:desc` form. Repeat for
    /// lexicographic ordering. Alias: `--by`.
    #[arg(long = "sort", alias = "by", value_name = "METRIC:DIR")]
    sort: Vec<String>,

    /// Maximum number of rows to return (default: 100).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Output format: `table` (default) or `json`.
    #[arg(long, value_name = "FORMAT", default_value = "table")]
    format: String,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct RegistrySortArgs {
    /// Filter by dataset ID.
    #[arg(long, value_name = "ID")]
    dataset: Option<String>,

    /// Filter by algorithm name (`est` or `hap`).
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Sort key in `metric:asc` or `metric:desc` form. Repeat for
    /// lexicographic ordering. Alias: `--by`.
    #[arg(long = "sort", alias = "by", value_name = "METRIC:DIR")]
    sort: Vec<String>,

    /// Maximum number of rows to return (default: 20).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Output format: `table` (default) or `json`.
    #[arg(long, value_name = "FORMAT", default_value = "table")]
    format: String,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct RegistryBestArgs {
    /// Dataset ID to query.
    #[arg(long, value_name = "ID")]
    dataset: String,

    /// Restrict to a single algorithm.
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Sort key in `metric:asc` or `metric:desc` form. Repeat for
    /// lexicographic ordering. Alias: `--by`.
    #[arg(long = "sort", alias = "by", value_name = "METRIC:DIR")]
    sort: Vec<String>,

    /// Maximum number of results (default: 10).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Output format: `table` (default) or `json`.
    #[arg(long, value_name = "FORMAT", default_value = "table")]
    format: String,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct RegistryRankArgs {
    /// Filter by dataset ID.
    #[arg(long, value_name = "ID")]
    dataset: Option<String>,

    /// Restrict to a single algorithm.
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Query-time weight in `metric=value` form. Repeat to define a score.
    #[arg(long, value_name = "METRIC=WEIGHT")]
    weight: Vec<String>,

    /// Maximum number of results (default: 20).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Output format: `table` (default) or `json`.
    #[arg(long, value_name = "FORMAT", default_value = "table")]
    format: String,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct RegistryParetoArgs {
    /// Filter by dataset ID.
    #[arg(long, value_name = "ID")]
    dataset: Option<String>,

    /// Restrict to a single algorithm.
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Metric to maximize. Repeat as needed.
    #[arg(long, value_name = "METRIC")]
    maximize: Vec<String>,

    /// Metric to minimize. Repeat as needed.
    #[arg(long, value_name = "METRIC")]
    minimize: Vec<String>,

    /// Maximum number of front rows to print after default objective sorting.
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Output format: `table` (default) or `json`.
    #[arg(long, value_name = "FORMAT", default_value = "table")]
    format: String,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct RegistryInspectArgs {
    /// Full run key or unique prefix.
    #[arg(long, value_name = "KEY")]
    run: String,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct RegistryExportArgs {
    /// Full run key or unique prefix for single-run export. Requires `--out`.
    #[arg(long, value_name = "KEY")]
    run: Option<String>,

    /// Output file for single-run export. Requires `--run`.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// Output directory for filtered multi-run export.
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// Filter by dataset ID (filtered export).
    #[arg(long, value_name = "ID")]
    dataset: Option<String>,

    /// Filter by algorithm name (filtered export).
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Metric column for `--min` / `--max` filtering (filtered export).
    #[arg(long, value_name = "NAME")]
    metric: Option<String>,

    /// Minimum metric value (inclusive, filtered export).
    #[arg(long, value_name = "VAL")]
    min: Option<f64>,

    /// Maximum metric value (inclusive, filtered export).
    #[arg(long, value_name = "VAL")]
    max: Option<f64>,

    /// Sort key in `metric:asc` or `metric:desc` form (filtered export).
    /// Alias: `--by`.
    #[arg(long = "sort", alias = "by", value_name = "METRIC:DIR")]
    sort: Vec<String>,

    /// Maximum number of rows to export (filtered export, default: 100).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Overwrite output file(s) if they already exist.
    #[arg(long)]
    force: bool,

    /// Path to the SQLite registry file.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    env_logger::init();
    let cli = Cli::parse();
    match dispatch(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lab: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Run(args) => run(args),
        Cmd::Registry(args) => registry(args),
    }
}

// ── `run` implementation ──────────────────────────────────────────────────────

fn run(args: RunArgs) -> Result<(), String> {
    let spec = load_spec(&args.spec)?;
    let cells = resolve_cells(&spec)?;
    let opts = RunOptions {
        run_db: args.run_db,
        override_existing: args.override_existing,
    };
    let summary = execute(&spec, &cells, opts)?;
    println!(
        "lab run done: {} total, {} skipped, {} overridden, {} completed, {} failed",
        summary.total, summary.skipped, summary.overridden, summary.completed, summary.failed
    );
    if summary.failed > 0 {
        return Err(format!("{} cell(s) failed", summary.failed));
    }
    Ok(())
}

// ── `registry` dispatcher ─────────────────────────────────────────────────────

fn registry(args: RegistryArgs) -> Result<(), String> {
    match args.cmd {
        RegistryCmd::List(a) => registry_list(a),
        RegistryCmd::Sort(a) => registry_sort(a),
        RegistryCmd::Best(a) => registry_best(a),
        RegistryCmd::Rank(a) => registry_rank(a),
        RegistryCmd::Pareto(a) => registry_pareto(a),
        RegistryCmd::Inspect(a) => registry_inspect(a),
        RegistryCmd::Export(a) => registry_export(a),
    }
}

// ── `registry list` ───────────────────────────────────────────────────────────

fn registry_list(args: RegistryListArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let sort = parse_sort_keys(&args.sort)?;
    let opts = ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        metric: args.metric,
        min: args.min,
        max: args.max,
        sort: sort.clone(),
        limit: args.limit,
    };
    let rows = reg.list(&opts)?;
    print_rows(&rows, &args.format, &sort)?;
    Ok(())
}

// ── `registry sort` ───────────────────────────────────────────────────────────

fn registry_sort(args: RegistrySortArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let sort = parse_sort_keys(&args.sort)?;
    print_sort_policy(&sort);
    let rows = reg.list(&ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        sort: sort.clone(),
        limit: args.limit.or(Some(20)),
        ..Default::default()
    })?;
    print_rows(&rows, &args.format, &sort)
}

// ── `registry best` ───────────────────────────────────────────────────────────

fn registry_best(args: RegistryBestArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let sort = parse_sort_keys(&args.sort)?;
    let opts = BestOpts {
        dataset_id: args.dataset,
        algorithm: args.algorithm,
        sort: sort.clone(),
        limit: args.limit,
    };
    let rows = reg.best(&opts)?;
    print_sort_policy(&sort);
    print_rows(&rows, &args.format, &sort)
}

// ── `registry rank` ───────────────────────────────────────────────────────────

fn registry_rank(args: RegistryRankArgs) -> Result<(), String> {
    let weights = parse_weights(&args.weight)?;
    if weights.is_empty() {
        return Err("registry rank requires at least one --weight metric=value".to_string());
    }

    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let mut scored: Vec<(RunRow, f64)> = reg
        .list(&ListOpts {
            dataset: args.dataset,
            algorithm: args.algorithm,
            limit: Some(10_000_000),
            ..Default::default()
        })?
        .into_iter()
        .map(|row| {
            let metrics = parse_metrics(&row.metrics_json);
            let score = weights
                .iter()
                .map(|(metric, weight)| metric_value(&metrics, metric).unwrap_or(0.0) * weight)
                .sum();
            (row, score)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.run_key.cmp(&b.0.run_key))
    });
    scored.truncate(args.limit.unwrap_or(20));

    if args.format == "json" {
        let values: Vec<_> = scored
            .iter()
            .map(|(row, score)| row_json(row, Some(*score)))
            .collect();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
    } else {
        println!(
            "{:<18}  {:<12}  {:<8}  {:<30}  score",
            "run_key (prefix)", "dataset", "algo", "config_slug"
        );
        println!("{}", "-".repeat(96));
        for (row, score) in &scored {
            println!(
                "{:<18}  {:<12}  {:<8}  {:<30}  {:.6}",
                &row.run_key[..row.run_key.len().min(16)],
                row.dataset_id,
                row.algorithm,
                row.config_slug,
                score,
            );
        }
        println!("({} rows)", scored.len());
    }
    Ok(())
}

// ── `registry pareto` ─────────────────────────────────────────────────────────

fn registry_pareto(args: RegistryParetoArgs) -> Result<(), String> {
    let objectives = parse_objectives(&args.maximize, &args.minimize)?;
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let rows = reg.list(&ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        limit: Some(10_000_000),
        ..Default::default()
    })?;

    let mut front: Vec<RunRow> = rows
        .iter()
        .filter(|candidate| {
            !rows.iter().any(|other| {
                other.run_key != candidate.run_key && dominates(other, candidate, &objectives)
            })
        })
        .cloned()
        .collect();
    front.sort_by(compare_rows_by_default_policy);
    front.truncate(args.limit.unwrap_or(front.len()));

    if args.format == "json" {
        let values: Vec<_> = front.iter().map(|row| row_json(row, None)).collect();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
    } else {
        print_rows(&front, &args.format, &[])?;
    }
    Ok(())
}

// ── `registry inspect` ────────────────────────────────────────────────────────

fn registry_inspect(args: RegistryInspectArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let key = if args.run.len() == 64 {
        args.run.clone()
    } else {
        reg.resolve_prefix(&args.run)?
    };
    let row = reg
        .get_row(&key)?
        .ok_or_else(|| format!("no run found for key '{key}'"))?;
    println!("run_key:       {}", row.run_key);
    println!("dataset_id:    {}", row.dataset_id);
    println!("dataset_path:  {}", row.dataset_path);
    println!("algorithm:     {}", row.algorithm);
    println!("config_slug:   {}", row.config_slug);
    println!("created_at:    {}", row.created_at);
    println!("last_seen_at:  {}", row.last_seen_at);
    if let Some(cell) = &row.source_cell_id {
        println!("source_cell:   {cell}");
    }
    println!("\n--- identity ---");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.identity_json) {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("{}", row.identity_json);
    }
    println!("\n--- metrics ---");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.metrics_json) {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("{}", row.metrics_json);
    }
    Ok(())
}

// ── `registry export` ─────────────────────────────────────────────────────────

fn registry_export(args: RegistryExportArgs) -> Result<(), String> {
    match (&args.run, &args.out, &args.out_dir) {
        (Some(key), Some(out), None) => {
            // Single-run export: read stored schedule_json and write to --out.
            if out.exists() && !args.force {
                return Err(format!(
                    "output file '{}' already exists; use --force to overwrite",
                    out.display()
                ));
            }
            let db_path = registry_path(args.run_db.as_deref());
            let reg = Registry::open(&db_path)?;
            let full_key = if key.len() == 64 {
                key.clone()
            } else {
                reg.resolve_prefix(key)?
            };
            let row = reg
                .get_row(&full_key)?
                .ok_or_else(|| format!("no run found for key '{full_key}'"))?;
            let json = row.schedule_json.ok_or_else(|| {
                format!(
                    "run '{}' has no stored schedule JSON; rerun with `lab run --override` to regenerate",
                    &full_key[..full_key.len().min(16)]
                )
            })?;
            std::fs::write(out, json)
                .map_err(|e| format!("failed to write {}: {e}", out.display()))?;
            println!("exported -> {}", out.display());
            Ok(())
        }
        (None, None, Some(out_dir)) => {
            // Filtered multi-run export.
            if !out_dir.exists() {
                std::fs::create_dir_all(out_dir)
                    .map_err(|e| format!("failed to create output dir: {e}"))?;
            }
            let db_path = registry_path(args.run_db.as_deref());
            let reg = Registry::open(&db_path)?;
            let sort = parse_sort_keys(&args.sort)?;
            let rows = reg.list(&ListOpts {
                dataset: args.dataset,
                algorithm: args.algorithm,
                metric: args.metric,
                min: args.min,
                max: args.max,
                sort,
                limit: args.limit.or(Some(100)),
            })?;
            let mut exported = 0usize;
            let mut unavailable = 0usize;
            // Track filenames to detect collisions within this batch.
            let mut used_names: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for row in &rows {
                let Some(json) = &row.schedule_json else {
                    eprintln!(
                        "  skip (no schedule_json): {} — rerun with `lab run --override`",
                        &row.run_key[..row.run_key.len().min(16)]
                    );
                    unavailable += 1;
                    continue;
                };
                // Build filename: <dataset>__<algorithm>__<config>.json
                let base_name = format!(
                    "{}__{}__{}.json",
                    row.dataset_id, row.algorithm, row.config_slug
                );
                let filename = if used_names.contains(&base_name) {
                    // Collision: append run_key prefix.
                    let key_prefix = &row.run_key[..row.run_key.len().min(8)];
                    format!(
                        "{}__{}__{}__{}.json",
                        row.dataset_id, row.algorithm, row.config_slug, key_prefix
                    )
                } else {
                    base_name.clone()
                };
                used_names.insert(filename.clone());
                used_names.insert(base_name);
                let dest = out_dir.join(&filename);
                if dest.exists() && !args.force {
                    eprintln!("  skip (exists, use --force): {}", dest.display());
                    continue;
                }
                std::fs::write(&dest, json)
                    .map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
                exported += 1;
            }
            println!(
                "exported {exported} schedule(s) to {}{}",
                out_dir.display(),
                if unavailable > 0 {
                    format!(" ({unavailable} skipped: no schedule_json)")
                } else {
                    String::new()
                }
            );
            Ok(())
        }
        (Some(_), None, _) => Err("--run requires --out for single-run export".to_string()),
        (None, Some(_), _) => Err("--out requires --run for single-run export".to_string()),
        (Some(_), Some(_), Some(_)) => {
            Err("--out and --out-dir are mutually exclusive".to_string())
        }
        (None, None, None) => Err(
            "registry export requires either --run + --out (single) or --out-dir (filtered)"
                .to_string(),
        ),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_sort_keys(raw: &[String]) -> Result<Vec<SortKey>, String> {
    raw.iter().map(|s| parse_sort_key(s)).collect()
}

fn print_sort_policy(sort: &[SortKey]) {
    let keys = if sort.is_empty() {
        default_sort_keys()
    } else {
        sort.to_vec()
    };
    let policy = keys
        .iter()
        .map(|k| format!("{}:{}", k.metric, k.direction.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("registry query sort: {policy}");
}

fn print_rows(rows: &[RunRow], format: &str, sort: &[SortKey]) -> Result<(), String> {
    if format == "json" {
        let values: Vec<_> = rows.iter().map(|row| row_json(row, None)).collect();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
        return Ok(());
    }
    if format != "table" {
        return Err(format!(
            "unsupported output format '{format}', expected table or json"
        ));
    }
    print!("{}", format_metric_rows(rows, sort));
    Ok(())
}

fn row_json(row: &RunRow, score: Option<f64>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "run_key": row.run_key,
        "dataset_id": row.dataset_id,
        "algorithm": row.algorithm,
        "config_slug": row.config_slug,
        "created_at": row.created_at,
        "last_seen_at": row.last_seen_at,
        "source_cell_id": row.source_cell_id,
        "metrics": parse_metrics(&row.metrics_json),
    });
    if let Some(score) = score {
        value["query_score"] = serde_json::json!(score);
    }
    value
}

fn parse_metrics(metrics_json: &str) -> serde_json::Value {
    serde_json::from_str(metrics_json).unwrap_or(serde_json::Value::Null)
}

fn metric_value(mv: &serde_json::Value, metric: &str) -> Result<f64, String> {
    match metric {
        "task_ratio" | "scheduled_task_ratio" => {
            Ok(mv["scheduled_task_ratio"].as_f64().unwrap_or(0.0))
        }
        "scheduled_task_count" => Ok(mv["scheduled_task_count"].as_f64().unwrap_or(0.0)),
        "scheduled_priority_sum" => Ok(mv["scheduled_priority_sum"].as_f64().unwrap_or(0.0)),
        "priority_ratio" | "scheduled_priority_ratio" => {
            Ok(mv["scheduled_priority_ratio"].as_f64().unwrap_or(0.0))
        }
        "priority_density" => Ok(mv["priority_density"].as_f64().unwrap_or(0.0)),
        "scheduled_time_sec" => Ok(mv["scheduled_time_sec"].as_f64().unwrap_or(0.0)),
        "requested_time_sec" => Ok(mv["requested_time_sec"].as_f64().unwrap_or(0.0)),
        "scheduled_time_ratio" => Ok(mv["scheduled_time_ratio"].as_f64().unwrap_or(0.0)),
        "utilization" => Ok(mv["utilization"].as_f64().unwrap_or(0.0)),
        "fragmentation_index" => Ok(mv["fragmentation"]["fragmentation_index"]
            .as_f64()
            .unwrap_or(0.0)),
        "runtime_ms" | "scheduler_runtime_ms" => {
            Ok(mv["scheduler_runtime_ms"].as_f64().unwrap_or(0.0))
        }
        "composite_score" | "composite_rank_score" => Err(
            "composite_rank_score is persisted only for backward-compatible schedule metrics; define query-time weights with `registry rank` instead"
                .to_string(),
        ),
        _ => Err(format!("unsupported metric '{metric}'")),
    }
}

/// Extracts a metric from a parsed `metrics_json` value.
/// Returns `None` when the key is absent or null — never defaults to 0.0.
fn metric_opt(mv: &serde_json::Value, metric: &str) -> Option<f64> {
    match metric {
        "task_ratio" | "scheduled_task_ratio" => mv["scheduled_task_ratio"].as_f64(),
        "scheduled_task_count" => mv["scheduled_task_count"].as_f64(),
        "scheduled_priority_sum" => mv["scheduled_priority_sum"].as_f64(),
        "priority_ratio" | "scheduled_priority_ratio" => mv["scheduled_priority_ratio"].as_f64(),
        "priority_density" => mv["priority_density"].as_f64(),
        "scheduled_time_sec" => mv["scheduled_time_sec"].as_f64(),
        "requested_time_sec" => mv["requested_time_sec"].as_f64(),
        "scheduled_time_ratio" => mv["scheduled_time_ratio"].as_f64(),
        "utilization" => mv["utilization"].as_f64(),
        "fragmentation_index" => mv["fragmentation"]["fragmentation_index"].as_f64(),
        "runtime_ms" | "scheduler_runtime_ms" => mv["scheduler_runtime_ms"].as_f64(),
        _ => None,
    }
}

/// Canonical metric columns shown in every table output.
/// Tuple: `(metric_name, column_header, column_width, decimal_places)`.
const METRIC_DISPLAY_COLS: &[(&str, &str, usize, usize)] = &[
    ("scheduled_priority_sum", "psum", 9, 2),
    ("scheduled_priority_ratio", "p_ratio", 8, 4),
    ("scheduled_task_ratio", "t_ratio", 8, 4),
    ("scheduled_time_ratio", "time_r", 8, 4),
    ("priority_density", "density", 8, 4),
    ("runtime_ms", "runtime", 10, 2),
];

/// Formats `val` right-aligned within `width` characters with `prec` decimal places.
/// Absent values are shown as `"-"` right-aligned in the same field.
fn fmt_f64_col(val: Option<f64>, width: usize, prec: usize) -> String {
    let s = match val {
        Some(f) => format!("{:.prec$}", f, prec = prec),
        None => "-".to_string(),
    };
    let pad = width.saturating_sub(s.len());
    format!("{}{}", " ".repeat(pad), s)
}

/// Normalises a sort-metric name to the canonical key used in `METRIC_DISPLAY_COLS`.
fn normalize_metric_for_display(m: &str) -> &str {
    match m {
        "task_ratio" | "scheduled_task_ratio" => "scheduled_task_ratio",
        "priority_ratio" | "scheduled_priority_ratio" => "scheduled_priority_ratio",
        "scheduler_runtime_ms" => "runtime_ms",
        other => other,
    }
}

/// Truncates `s` to `max` characters, appending `".."` if trimmed.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}..", &s[..max.saturating_sub(2)])
    }
}

/// Formats `rows` as a metric table string (used by [`print_rows`] and tests).
///
/// Displays identity columns followed by the six canonical metric columns.
/// If `sort` contains metrics not in the default set, those are appended as
/// extra columns so the sort criterion is always visible.
/// Missing metric values print as `"-"` rather than `0.0`.
fn format_metric_rows(rows: &[RunRow], sort: &[SortKey]) -> String {
    let default_metrics: std::collections::HashSet<&str> =
        METRIC_DISPLAY_COLS.iter().map(|(m, _, _, _)| *m).collect();

    // Append columns for any sort key not already covered by the default set.
    let mut extra_cols: Vec<&str> = vec![];
    for sk in sort {
        let norm = normalize_metric_for_display(&sk.metric);
        if !default_metrics.contains(norm) && !extra_cols.contains(&norm) {
            extra_cols.push(norm);
        }
    }

    let extra_col_width: usize = 10;

    let mut out = String::new();

    // Header line
    out.push_str(&format!(
        "{:<18}  {:<12}  {:<8}  {:<30}",
        "run_key (prefix)", "dataset", "algo", "config_slug"
    ));
    for (_, hdr, w, _) in METRIC_DISPLAY_COLS {
        out.push_str("  ");
        let pad = w.saturating_sub(hdr.len());
        out.push_str(&" ".repeat(pad));
        out.push_str(hdr);
    }
    for col in &extra_cols {
        out.push_str("  ");
        let pad = extra_col_width.saturating_sub(col.len());
        out.push_str(&" ".repeat(pad));
        out.push_str(col);
    }
    out.push_str("  created_at\n");

    // Separator
    let metric_width: usize = METRIC_DISPLAY_COLS
        .iter()
        .map(|(_, _, w, _)| w + 2)
        .sum::<usize>();
    let extra_width: usize = extra_cols.len() * (extra_col_width + 2);
    let sep_len = 74 + metric_width + extra_width + 2 + 19;
    out.push_str(&"-".repeat(sep_len));
    out.push('\n');

    // Data rows
    for row in rows {
        let mv = parse_metrics(&row.metrics_json);
        out.push_str(&format!(
            "{:<18}  {:<12}  {:<8}  {:<30}",
            &row.run_key[..row.run_key.len().min(16)],
            truncate(&row.dataset_id, 12),
            truncate(&row.algorithm, 8),
            truncate(&row.config_slug, 30),
        ));
        for (metric, _, w, prec) in METRIC_DISPLAY_COLS {
            out.push_str("  ");
            out.push_str(&fmt_f64_col(metric_opt(&mv, metric), *w, *prec));
        }
        for col in &extra_cols {
            out.push_str("  ");
            out.push_str(&fmt_f64_col(metric_opt(&mv, col), extra_col_width, 4));
        }
        out.push_str("  ");
        out.push_str(&row.created_at);
        out.push('\n');
    }
    out.push_str(&format!("({} rows)\n", rows.len()));
    out
}

fn parse_weights(raw: &[String]) -> Result<Vec<(String, f64)>, String> {
    raw.iter()
        .map(|entry| {
            let (metric, weight) = entry
                .split_once('=')
                .ok_or_else(|| format!("invalid weight '{entry}', expected metric=value"))?;
            let metric = metric.trim();
            if metric.is_empty() {
                return Err(format!("invalid weight '{entry}', metric cannot be empty"));
            }
            let weight = weight
                .trim()
                .parse::<f64>()
                .map_err(|e| format!("invalid weight in '{entry}': {e}"))?;
            metric_value(&serde_json::Value::Null, metric)?;
            Ok((metric.to_string(), weight))
        })
        .collect()
}

fn parse_objectives(
    maximize: &[String],
    minimize: &[String],
) -> Result<Vec<(String, bool)>, String> {
    let mut objectives = Vec::new();
    if maximize.is_empty() && minimize.is_empty() {
        objectives.extend([
            ("scheduled_priority_ratio".to_string(), true),
            ("scheduled_task_ratio".to_string(), true),
            ("priority_density".to_string(), true),
            ("runtime_ms".to_string(), false),
        ]);
        return Ok(objectives);
    }
    for metric in maximize {
        metric_value(&serde_json::Value::Null, metric)?;
        objectives.push((metric.clone(), true));
    }
    for metric in minimize {
        metric_value(&serde_json::Value::Null, metric)?;
        objectives.push((metric.clone(), false));
    }
    Ok(objectives)
}

fn dominates(a: &RunRow, b: &RunRow, objectives: &[(String, bool)]) -> bool {
    let a_metrics = parse_metrics(&a.metrics_json);
    let b_metrics = parse_metrics(&b.metrics_json);
    let mut strictly_better = false;
    for (metric, maximize) in objectives {
        let av = metric_value(&a_metrics, metric).unwrap_or(0.0);
        let bv = metric_value(&b_metrics, metric).unwrap_or(0.0);
        if *maximize {
            if av < bv {
                return false;
            }
            strictly_better |= av > bv;
        } else {
            if av > bv {
                return false;
            }
            strictly_better |= av < bv;
        }
    }
    strictly_better
}

fn compare_rows_by_default_policy(a: &RunRow, b: &RunRow) -> std::cmp::Ordering {
    let am = parse_metrics(&a.metrics_json);
    let bm = parse_metrics(&b.metrics_json);
    metric_value(&bm, "scheduled_priority_ratio")
        .unwrap_or(0.0)
        .total_cmp(&metric_value(&am, "scheduled_priority_ratio").unwrap_or(0.0))
        .then_with(|| {
            metric_value(&bm, "scheduled_task_ratio")
                .unwrap_or(0.0)
                .total_cmp(&metric_value(&am, "scheduled_task_ratio").unwrap_or(0.0))
        })
        .then_with(|| {
            metric_value(&bm, "priority_density")
                .unwrap_or(0.0)
                .total_cmp(&metric_value(&am, "priority_density").unwrap_or(0.0))
        })
        .then_with(|| {
            metric_value(&am, "runtime_ms")
                .unwrap_or(0.0)
                .total_cmp(&metric_value(&bm, "runtime_ms").unwrap_or(0.0))
        })
        .then_with(|| a.run_key.cmp(&b.run_key))
}

// ── Spec loading ──────────────────────────────────────────────────────────────

/// Loads and deserialises an [`ExperimentSpec`] from `path`, resolving
/// relative `datasets[*].path` entries against the spec file's parent
/// directory.  The `output_dir` field is optional and kept only for backward
/// compatibility with existing spec files; the runner ignores it.
fn load_spec(path: &Path) -> Result<ExperimentSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: ExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));

    for d in &mut spec.datasets {
        d.path = resolve_relative(base, &d.path);
    }
    // Resolve output_dir relative to spec location if present.
    if let Some(ref dir) = spec.output_dir {
        spec.output_dir = Some(if dir.is_absolute() {
            dir.clone()
        } else {
            base.join(dir)
        });
    }
    Ok(spec)
}

/// Resolves `path` relative to `base`, preferring the `base`-joined form when
/// it exists on disk.
fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let candidate = base.join(path);
    if candidate.exists() {
        return candidate;
    }
    if path.exists() {
        return path.to_path_buf();
    }
    candidate
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_run_row(metrics_json: &str) -> RunRow {
        RunRow {
            run_key: "b50d151629d65018abcdef1234567890abcdef1234567890abcdef1234567890ab"
                .to_string(),
            dataset_id: "isdc_s".to_string(),
            dataset_path: "/data/isdc_s.json".to_string(),
            algorithm: "est".to_string(),
            config_slug: "e2-k1-b1-future_flexibility".to_string(),
            identity_json: "{}".to_string(),
            metrics_json: metrics_json.to_string(),
            schedule_json: None,
            created_at: "2024-05-26T10:00:00Z".to_string(),
            last_seen_at: "2024-05-26T10:00:00Z".to_string(),
            source_cell_id: None,
        }
    }

    const FULL_METRICS: &str = r#"{
        "scheduled_priority_sum": 1234.56,
        "scheduled_priority_ratio": 0.8123,
        "scheduled_task_ratio": 0.7000,
        "scheduled_time_ratio": 0.6421,
        "priority_density": 1.1604,
        "scheduler_runtime_ms": 98.2
    }"#;

    #[test]
    fn metric_columns_appear_in_header() {
        let row = make_run_row(FULL_METRICS);
        let out = format_metric_rows(&[row], &[]);
        let header_line = out.lines().next().unwrap();
        assert!(header_line.contains("psum"), "header missing psum");
        assert!(header_line.contains("p_ratio"), "header missing p_ratio");
        assert!(header_line.contains("t_ratio"), "header missing t_ratio");
        assert!(header_line.contains("time_r"), "header missing time_r");
        assert!(header_line.contains("density"), "header missing density");
        assert!(header_line.contains("runtime"), "header missing runtime");
        assert!(
            header_line.contains("created_at"),
            "header missing created_at"
        );
    }

    #[test]
    fn metric_values_appear_in_data_row() {
        let row = make_run_row(FULL_METRICS);
        let out = format_metric_rows(&[row], &[]);
        // Skip header and separator lines; data is on line index 2
        let data_line = out.lines().nth(2).unwrap();
        assert!(data_line.contains("1234.56"), "psum value missing");
        assert!(data_line.contains("0.8123"), "p_ratio value missing");
        assert!(data_line.contains("0.7000"), "t_ratio value missing");
        assert!(data_line.contains("0.6421"), "time_r value missing");
        assert!(data_line.contains("1.1604"), "density value missing");
        assert!(data_line.contains("98.20"), "runtime value missing");
        assert!(data_line.contains("2024-05-26"), "created_at missing");
    }

    #[test]
    fn missing_metrics_show_dash_not_zero() {
        let row = make_run_row("{}"); // no metrics at all
        let out = format_metric_rows(&[row], &[]);
        let data_line = out.lines().nth(2).unwrap();
        // Verify that absent metrics produce "-" rather than defaulting to 0.0.
        assert!(
            !data_line.contains("0.0000"),
            "missing ratio should be '-' not 0.0000: {data_line}"
        );
        assert!(
            !data_line.contains(" 0.00"),
            "missing psum/runtime should be '-' not 0.00: {data_line}"
        );
    }

    #[test]
    fn extra_sort_column_appended_for_non_default_metric() {
        let row = make_run_row(r#"{"utilization": 0.75}"#);
        let sort = vec![parse_sort_key("utilization:desc").unwrap()];
        let out = format_metric_rows(&[row], &sort);
        let header_line = out.lines().next().unwrap();
        assert!(
            header_line.contains("utilization"),
            "extra sort column missing from header: {header_line}"
        );
        let data_line = out.lines().nth(2).unwrap();
        assert!(
            data_line.contains("0.7500"),
            "utilization value missing from data row: {data_line}"
        );
    }

    #[test]
    fn default_sort_metrics_do_not_create_extra_columns() {
        let row = make_run_row(FULL_METRICS);
        // All of these are already in METRIC_DISPLAY_COLS
        let sort = vec![
            parse_sort_key("scheduled_priority_ratio:desc").unwrap(),
            parse_sort_key("runtime_ms:asc").unwrap(),
        ];
        let out = format_metric_rows(&[row], &sort);
        // Header should have exactly one occurrence of "p_ratio" and "runtime"
        let header_line = out.lines().next().unwrap();
        assert_eq!(header_line.matches("p_ratio").count(), 1);
        assert_eq!(header_line.matches("runtime").count(), 1);
    }

    #[test]
    fn fmt_f64_col_right_aligns_to_width() {
        let s = fmt_f64_col(Some(1.5), 8, 4);
        assert_eq!(s.len(), 8, "unexpected len for '{s}'");
        assert!(s.starts_with(' '), "should be right-aligned: '{s}'");
    }

    #[test]
    fn fmt_f64_col_shows_dash_for_none() {
        let s = fmt_f64_col(None, 8, 4);
        assert_eq!(s.len(), 8);
        assert!(s.trim() == "-");
    }
}
