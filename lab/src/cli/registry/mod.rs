//! `lab registry` CLI commands.

mod crud;
mod format;
mod scoring;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub(crate) struct RegistryArgs {
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

    /// Filter by algorithm name (`est`, `lst`, or `hap`).
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

    /// Filter by algorithm name (`est`, `lst`, or `hap`).
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

pub(crate) fn registry(args: RegistryArgs) -> Result<(), String> {
    match args.cmd {
        RegistryCmd::List(a) => crud::list(a),
        RegistryCmd::Sort(a) => crud::sort(a),
        RegistryCmd::Best(a) => crud::best(a),
        RegistryCmd::Rank(a) => crud::rank(a),
        RegistryCmd::Pareto(a) => crud::pareto(a),
        RegistryCmd::Inspect(a) => crud::inspect(a),
        RegistryCmd::Export(a) => crud::export(a),
    }
}
