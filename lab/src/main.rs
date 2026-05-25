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
//!                     [--min <VAL>] [--max <VAL>] [--limit <N>]
//!                     [--run-db <PATH>] [--format json]
//! lab registry best   --dataset <ID> [--algorithm <NAME>] [--metric <NAME>]
//!                     [--limit <N>] [--run-db <PATH>]
//! lab registry inspect --run <KEY_OR_PREFIX> [--run-db <PATH>]
//! lab registry regenerate --run <KEY_OR_PREFIX> --out <FILE>
//!                         [--run-db <PATH>] [--force]
//! ```

use clap::{Parser, Subcommand};
use lab::cell::resolve_cells;
use lab::output;
use lab::registry::{BestOpts, ListOpts, Registry, RunIdentity, registry_path};
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
    /// Show the best runs for a dataset.
    Best(RegistryBestArgs),
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

    /// Metric column to filter/sort by.
    #[arg(long, value_name = "NAME")]
    metric: Option<String>,

    /// Minimum metric value (inclusive).
    #[arg(long, value_name = "VAL")]
    min: Option<f64>,

    /// Maximum metric value (inclusive).
    #[arg(long, value_name = "VAL")]
    max: Option<f64>,

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
struct RegistryBestArgs {
    /// Dataset ID to query.
    #[arg(long, value_name = "ID")]
    dataset: String,

    /// Restrict to a single algorithm.
    #[arg(long, value_name = "NAME")]
    algorithm: Option<String>,

    /// Metric to rank by (default: `composite_score`).
    #[arg(long, value_name = "NAME")]
    metric: Option<String>,

    /// Maximum number of results (default: 10).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

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
        RegistryCmd::Best(a) => registry_best(a),
        RegistryCmd::Inspect(a) => registry_inspect(a),
        RegistryCmd::Regenerate(a) => registry_regenerate(a),
    }
}

// ── `registry list` ───────────────────────────────────────────────────────────

fn registry_list(args: RegistryListArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let opts = ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        metric: args.metric,
        min: args.min,
        max: args.max,
        limit: args.limit,
    };
    let rows = reg.list(&opts)?;

    if args.format == "json" {
        let values: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "run_key": r.run_key,
                    "dataset_id": r.dataset_id,
                    "algorithm": r.algorithm,
                    "config_slug": r.config_slug,
                    "created_at": r.created_at,
                    "last_seen_at": r.last_seen_at,
                    "source_cell_id": r.source_cell_id,
                    "metrics": serde_json::from_str::<serde_json::Value>(&r.metrics_json).unwrap_or(serde_json::Value::Null),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
    } else {
        println!(
            "{:<18}  {:<12}  {:<8}  {:<30}  created_at",
            "run_key (prefix)", "dataset", "algo", "config_slug"
        );
        println!("{}", "-".repeat(90));
        for r in &rows {
            println!(
                "{:<18}  {:<12}  {:<8}  {:<30}  {}",
                &r.run_key[..r.run_key.len().min(16)],
                r.dataset_id,
                r.algorithm,
                r.config_slug,
                r.created_at,
            );
        }
        println!("({} rows)", rows.len());
    }
    Ok(())
}

// ── `registry best` ───────────────────────────────────────────────────────────

fn registry_best(args: RegistryBestArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let opts = BestOpts {
        dataset_id: args.dataset,
        algorithm: args.algorithm,
        metric: args.metric.clone(),
        limit: args.limit,
    };
    let rows = reg.best(&opts)?;
    let metric_label = args.metric.as_deref().unwrap_or("composite_score");
    println!(
        "{:<18}  {:<8}  {:<30}  {}",
        "run_key (prefix)", "algo", "config_slug", metric_label
    );
    println!("{}", "-".repeat(80));
    for r in &rows {
        let mv: serde_json::Value =
            serde_json::from_str(&r.metrics_json).unwrap_or(serde_json::Value::Null);
        let val = extract_metric(&mv, metric_label);
        println!(
            "{:<18}  {:<8}  {:<30}  {:.4}",
            &r.run_key[..r.run_key.len().min(16)],
            r.algorithm,
            r.config_slug,
            val,
        );
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

fn extract_metric(mv: &serde_json::Value, metric: &str) -> f64 {
    match metric {
        "task_ratio" | "scheduled_task_ratio" => mv["scheduled_task_ratio"].as_f64().unwrap_or(0.0),
        "priority_ratio" | "scheduled_priority_ratio" => {
            mv["scheduled_priority_ratio"].as_f64().unwrap_or(0.0)
        }
        "priority_density" => mv["priority_density"].as_f64().unwrap_or(0.0),
        "utilization" => mv["utilization"].as_f64().unwrap_or(0.0),
        "fragmentation_index" => mv["fragmentation"]["fragmentation_index"]
            .as_f64()
            .unwrap_or(0.0),
        "runtime_ms" | "scheduler_runtime_ms" => mv["scheduler_runtime_ms"].as_f64().unwrap_or(0.0),
        _ => mv["composite_rank_score"].as_f64().unwrap_or(0.0),
    }
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
