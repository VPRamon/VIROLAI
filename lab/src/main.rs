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
//!                 [--resume <existing_run_dir>]
//!                 [--output-dir <dir>]
//!                 [--dry-run]
//!                 [--cache] [--run-db <PATH>]
//! ```
//!
//! Loads an experiment spec, resolves the Cartesian product of cells,
//! and executes them in parallel.  With `--resume` it skips cells
//! already marked `completed` in the existing run's `state.jsonl`.
//! With `--dry-run` it only resolves cells and writes
//! `experiment.json` without running any scheduler.
//! With `--cache` it enables the SQLite registry cache.
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
//! lab registry regenerate --run <KEY_OR_PREFIX> --out <FILE>
//!                         [--run-db <PATH>] [--force]
//! ```

use clap::{Parser, Subcommand};
use lab::cell::resolve_cells;
use lab::output;
use lab::registry::{
    BestOpts, ListOpts, Registry, RunIdentity, RunRow, SortKey, default_sort_keys, parse_sort_key,
    registry_path,
};
use lab::runner::{RunOptions, execute_with_options};
use lab::spec::ExperimentSpec;
use schedulers::metrics::{MetricsContext, ScheduleMetrics};
use schedulers::schedule::{LocationMeta, PeriodMeta, ScheduleMetadata, ScheduleOutput};
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

    /// Resume an existing run directory, skipping already-completed cells.
    #[arg(long, value_name = "DIR")]
    resume: Option<PathBuf>,

    /// Override the output directory declared in the spec (only meaningful
    /// with `--dry-run`).
    #[arg(long, value_name = "DIR")]
    output_dir: Option<PathBuf>,

    /// Resolve cells and write `experiment.json` without executing any
    /// scheduler runs.
    #[arg(long)]
    dry_run: bool,

    /// Skip writing `state.jsonl` entirely; print per-cell progress to stderr
    /// instead.  Incompatible with `--resume`.
    #[arg(long)]
    no_state: bool,

    /// Enable SQLite registry cache: skip cells whose identity already exists
    /// in the registry and insert successful runs after execution.
    #[arg(long)]
    cache: bool,

    /// Path to the SQLite registry file.
    /// Defaults to `.lab/runs.sqlite` when `--cache` is enabled.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,
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
    /// Regenerate a schedule JSON from a stored registry record.
    Regenerate(RegistryRegenerateArgs),
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
struct RegistryRegenerateArgs {
    /// Full run key or unique prefix.
    #[arg(long, value_name = "KEY")]
    run: String,

    /// Output schedule JSON file.
    #[arg(long, value_name = "FILE")]
    out: PathBuf,

    /// Overwrite the output file if it already exists.
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
    if args.no_state && args.resume.is_some() {
        return Err("--no-state and --resume are mutually exclusive".to_string());
    }

    let spec = load_spec(&args.spec)?;
    let cells = resolve_cells(&spec)?;

    if args.dry_run {
        let target = args
            .output_dir
            .as_deref()
            .unwrap_or(&spec.output_dir)
            .to_path_buf();
        let run_dir = output::create_run_dir(&target, &spec.name)?;
        output::write_manifest(&run_dir, &spec, &cells)?;
        println!(
            "[dry-run] resolved {} cells; manifest -> {}",
            cells.len(),
            run_dir.join(output::EXPERIMENT_FILE).display()
        );
        for c in &cells {
            println!("  {}", c.cell_id);
        }
        return Ok(());
    }

    let (run_dir, resume) = if let Some(existing) = args.resume.as_ref() {
        (existing.clone(), true)
    } else {
        let dir = output::create_run_dir(&spec.output_dir, &spec.name)?;
        output::write_manifest(&dir, &spec, &cells)?;
        (dir, false)
    };

    let opts = RunOptions {
        resume,
        no_state_log: args.no_state,
        cache: args.cache,
        run_db: args.run_db.clone(),
    };
    let summary = execute_with_options(&spec, &cells, &run_dir, opts)?;
    println!(
        "lab run done: {} cells total, {} skipped (resume), {} registry hits, {} completed, {} failed",
        summary.total,
        summary.already_done,
        summary.registry_hits,
        summary.completed,
        summary.failed
    );
    println!("artifacts -> {}", summary.run_dir.display());
    if summary.failed > 0 {
        if args.no_state {
            return Err(format!("{} cell(s) failed", summary.failed));
        }
        return Err(format!(
            "{} cell(s) failed; see {}",
            summary.failed,
            run_dir.join(output::STATE_FILE).display()
        ));
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
        RegistryCmd::Regenerate(a) => registry_regenerate(a),
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
        sort,
        limit: args.limit,
    };
    let rows = reg.list(&opts)?;
    print_rows(&rows, &args.format)?;
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
        sort,
        limit: args.limit.or(Some(20)),
        ..Default::default()
    })?;
    print_rows(&rows, &args.format)
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
    println!(
        "{:<18}  {:<8}  {:<30}  metrics",
        "run_key (prefix)", "algo", "config_slug"
    );
    println!("{}", "-".repeat(80));
    for r in &rows {
        let mv: serde_json::Value =
            serde_json::from_str(&r.metrics_json).unwrap_or(serde_json::Value::Null);
        println!(
            "{:<18}  {:<8}  {:<30}  {}",
            &r.run_key[..r.run_key.len().min(16)],
            r.algorithm,
            r.config_slug,
            compact_metric_summary(&mv),
        );
    }
    Ok(())
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
        println!(
            "{:<18}  {:<12}  {:<8}  {:<30}  metrics",
            "run_key (prefix)", "dataset", "algo", "config_slug"
        );
        println!("{}", "-".repeat(110));
        for row in &front {
            let metrics = parse_metrics(&row.metrics_json);
            println!(
                "{:<18}  {:<12}  {:<8}  {:<30}  {}",
                &row.run_key[..row.run_key.len().min(16)],
                row.dataset_id,
                row.algorithm,
                row.config_slug,
                compact_metric_summary(&metrics),
            );
        }
        println!("({} rows on Pareto front)", front.len());
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

// ── `registry regenerate` ─────────────────────────────────────────────────────

fn registry_regenerate(args: RegistryRegenerateArgs) -> Result<(), String> {
    if args.out.exists() && !args.force {
        return Err(format!(
            "output file '{}' already exists; use --force to overwrite",
            args.out.display()
        ));
    }

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

    let identity: RunIdentity = serde_json::from_str(&row.identity_json)
        .map_err(|e| format!("failed to parse stored identity: {e}"))?;

    // Reconstruct run config from stored config_json.
    let run_config: lab::config::RunConfig = serde_json::from_str(&identity.config_json)
        .map_err(|e| format!("failed to parse stored config JSON: {e}"))?;

    // Parse horizon override if present.
    let horizon_override = identity
        .horizon_json
        .as_deref()
        .map(serde_json::from_str::<lab::config::HorizonOverride>)
        .transpose()
        .map_err(|e: serde_json::Error| format!("failed to parse stored horizon: {e}"))?;

    let dataset_path = PathBuf::from(&identity.dataset_path);
    let prepared = lab::problem::prepare_problem(&dataset_path, horizon_override).map_err(|e| {
        format!(
            "failed to prepare dataset '{}': {e}",
            dataset_path.display()
        )
    })?;

    use std::time::Instant;
    let started = Instant::now();
    let schedule = match run_config {
        lab::config::RunConfig::Est(cfg) => {
            let scheduler = cfg.build_scheduler()?;
            scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("EST regenerate failed: {e}"))?
        }
        lab::config::RunConfig::Hap(cfg) => {
            let scheduler = cfg.build_scheduler()?;
            scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("HAP regenerate failed: {e}"))?
        }
    };
    let runtime_ms = started.elapsed().as_secs_f64() * 1000.0;

    let ctx = MetricsContext::new();
    let mut metrics =
        ScheduleMetrics::compute(&schedule, &prepared.problem, &prepared.horizon, &ctx);
    metrics.scheduler_runtime_ms = Some(runtime_ms);
    let metrics_value =
        serde_json::to_value(&metrics).map_err(|e| format!("failed to serialize metrics: {e}"))?;

    let location = prepared.problem.telescope.as_ref().map(|t| LocationMeta {
        name: t.name.clone(),
        longitude_deg: t.location.lon.value(),
        latitude_deg: t.location.lat.value(),
        height_m: t.location.height.value(),
    });
    let period = Some(PeriodMeta {
        start_mjd_utc: prepared.horizon.start.value(),
        end_mjd_utc: prepared.horizon.end.value(),
    });
    let metadata = ScheduleMetadata {
        algorithm: run_config.algorithm().to_string(),
        algorithm_config: serde_json::from_str(&identity.config_json).unwrap_or_default(),
        location,
        period,
        dataset_id: Some(identity.dataset_id.clone()),
        dataset_label: None,
    };
    let output_obj = ScheduleOutput::new(prepared.raw_json.clone(), &schedule, Some(metadata))
        .with_metrics(metrics_value);
    let text = serde_json::to_string_pretty(&output_obj)
        .map_err(|e| format!("failed to serialize schedule: {e}"))?;
    std::fs::write(&args.out, text)
        .map_err(|e| format!("failed to write {}: {e}", args.out.display()))?;
    println!("regenerated -> {}", args.out.display());
    Ok(())
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

fn print_rows(rows: &[RunRow], format: &str) -> Result<(), String> {
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
    println!(
        "{:<18}  {:<12}  {:<8}  {:<30}  created_at",
        "run_key (prefix)", "dataset", "algo", "config_slug"
    );
    println!("{}", "-".repeat(90));
    for row in rows {
        println!(
            "{:<18}  {:<12}  {:<8}  {:<30}  {}",
            &row.run_key[..row.run_key.len().min(16)],
            row.dataset_id,
            row.algorithm,
            row.config_slug,
            row.created_at,
        );
    }
    println!("({} rows)", rows.len());
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

fn compact_metric_summary(mv: &serde_json::Value) -> String {
    format!(
        "priority={:.4} task={:.4} density={:.4} runtime_ms={:.2}",
        metric_value(mv, "scheduled_priority_ratio").unwrap_or(0.0),
        metric_value(mv, "scheduled_task_ratio").unwrap_or(0.0),
        metric_value(mv, "priority_density").unwrap_or(0.0),
        metric_value(mv, "runtime_ms").unwrap_or(0.0),
    )
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
/// relative `datasets[*].path` and `output_dir` entries against the spec
/// file's parent directory.
fn load_spec(path: &Path) -> Result<ExperimentSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: ExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));

    for d in &mut spec.datasets {
        d.path = resolve_relative(base, &d.path);
    }
    spec.output_dir = if spec.output_dir.is_absolute() {
        spec.output_dir.clone()
    } else {
        base.join(&spec.output_dir)
    };
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
