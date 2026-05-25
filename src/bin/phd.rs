//! `phd` — unified user-facing CLI for the PhD scheduling workspace.
//!
//! Phase 1 (foundation) responsibilities:
//! - `phd run` / `phd matrix` / `phd dataset adapt` — dispatch to the
//!   existing sibling binaries (`scheduler`, `experiments`,
//!   `ctao_adapter`) so users only need to remember one entry point.
//! - `phd manifest create` — walk a `run-<ts>/` directory produced by
//!   `experiments` and emit per-cell `manifest.json` files plus a
//!   batch index (`manifest-batch.json`) under
//!   `<run-dir>/cells/<cell_id>/manifest.json`.
//! - `phd manifest validate` — load a manifest and run the structural
//!   validator from [`scheduler::manifest`].
//! - `phd publish` — uploads manifests and (optionally) full schedules
//!   to the webapp `/v1/workspaces/{id}` endpoints with idempotency,
//!   chunked batches and exponential-backoff retries;
//!   wired in Phase 3.
//!
//! Subprocess dispatch uses the sibling binary that lives next to
//! `phd` in the same directory (the cargo `target/.../` layout). When
//! that lookup fails the CLI falls back to the binary name on `$PATH`
//! so users can install only what they need.

#[path = "phd/ranking.rs"]
mod ranking;

use clap::{Parser, Subcommand};
use scheduler::manifest::{
    AlgorithmRef, ArtifactRef, DatasetRef, Horizon, Manifest, Producer, Provenance, RunInfo,
    RunKind, RunStatus, ValidationReport, ValidationStatus, WorkspaceContext,
};
use scheduler::metrics::ScheduleMetrics;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const PHD_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHA: Option<&str> = option_env!("GIT_SHA");

#[derive(Parser, Debug)]
#[command(
    name = "phd",
    version,
    about = "PhD scheduling CLI — run simulations, build manifests, publish to the webapp",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a single scheduling problem (delegates to the `scheduler` binary).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Run {
        /// Forwarded as-is to the `scheduler` binary.
        args: Vec<String>,
    },
    /// Run a sweep / matrix experiment (delegates to `experiments`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Matrix {
        /// Forwarded as-is to the `experiments` binary.
        args: Vec<String>,
    },
    /// Run a sweep and collect flat results — the canonical way to run experiments.
    ///
    /// Executes the experiment defined in SPEC, writes one self-contained
    /// schedule JSON per cell to OUT (flat, no subdirectories), and optionally
    /// emits a companion `<cell_id>.manifest.json` next to each schedule.
    ///
    /// Use this command in preference to `phd matrix` for all routine sweeps.
    Sweep {
        /// Path to the experiment spec JSON (same format as `phd matrix --spec`).
        #[arg(long, value_name = "FILE")]
        spec: PathBuf,
        /// Output directory (created if absent). Schedule files land here flat.
        /// Defaults to `./out`.
        #[arg(long, value_name = "DIR", default_value = "out")]
        out: PathBuf,
        /// Also write `<cell_id>.manifest.json` next to each schedule.
        #[arg(long)]
        manifest: bool,
        /// Override parallelism (threads). Defaults to spec's `max_parallel`.
        #[arg(long, value_name = "N")]
        parallel: Option<usize>,
    },
    /// Dataset utilities.
    Dataset {
        #[command(subcommand)]
        cmd: DatasetCmd,
    },
    /// Manifest operations (build / validate / inspect).
    Manifest {
        #[command(subcommand)]
        cmd: ManifestCmd,
    },
    /// Publish manifest(s) to a webapp workspace. (Phase 3 — not yet implemented.)
    Publish(PublishArgs),
}

#[derive(Subcommand, Debug)]
enum DatasetCmd {
    /// CTA-O dataset adapter (delegates to `ctao_adapter`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Adapt { args: Vec<String> },
}

#[derive(Subcommand, Debug)]
enum ManifestCmd {
    /// Build manifest(s) from a `experiments` run directory or a single schedule JSON.
    ///
    /// Use `--run <DIR>` to build manifests for all cells in an experiment run, or
    /// `--schedule <FILE>` to build a manifest from a single self-contained schedule JSON.
    Create {
        /// Path to the `run-<ts>/` directory produced by `phd matrix` or `phd sweep`.
        /// Conflicts with `--schedule`.
        #[arg(long, value_name = "DIR", conflicts_with = "schedule")]
        run: Option<PathBuf>,
        /// Path to a single schedule JSON (must contain embedded `schedule_metadata` and
        /// `schedule_metrics`). Conflicts with `--run`.
        #[arg(long, value_name = "FILE", conflicts_with = "run")]
        schedule: Option<PathBuf>,
        /// Optional output path.
        ///  - With `--run`:      output directory (defaults to `<run>/cells`).
        ///  - With `--schedule`: output file (defaults to stdout).
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Skip cells whose manifest already exists (only applies with `--run`).
        #[arg(long)]
        skip_existing: bool,
    },
    /// Validate a manifest against the structural rules + schema invariants.
    Validate {
        /// Path to a `manifest.json` file.
        path: PathBuf,
    },
    /// Read all `*.manifest.json` files in DIR and write a flat CSV to OUT.
    Summarize {
        /// Directory containing `*.manifest.json` files (searched recursively).
        #[arg(long, value_name = "DIR")]
        dir: PathBuf,
        /// Output CSV file path (default: `<dir>/summary.csv`).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
}

#[derive(Parser, Debug)]
struct PublishArgs {
    /// Target workspace id (slug). Created if `--create-workspace` is set.
    #[arg(long)]
    workspace: String,
    /// Publish a single manifest file.
    #[arg(long, value_name = "FILE", conflicts_with = "dir")]
    pub manifest: Option<PathBuf>,
    /// Publish every artifact under DIR (recursive). Files are
    /// classified by content: manifests go to the manifests/batch
    /// endpoint and self-contained schedules to schedules/batch.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// When publishing from a directory, also upload self-contained
    /// schedules so the webapp persists them for drill-down.
    /// Set to `false` to upload manifests only.
    #[arg(long = "include-schedules", default_value_t = true, action = clap::ArgAction::Set)]
    include_schedules: bool,
    /// Webapp base URL (default: http://localhost:8080 or $PHD_WEBAPP_URL).
    #[arg(long, value_name = "URL")]
    url: Option<String>,
    /// Optional bearer token (or set $PHD_WEBAPP_TOKEN).
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Create the workspace if it doesn't exist.
    #[arg(long = "create-workspace")]
    create_workspace: bool,
    /// Workspace display name when creating (defaults to id).
    #[arg(long = "workspace-name")]
    workspace_name: Option<String>,
    /// Maximum retry attempts for transient failures.
    #[arg(long, default_value_t = 3)]
    retries: u32,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("phd: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: Cmd) -> Result<ExitCode, String> {
    match cmd {
        Cmd::Run { args } => exec_sibling("scheduler", &args),
        Cmd::Matrix { args } => exec_sibling("experiments", &args),
        Cmd::Sweep {
            spec,
            out,
            manifest,
            parallel,
        } => sweep(&spec, &out, manifest, parallel),
        Cmd::Dataset {
            cmd: DatasetCmd::Adapt { args },
        } => exec_sibling("ctao_adapter", &args),
        Cmd::Manifest {
            cmd:
                ManifestCmd::Create {
                    run,
                    schedule,
                    out,
                    skip_existing,
                },
        } => match (run, schedule) {
            (Some(run_dir), None) => manifest_create(&run_dir, out.as_deref(), skip_existing),
            (None, Some(sched_file)) => manifest_create_from_schedule(&sched_file, out.as_deref()),
            _ => Err("phd manifest create: specify exactly one of --run or --schedule".to_string()),
        },
        Cmd::Manifest {
            cmd: ManifestCmd::Validate { path },
        } => manifest_validate(&path),
        Cmd::Manifest {
            cmd: ManifestCmd::Summarize { dir, out },
        } => manifest_summarize(&dir, out.as_deref()),
        Cmd::Publish(args) => publish(args),
    }
}

// ---------------------------------------------------------------------------
// Sibling binary dispatch
// ---------------------------------------------------------------------------

fn exec_sibling(name: &str, args: &[String]) -> Result<ExitCode, String> {
    let exe = locate_sibling(name);
    let status = Command::new(&exe)
        .args(args)
        .status()
        .map_err(|e| format!("failed to spawn `{}`: {e}", exe.display()))?;
    Ok(if let Some(code) = status.code() {
        if code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from((code & 0xff) as u8)
        }
    } else {
        ExitCode::FAILURE
    })
}

fn locate_sibling(name: &str) -> PathBuf {
    if let Ok(current) = std::env::current_exe()
        && let Some(dir) = current.parent()
    {
        let candidate = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        });
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(name)
}

// ---------------------------------------------------------------------------
// `phd sweep`
// ---------------------------------------------------------------------------

fn sweep(
    spec_path: &Path,
    out_dir: &Path,
    emit_manifest: bool,
    parallel_override: Option<usize>,
) -> Result<ExitCode, String> {
    if !spec_path.is_file() {
        return Err(format!("spec file `{}` not found", spec_path.display()));
    }

    // Build a temp output dir for the experiments run.
    let tmp = tempfile::TempDir::new().map_err(|e| format!("failed to create temp dir: {e}"))?;
    let tmp_out = tmp.path().join("sweep_run");
    fs::create_dir_all(&tmp_out).map_err(|e| format!("failed to create temp output dir: {e}"))?;

    let spec_for_run: PathBuf =
        patch_spec_for_run(spec_path, tmp.path(), &tmp_out, parallel_override)?;

    // Run experiments without state.jsonl — progress goes to stderr.
    let status = Command::new(locate_sibling("experiments"))
        .arg("run")
        .arg("--spec")
        .arg(&spec_for_run)
        .arg("--no-state")
        .status()
        .map_err(|e| format!("failed to spawn experiments: {e}"))?;
    if !status.success() {
        return Err("experiments run failed".to_string());
    }

    // Find the single run-<ts> directory experiments created.
    let run_dir = find_single_run_dir(&tmp_out)
        .ok_or_else(|| format!("could not locate run dir under {}", tmp_out.display()))?;

    // Ensure the flat output directory exists.
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create output dir {}: {e}", out_dir.display()))?;

    // Read the experiment manifest for manifest generation.
    let exp_path = run_dir.join("experiment.json");
    let exp_text = fs::read_to_string(&exp_path)
        .map_err(|e| format!("failed to read experiment.json: {e}"))?;
    let exp: ExperimentManifestFile = serde_json::from_str(&exp_text)
        .map_err(|e| format!("failed to parse experiment.json: {e}"))?;
    let dataset_lookup: std::collections::HashMap<&str, &DatasetSpecLite> = exp
        .spec
        .datasets
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();
    let now = current_rfc3339();
    let run_id = run_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("run")
        .to_string();

    // Flatten: copy schedules to out_dir, optionally write manifests.
    let schedules_dir = run_dir.join("schedules");
    let mut copied = 0usize;
    let mut manifests_written = 0usize;
    let mut run_records = Vec::new();

    for cell in &exp.cells {
        let src = schedules_dir.join(format!("{}.json", cell.cell_id));
        if !src.exists() {
            eprintln!("phd sweep: warning: missing schedule for {}", cell.cell_id);
            continue;
        }
        let dst = out_dir.join(format!("{}.json", cell.cell_id));
        fs::copy(&src, &dst)
            .map_err(|e| format!("failed to copy {} -> {}: {e}", src.display(), dst.display()))?;
        copied += 1;

        let mut manifest_path_for_record = None;
        if emit_manifest {
            match build_cell_manifest(
                &run_dir,
                &exp.spec.name,
                &run_id,
                &now,
                &dataset_lookup,
                cell,
            ) {
                Ok(m) => {
                    let manifest_path = out_dir.join(format!("{}.manifest.json", cell.cell_id));
                    let text = serde_json::to_string_pretty(&m)
                        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
                    fs::write(&manifest_path, &text)
                        .map_err(|e| format!("failed to write {}: {e}", manifest_path.display()))?;
                    manifests_written += 1;
                    manifest_path_for_record = Some(manifest_path);
                }
                Err(e) => {
                    eprintln!(
                        "phd sweep: warning: manifest for `{}` failed: {e}",
                        cell.cell_id
                    );
                }
            }
        }

        let record = ranking::record_from_schedule(
            &cell.cell_id,
            &cell.dataset_id,
            &cell.algorithm,
            &cell.run_config,
            &dst,
            manifest_path_for_record,
        )?;
        run_records.push(record);
    }

    let artifacts = ranking::write_analysis_outputs(out_dir, &run_records)?;

    println!(
        "phd sweep: {} schedule(s) written to {}{}",
        copied,
        out_dir.display(),
        if emit_manifest {
            format!(" + {manifests_written} manifest(s)")
        } else {
            String::new()
        }
    );
    println!(
        "phd sweep: ranking outputs → {}, {}, {}, {}, {}",
        artifacts.all_runs_csv.display(),
        artifacts.rankings_by_dataset_csv.display(),
        artifacts.summary_by_config_csv.display(),
        artifacts.pareto_front_csv.display(),
        artifacts.best_schedules_dir.display()
    );
    if emit_manifest && manifests_written > 0 {
        let summary_path = out_dir.join("summary.csv");
        match manifest_summarize(out_dir, Some(&summary_path)) {
            Ok(_) => {}
            Err(e) => eprintln!("phd sweep: warning: failed to write summary.csv: {e}"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn patch_spec_for_run(
    spec_path: &Path,
    tmp_dir: &Path,
    tmp_out: &Path,
    parallel_override: Option<usize>,
) -> Result<PathBuf, String> {
    let text = fs::read_to_string(spec_path).map_err(|e| format!("failed to read spec: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("failed to parse spec: {e}"))?;
    if let Some(obj) = v.as_object_mut() {
        if let Some(n) = parallel_override {
            obj.insert("max_parallel".to_string(), serde_json::json!(n));
        }
        obj.insert("output_dir".to_string(), serde_json::json!(tmp_out));
        let base = spec_path.parent().unwrap_or_else(|| Path::new("."));
        resolve_dataset_paths(obj, base);
    }

    let patched_path = tmp_dir.join("spec_patched.json");
    let mut f = fs::File::create(&patched_path)
        .map_err(|e| format!("failed to write patched spec: {e}"))?;
    let patched = serde_json::to_string_pretty(&v)
        .map_err(|e| format!("failed to serialize patched spec: {e}"))?;
    f.write_all(patched.as_bytes())
        .map_err(|e| format!("failed to write patched spec: {e}"))?;
    Ok(patched_path)
}

fn resolve_dataset_paths(obj: &mut serde_json::Map<String, serde_json::Value>, base: &Path) {
    if let Some(serde_json::Value::Array(datasets)) = obj.get_mut("datasets") {
        for dataset in datasets.iter_mut() {
            if let Some(dataset_obj) = dataset.as_object_mut()
                && let Some(serde_json::Value::String(path_str)) = dataset_obj.get_mut("path")
            {
                let path = Path::new(path_str);
                if path.is_absolute() {
                    continue;
                }
                let resolved = resolve_relative(base, path);
                *path_str = resolved.to_string_lossy().into_owned();
            }
        }
    }
}

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

/// Find the single `run-<ts>/` directory inside `<out>/<exp_slug>/`.
fn find_single_run_dir(tmp_out: &Path) -> Option<PathBuf> {
    // experiments creates <tmp_out>/<exp_slug>/run-<ts>/
    let exp_dirs: Vec<PathBuf> = fs::read_dir(tmp_out)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    for exp_dir in &exp_dirs {
        let run_dirs: Vec<PathBuf> = fs::read_dir(exp_dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        if let Some(run_dir) = run_dirs.into_iter().next() {
            return Some(run_dir);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// `phd manifest create`
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExperimentManifestFile {
    spec: ExperimentSpecLite,
    cells: Vec<MatrixCellLite>,
}

#[derive(Debug, Deserialize)]
struct ExperimentSpecLite {
    name: String,
    #[serde(default)]
    datasets: Vec<DatasetSpecLite>,
}

#[derive(Debug, Deserialize)]
struct DatasetSpecLite {
    id: String,
    #[serde(default)]
    label: Option<String>,
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct MatrixCellLite {
    cell_id: String,
    dataset_id: String,
    dataset_path: PathBuf,
    #[serde(default)]
    dataset_label: Option<String>,
    algorithm: String,
    run_config: serde_json::Value,
}

fn manifest_create(
    run_dir: &Path,
    out_override: Option<&Path>,
    skip_existing: bool,
) -> Result<ExitCode, String> {
    if !run_dir.is_dir() {
        return Err(format!("run dir `{}` does not exist", run_dir.display()));
    }
    let exp_path = run_dir.join("experiment.json");
    let exp_text = fs::read_to_string(&exp_path)
        .map_err(|e| format!("failed to read {}: {e}", exp_path.display()))?;
    let exp: ExperimentManifestFile = serde_json::from_str(&exp_text)
        .map_err(|e| format!("failed to parse {}: {e}", exp_path.display()))?;

    let out_root = out_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| run_dir.join("cells"));
    fs::create_dir_all(&out_root)
        .map_err(|e| format!("failed to create {}: {e}", out_root.display()))?;

    let now = current_rfc3339();
    let run_id = run_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("run")
        .to_string();

    let dataset_lookup: std::collections::HashMap<&str, &DatasetSpecLite> = exp
        .spec
        .datasets
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();

    let mut emitted: Vec<(String, PathBuf, String)> = Vec::new();
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for cell in &exp.cells {
        let cell_dir = out_root.join(&cell.cell_id);
        let manifest_path = cell_dir.join("manifest.json");
        if skip_existing && manifest_path.exists() {
            skipped += 1;
            continue;
        }

        match build_cell_manifest(
            run_dir,
            &exp.spec.name,
            &run_id,
            &now,
            &dataset_lookup,
            cell,
        ) {
            Ok(m) => {
                fs::create_dir_all(&cell_dir)
                    .map_err(|e| format!("failed to create {}: {e}", cell_dir.display()))?;
                let text = serde_json::to_string_pretty(&m)
                    .map_err(|e| format!("failed to serialize manifest: {e}"))?;
                fs::write(&manifest_path, &text)
                    .map_err(|e| format!("failed to write {}: {e}", manifest_path.display()))?;
                emitted.push((cell.cell_id.clone(), manifest_path, m.manifest_id));
            }
            Err(e) => {
                eprintln!("phd: warning: skipping cell `{}`: {e}", cell.cell_id);
                failed += 1;
            }
        }
    }

    // Batch index next to the experiment manifest.
    let batch_path = run_dir.join("manifest-batch.json");
    let batch = serde_json::json!({
        "schema": "phd-manifest-batch/1",
        "batch_id": uuid::Uuid::new_v4().to_string(),
        "run_id": run_id,
        "experiment": exp.spec.name,
        "created_at": now,
        "manifests": emitted
            .iter()
            .map(|(cid, path, mid)| serde_json::json!({
                "cell_id": cid,
                "manifest_id": mid,
                "path": path.strip_prefix(run_dir).unwrap_or(path).display().to_string(),
            }))
            .collect::<Vec<_>>(),
    });
    fs::write(
        &batch_path,
        serde_json::to_string_pretty(&batch)
            .map_err(|e| format!("failed to serialize batch index: {e}"))?,
    )
    .map_err(|e| format!("failed to write {}: {e}", batch_path.display()))?;

    println!(
        "phd manifest create: {} written, {} skipped, {} failed → {}",
        emitted.len(),
        skipped,
        failed,
        out_root.display()
    );
    println!("batch index → {}", batch_path.display());

    if failed > 0 && emitted.is_empty() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

// ---------------------------------------------------------------------------
// `phd manifest create --schedule`
// ---------------------------------------------------------------------------

fn manifest_create_from_schedule(
    schedule_path: &Path,
    out: Option<&Path>,
) -> Result<ExitCode, String> {
    let text = fs::read_to_string(schedule_path)
        .map_err(|e| format!("failed to read {}: {e}", schedule_path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", schedule_path.display()))?;

    // Extract schedule_metadata.
    let meta = doc
        .get("schedule_metadata")
        .ok_or_else(|| "schedule JSON has no `schedule_metadata` field — run with `phd sweep --manifest` or `phd matrix` first".to_string())?;

    let algorithm = meta
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let algorithm_config = meta
        .get("algorithm_config")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let dataset_id = meta
        .get("dataset_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let dataset_label = meta
        .get("dataset_label")
        .and_then(|v| v.as_str())
        .unwrap_or(&dataset_id)
        .to_string();

    let (start_mjd, end_mjd) = if let Some(period) = meta.get("period") {
        let s = period
            .get("start_mjd_utc")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let e = period
            .get("end_mjd_utc")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        (s, e)
    } else {
        (0.0, 0.0)
    };

    // Extract schedule_metrics.
    let metrics: ScheduleMetrics = if let Some(m) = doc.get("schedule_metrics") {
        serde_json::from_value(m.clone())
            .map_err(|e| format!("failed to parse `schedule_metrics`: {e}"))?
    } else {
        return Err("schedule JSON has no `schedule_metrics` field — rebuild schedule with `phd sweep` or `phd matrix` (recent version)".to_string());
    };

    let now = current_rfc3339();
    let schedule_sha = sha256_of(schedule_path).unwrap_or_else(|_| "0".repeat(64));
    let schedule_size = fs::metadata(schedule_path).map(|m| m.len()).unwrap_or(0);

    let observatory_id = meta
        .get("observatory_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let ws_ctx = WorkspaceContext {
        observatory_id,
        period: Some(Horizon {
            start_mjd_utc: start_mjd,
            end_mjd_utc: end_mjd,
        }),
        block_pool_hash: meta
            .get("block_pool_hash")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        block_count: meta.get("block_count").and_then(|v| v.as_u64()),
    };
    let extensions = workspace_context_to_extensions(ws_ctx);

    let manifest = Manifest {
        manifest_schema_version: scheduler::manifest::MANIFEST_SCHEMA_VERSION.to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        created_at: now.clone(),
        producer: Producer {
            name: "phd".to_string(),
            version: PHD_VERSION.to_string(),
            git_sha: GIT_SHA.map(str::to_string),
            host: hostname(),
        },
        dataset: DatasetRef {
            id: dataset_id,
            name: dataset_label,
            source_path: String::new(),
            sha256: String::new(),
            schema_version: "scheduling_problem/1".to_string(),
        },
        algorithm: AlgorithmRef {
            id: algorithm.clone(),
            label: algorithm.to_uppercase(),
            version: PHD_VERSION.to_string(),
            config: algorithm_config,
        },
        run: RunInfo {
            run_id: "standalone".to_string(),
            kind: RunKind::MatrixCell,
            started_at: now.clone(),
            finished_at: now.clone(),
            status: RunStatus::Completed,
            exit_code: 0,
        },
        horizon: Horizon {
            start_mjd_utc: start_mjd,
            end_mjd_utc: end_mjd,
        },
        metrics,
        artifacts: scheduler::manifest::Artifacts {
            schedule: Some(ArtifactRef {
                uri: file_uri(schedule_path),
                size_bytes: schedule_size,
                sha256: schedule_sha,
                media_type: "application/json".to_string(),
            }),
            trace: None,
            problem: None,
        },
        links: scheduler::manifest::Links::default(),
        provenance: Provenance {
            matrix_run_id: None,
            cell_id: None,
            parent_manifest: None,
            repo_root: None,
            cli_args: std::env::args().collect(),
        },
        validation: ValidationReport {
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        },
        extensions,
    };

    let serialized = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;

    if let Some(out_path) = out {
        fs::write(out_path, &serialized)
            .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
        println!("phd manifest create: → {}", out_path.display());
    } else {
        println!("{serialized}");
    }
    Ok(ExitCode::SUCCESS)
}

fn build_cell_manifest(
    run_dir: &Path,
    experiment_name: &str,
    run_id: &str,
    now: &str,
    dataset_lookup: &std::collections::HashMap<&str, &DatasetSpecLite>,
    cell: &MatrixCellLite,
) -> Result<Manifest, String> {
    let schedule_path = run_dir
        .join("schedules")
        .join(format!("{}.json", cell.cell_id));

    let schedule_text = fs::read_to_string(&schedule_path)
        .map_err(|e| format!("missing schedule ({}): {e}", schedule_path.display()))?;
    let schedule_doc: serde_json::Value = serde_json::from_str(&schedule_text)
        .map_err(|e| format!("invalid schedule JSON in {}: {e}", schedule_path.display()))?;

    let metrics: ScheduleMetrics = schedule_doc
        .get("schedule_metrics")
        .ok_or_else(|| {
            format!(
                "schedule `{}` has no `schedule_metrics` field",
                schedule_path.display()
            )
        })
        .and_then(|v| {
            serde_json::from_value(v.clone()).map_err(|e| {
                format!(
                    "invalid `schedule_metrics` in {}: {e}",
                    schedule_path.display()
                )
            })
        })?;

    let dataset_meta = dataset_lookup.get(cell.dataset_id.as_str());
    let dataset_path: PathBuf = dataset_meta
        .map(|d| d.path.clone())
        .unwrap_or_else(|| cell.dataset_path.clone());
    let dataset_sha = sha256_of(&dataset_path).unwrap_or_else(|_| "0".repeat(64));
    let dataset_name = dataset_meta
        .and_then(|d| d.label.clone())
        .unwrap_or_else(|| {
            cell.dataset_label
                .clone()
                .unwrap_or_else(|| cell.dataset_id.clone())
        });

    let schedule_artifact = if schedule_path.exists() {
        let bytes = fs::metadata(&schedule_path).map(|m| m.len()).unwrap_or(0);
        let sha = sha256_of(&schedule_path).unwrap_or_else(|_| "0".repeat(64));
        Some(ArtifactRef {
            uri: file_uri(&schedule_path),
            size_bytes: bytes,
            sha256: sha,
            media_type: "application/json".to_string(),
        })
    } else {
        None
    };

    let horizon = horizon_from_schedule_metadata(&schedule_doc)
        .unwrap_or_else(|| horizon_from_metrics(&metrics));
    let ws_ctx = derive_workspace_context_from_dataset(&dataset_path, horizon);
    let extensions = workspace_context_to_extensions(ws_ctx);

    let manifest = Manifest {
        manifest_schema_version: scheduler::manifest::MANIFEST_SCHEMA_VERSION.to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        created_at: now.to_string(),
        producer: Producer {
            name: "phd".to_string(),
            version: PHD_VERSION.to_string(),
            git_sha: GIT_SHA.map(str::to_string),
            host: hostname(),
        },
        dataset: DatasetRef {
            id: cell.dataset_id.clone(),
            name: dataset_name,
            source_path: dataset_path.display().to_string(),
            sha256: dataset_sha,
            schema_version: "scheduling_problem/1".to_string(),
        },
        algorithm: AlgorithmRef {
            id: cell.algorithm.clone(),
            label: cell.algorithm.to_uppercase(),
            version: PHD_VERSION.to_string(),
            config: cell.run_config.clone(),
        },
        run: RunInfo {
            run_id: run_id.to_string(),
            kind: RunKind::MatrixCell,
            started_at: now.to_string(),
            finished_at: now.to_string(),
            status: RunStatus::Completed,
            exit_code: 0,
        },
        horizon,
        metrics,
        artifacts: scheduler::manifest::Artifacts {
            schedule: schedule_artifact,
            trace: None,
            problem: None,
        },
        links: scheduler::manifest::Links::default(),
        provenance: Provenance {
            matrix_run_id: Some(run_id.to_string()),
            cell_id: Some(cell.cell_id.clone()),
            parent_manifest: None,
            repo_root: None,
            cli_args: std::env::args().collect(),
        },
        validation: ValidationReport {
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        },
        extensions,
    };
    let _ = experiment_name; // currently unused but reserved for future provenance fields.
    Ok(manifest)
}

/// Read a scheduling-problem JSON file and derive a [`WorkspaceContext`].
///
/// Returns a context with `block_pool_hash` (sha256 over sorted block ids),
/// `block_count`, and `observatory_id` when those fields can be parsed
/// without typed deserialization. `period` is set to `horizon` so cohort
/// keys align with the manifest horizon. On any parse error returns a
/// minimal context populated only with `period`.
fn derive_workspace_context_from_dataset(
    dataset_path: &Path,
    horizon: Horizon,
) -> WorkspaceContext {
    let mut ctx = WorkspaceContext {
        period: Some(horizon),
        ..Default::default()
    };
    let Ok(text) = fs::read_to_string(dataset_path) else {
        return ctx;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return ctx;
    };

    if let Some(blocks) = value.get("scheduling_blocks").and_then(|v| v.as_array()) {
        let mut ids: Vec<String> = blocks
            .iter()
            .filter_map(|b| {
                b.get("id").and_then(|id| match id {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
            })
            .collect();
        ids.sort();
        if !ids.is_empty() {
            let mut hasher = Sha256::new();
            for id in &ids {
                hasher.update(id.as_bytes());
                hasher.update([0u8]);
            }
            ctx.block_pool_hash = Some(format!("{:x}", hasher.finalize()));
            ctx.block_count = Some(ids.len() as u64);
        }
    }

    if let Some(resources) = value.get("resources").and_then(|v| v.as_array())
        && let Some(name) = resources
            .first()
            .and_then(|r| r.get("name"))
            .and_then(|v| v.as_str())
    {
        ctx.observatory_id = Some(name.to_string());
    }

    ctx
}

fn workspace_context_to_extensions(ctx: WorkspaceContext) -> serde_json::Value {
    if ctx == WorkspaceContext::default() {
        return serde_json::Value::Null;
    }
    serde_json::json!({ "workspace_context": ctx })
}

fn horizon_from_metrics(metrics: &ScheduleMetrics) -> Horizon {
    // The matrix runner does not currently surface horizon MJDs in
    // `metrics/<cell>.json`; we record total_horizon_sec under
    // `start=0, end=duration_in_days` as a stable, lossless placeholder
    // that downstream consumers can recognise (start==0).
    let days = metrics.total_horizon_sec / 86_400.0;
    Horizon {
        start_mjd_utc: 0.0,
        end_mjd_utc: days,
    }
}

/// Extract `schedule_metadata.period.{start,end}_mjd_utc` from a schedule
/// JSON document and turn it into a [`Horizon`].
fn horizon_from_schedule_metadata(schedule_doc: &serde_json::Value) -> Option<Horizon> {
    let period = schedule_doc
        .get("schedule_metadata")
        .and_then(|m| m.get("period"))?;
    let start_mjd_utc = period.get("start_mjd_utc").and_then(|v| v.as_f64())?;
    let end_mjd_utc = period.get("end_mjd_utc").and_then(|v| v.as_f64())?;
    Some(Horizon {
        start_mjd_utc,
        end_mjd_utc,
    })
}

fn manifest_summarize(dir: &Path, out: Option<&Path>) -> Result<ExitCode, String> {
    if !dir.is_dir() {
        return Err(format!("directory `{}` does not exist", dir.display()));
    }

    let manifest_files = collect_manifest_files(dir)?;
    if manifest_files.is_empty() {
        return Err(format!(
            "no *.manifest.json files found under `{}`",
            dir.display()
        ));
    }

    let out_path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join("summary.csv"));

    let header = "manifest_id,cell_id,dataset_id,algorithm_id,config_json,schedule_path,\
horizon_start_mjd_utc,horizon_end_mjd_utc,scheduled_task_count,total_task_count,\
scheduled_task_ratio,scheduled_priority_sum,total_priority_sum,scheduled_priority_ratio,\
priority_density,scheduled_priority_mean,scheduled_priority_p50,scheduled_priority_p90,\
scheduled_time_sec,utilization,fragmentation_index,composite_rank_score";

    let mut rows: Vec<String> = Vec::new();

    for mf_path in &manifest_files {
        let text = match fs::read_to_string(mf_path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "phd manifest summarize: skipping {}: {e}",
                    mf_path.display()
                );
                continue;
            }
        };
        let m: Manifest = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(
                    "phd manifest summarize: skipping {}: {e}",
                    mf_path.display()
                );
                continue;
            }
        };

        let cell_id = m.provenance.cell_id.as_deref().unwrap_or("").to_string();
        let schedule_path = m
            .artifacts
            .schedule
            .as_ref()
            .map(|a| a.uri.clone())
            .unwrap_or_default();
        let config_json =
            serde_json::to_string(&m.algorithm.config).unwrap_or_else(|_| "{}".to_string());
        let config_csv = csv_field(&config_json);

        let mx = &m.metrics;
        let row = format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_field(&m.manifest_id),
            csv_field(&cell_id),
            csv_field(&m.dataset.id),
            csv_field(&m.algorithm.id),
            config_csv,
            csv_field(&schedule_path),
            m.horizon.start_mjd_utc,
            m.horizon.end_mjd_utc,
            mx.scheduled_task_count,
            mx.total_task_count,
            mx.scheduled_task_ratio,
            mx.scheduled_priority_sum,
            mx.total_priority_sum,
            mx.scheduled_priority_ratio,
            mx.priority_density,
            mx.scheduled_priority.mean,
            mx.scheduled_priority.p50,
            mx.scheduled_priority.p90,
            mx.scheduled_time_sec,
            mx.utilization,
            mx.fragmentation.fragmentation_index,
            mx.composite_rank_score,
        );
        rows.push(row);
    }

    let csv_content = format!("{}\n{}\n", header, rows.join("\n"));
    fs::write(&out_path, &csv_content)
        .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;

    println!(
        "phd manifest summarize: {} row(s) → {}",
        rows.len(),
        out_path.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn collect_manifest_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    collect_manifest_files_inner(dir, &mut result)
        .map_err(|e| format!("failed to read directory: {e}"))?;
    result.sort();
    Ok(result)
}

fn collect_manifest_files_inner(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_manifest_files_inner(&path, out)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".manifest.json"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `phd manifest validate`
// ---------------------------------------------------------------------------

fn manifest_validate(path: &Path) -> Result<ExitCode, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| format!("invalid manifest JSON {}: {e}", path.display()))?;
    let report = manifest.validate();
    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("failed to serialize report: {e}"))?;
    println!("{json}");
    Ok(match report.status {
        ValidationStatus::Valid | ValidationStatus::Warning => ExitCode::SUCCESS,
        ValidationStatus::Invalid => ExitCode::FAILURE,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256_of(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_uri(path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", abs.display())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty())
}

fn current_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// `phd publish`
// ---------------------------------------------------------------------------

fn publish(args: PublishArgs) -> Result<ExitCode, String> {
    if args.manifest.is_none() && args.dir.is_none() {
        return Err("either --manifest <FILE> or --dir <DIR> is required".into());
    }
    let base = args
        .url
        .clone()
        .or_else(|| std::env::var("PHD_WEBAPP_URL").ok())
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let base = base.trim_end_matches('/').to_string();
    let token = args
        .token
        .clone()
        .or_else(|| std::env::var("PHD_WEBAPP_TOKEN").ok());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build();

    if args.create_workspace {
        let name = args
            .workspace_name
            .clone()
            .unwrap_or_else(|| args.workspace.clone());
        match create_workspace_call(&agent, &base, token.as_deref(), &args.workspace, &name) {
            Ok(()) => println!("workspace `{}` ready", args.workspace),
            Err(PublishError::Conflict) => {
                println!("workspace `{}` already exists, reusing", args.workspace);
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    // Single-manifest path: simplest case, kept verbatim.
    if let Some(single) = &args.manifest {
        let path = single.as_path();
        match publish_one(
            &agent,
            &base,
            token.as_deref(),
            &args.workspace,
            path,
            args.retries,
        ) {
            Ok(true) => println!("  + {}  (created)", path.display()),
            Ok(false) => println!("  = {}  (already present)", path.display()),
            Err(e) => {
                eprintln!("  ! {}: {e}", path.display());
                return Ok(ExitCode::FAILURE);
            }
        }
        println!(
            "phd publish: 1 manifest → {} (workspace `{}`)",
            base, args.workspace
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Directory path: classify each .json as manifest or schedule.
    let dir = args.dir.as_deref().unwrap();
    let (manifest_paths, schedule_paths) = collect_dir_paths(dir, args.include_schedules)?;
    if manifest_paths.is_empty() && schedule_paths.is_empty() {
        return Err(format!(
            "no manifest or schedule files found under `{}`",
            dir.display()
        ));
    }
    println!(
        "phd publish: detected {} manifest(s) and {} schedule(s) under `{}`",
        manifest_paths.len(),
        schedule_paths.len(),
        dir.display()
    );

    const CHUNK_SIZE: usize = 100;
    const SCHEDULE_CHUNK_SIZE: usize = 25;
    let mut total_created = 0usize;
    let mut total_deduped = 0usize;
    let mut total_failed = 0usize;

    // Manifests first: cheap and useful even if schedules fail later.
    if !manifest_paths.is_empty() {
        let total = manifest_paths.len();
        let mut done = 0usize;
        for chunk in manifest_paths.chunks(CHUNK_SIZE) {
            match publish_batch_chunk(
                &agent,
                &base,
                token.as_deref(),
                &args.workspace,
                chunk,
                args.retries,
            ) {
                Ok((c, d, f)) => {
                    total_created += c;
                    total_deduped += d;
                    total_failed += f;
                }
                Err(e) => {
                    eprintln!("  ! manifest batch failed: {e}");
                    total_failed += chunk.len();
                }
            }
            done += chunk.len();
            eprintln!("[{done}/{total}] manifests uploaded");
        }
    }

    // Schedules next: heavier payload, smaller chunks.
    if !schedule_paths.is_empty() {
        let total = schedule_paths.len();
        let mut done = 0usize;
        for chunk in schedule_paths.chunks(SCHEDULE_CHUNK_SIZE) {
            match publish_schedules_batch_chunk(
                &agent,
                &base,
                token.as_deref(),
                &args.workspace,
                chunk,
                args.retries,
            ) {
                Ok((c, d, f)) => {
                    total_created += c;
                    total_deduped += d;
                    total_failed += f;
                }
                Err(e) => {
                    eprintln!("  ! schedule batch failed: {e}");
                    total_failed += chunk.len();
                }
            }
            done += chunk.len();
            eprintln!("[{done}/{total}] schedules uploaded");
        }
    }

    println!(
        "phd publish: {} created, {} deduplicated, {} failed → {} (workspace `{}`)",
        total_created, total_deduped, total_failed, base, args.workspace
    );
    Ok(if total_failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[derive(Debug)]
enum PublishError {
    Http(u16, String),
    Conflict,
    Transport(String),
    Body(String),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(s, m) => write!(f, "HTTP {s}: {m}"),
            Self::Conflict => write!(f, "conflict"),
            Self::Transport(m) => write!(f, "transport: {m}"),
            Self::Body(m) => write!(f, "body: {m}"),
        }
    }
}

/// Walk DIR and classify each `.json` file as a manifest or a
/// self-contained schedule by reading it. Files that are neither (or
/// invalid JSON) are skipped with a warning. Returns
/// `(manifests, schedules)`. When `include_schedules` is false, the
/// schedule list is left empty.
fn collect_dir_paths(
    dir: &Path,
    include_schedules: bool,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    if !dir.is_dir() {
        return Err(format!("dir `{}` not found", dir.display()));
    }
    let mut all = Vec::new();
    walk_json(dir, &mut all);
    all.sort();

    let mut manifests = Vec::new();
    let mut schedules = Vec::new();
    for path in all {
        match classify_json(&path) {
            JsonKind::Manifest => manifests.push(path),
            JsonKind::Schedule if include_schedules => schedules.push(path),
            JsonKind::Schedule => { /* dropped per --include-schedules=false */ }
            JsonKind::StandaloneMetrics => {
                eprintln!(
                    "  · skipping `{}`: standalone schedule_metrics.json is no longer accepted; \
                     embed metrics inside a manifest or upload the full schedule",
                    path.display()
                );
            }
            JsonKind::Unknown => {
                eprintln!(
                    "  · skipping `{}` (not a manifest nor a self-contained schedule)",
                    path.display()
                );
            }
            JsonKind::Unreadable(e) => {
                eprintln!("  · skipping `{}`: {e}", path.display());
            }
        }
    }
    Ok((manifests, schedules))
}

fn walk_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_json(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(p);
        }
    }
}

enum JsonKind {
    Manifest,
    Schedule,
    StandaloneMetrics,
    Unknown,
    Unreadable(String),
}

fn classify_json(path: &Path) -> JsonKind {
    // Cheap path-name heuristic first; fall back to content inspection.
    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
        if name == "manifest.json" || name.ends_with(".manifest.json") {
            return JsonKind::Manifest;
        }
        if name == "schedule_metrics.json" || name.ends_with(".schedule_metrics.json") {
            return JsonKind::StandaloneMetrics;
        }
    }
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => return JsonKind::Unreadable(e.to_string()),
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return JsonKind::Unreadable(e.to_string()),
    };
    if value.get("manifest_schema_version").is_some() && value.get("manifest_id").is_some() {
        JsonKind::Manifest
    } else if value.get("schedule_metadata").is_some()
        && (value.get("scheduling_blocks").is_some() || value.get("blocks").is_some())
    {
        JsonKind::Schedule
    } else if value.get("scheduled_task_count").is_some()
        && value.get("fragmentation").is_some()
        && value.get("manifest_schema_version").is_none()
    {
        // Bare ScheduleMetrics blob (no envelope).
        JsonKind::StandaloneMetrics
    } else {
        JsonKind::Unknown
    }
}

fn create_workspace_call(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    id: &str,
    name: &str,
) -> Result<(), PublishError> {
    let url = format!("{base}/v1/workspaces");
    let mut req = agent.post(&url);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body = serde_json::json!({ "name": name });
    match req.send_json(body) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(409, _)) => {
            // Slug derived server-side; verify the requested id matches by fetching the ws.
            let _ = id;
            Err(PublishError::Conflict)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(PublishError::Http(code, body))
        }
        Err(e) => Err(PublishError::Transport(e.to_string())),
    }
}

fn publish_one(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    workspace: &str,
    path: &Path,
    retries: u32,
) -> Result<bool, PublishError> {
    let bytes = fs::read(path).map_err(|e| PublishError::Body(e.to_string()))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| PublishError::Body(e.to_string()))?;
    let key = manifest
        .get("manifest_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| sha256_short(&bytes));
    let payload = serde_json::json!({
        "manifest": manifest,
        "idempotency_key": key,
    });
    let url = format!("{base}/v1/workspaces/{workspace}/manifests");

    let mut attempt = 0u32;
    loop {
        let mut req = agent.post(&url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.send_json(payload.clone()) {
            Ok(resp) => {
                let status = resp.status();
                let body: serde_json::Value = resp
                    .into_json()
                    .map_err(|e| PublishError::Body(e.to_string()))?;
                let created = body
                    .get("created")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let _ = status;
                return Ok(created);
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if (500..600).contains(&code) && attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Http(code, body));
            }
            Err(e) => {
                if attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Transport(e.to_string()));
            }
        }
    }
}

/// Upload a batch of manifests to the `/v1/workspaces/{workspace}/manifests/batch`
/// endpoint. Returns `(created, deduped, failed)` counts.
fn publish_batch_chunk(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    workspace: &str,
    paths: &[PathBuf],
    retries: u32,
) -> Result<(usize, usize, usize), PublishError> {
    let mut items: Vec<serde_json::Value> = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(path).map_err(|e| PublishError::Body(e.to_string()))?;
        let manifest: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| PublishError::Body(e.to_string()))?;
        let key = manifest
            .get("manifest_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| sha256_short(&bytes));
        items.push(serde_json::json!({
            "manifest": manifest,
            "idempotency_key": key,
        }));
    }

    let payload = serde_json::json!({ "items": items });
    let url = format!("{base}/v1/workspaces/{workspace}/manifests/batch");

    let mut attempt = 0u32;
    loop {
        let mut req = agent.post(&url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.send_json(payload.clone()) {
            Ok(resp) => {
                let body: serde_json::Value = resp
                    .into_json()
                    .map_err(|e| PublishError::Body(e.to_string()))?;
                let results = body
                    .get("results")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let created = results
                    .iter()
                    .filter(|r| r.get("created").and_then(|v| v.as_bool()).unwrap_or(false))
                    .count();
                let failed = results.iter().filter(|r| r.get("error").is_some()).count();
                let deduped = results.len().saturating_sub(created + failed);
                return Ok((created, deduped, failed));
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if (500..600).contains(&code) && attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Http(code, body));
            }
            Err(e) => {
                if attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Transport(e.to_string()));
            }
        }
    }
}

/// Upload a batch of self-contained schedules to the
/// `/v1/workspaces/{workspace}/schedules/batch` endpoint. The webapp
/// derives a manifest for each one and persists the schedule body so
/// drill-down stays possible.
fn publish_schedules_batch_chunk(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    workspace: &str,
    paths: &[PathBuf],
    retries: u32,
) -> Result<(usize, usize, usize), PublishError> {
    let mut items: Vec<serde_json::Value> = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(path).map_err(|e| PublishError::Body(e.to_string()))?;
        let schedule: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| PublishError::Body(e.to_string()))?;
        items.push(serde_json::json!({
            "schedule": schedule,
            "idempotency_key": sha256_short(&bytes),
        }));
    }
    let payload = serde_json::json!({ "items": items });
    let url = format!("{base}/v1/workspaces/{workspace}/schedules/batch");

    let mut attempt = 0u32;
    loop {
        let mut req = agent.post(&url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.send_json(payload.clone()) {
            Ok(resp) => {
                let body: serde_json::Value = resp
                    .into_json()
                    .map_err(|e| PublishError::Body(e.to_string()))?;
                let summary = body.get("summary").cloned().unwrap_or_default();
                let created = summary.get("created").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let deduped = summary
                    .get("deduplicated")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let failed = summary.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                return Ok((created, deduped, failed));
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if (500..600).contains(&code) && attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Http(code, body));
            }
            Err(e) => {
                if attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Transport(e.to_string()));
            }
        }
    }
}

fn backoff(attempt: u32) -> std::time::Duration {
    let secs = 1u64 << (attempt - 1).min(4);
    std::time::Duration::from_millis(secs * 250)
}

fn sha256_short(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let full = format!("{:x}", h.finalize());
    full[..16].to_string()
}
