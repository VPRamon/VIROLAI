//! `virolai` - user-facing CLI for VIROLAI workflows.

mod process;
mod publish;
mod sweep;

use clap::{Parser, Subcommand};
use publish::PublishArgs;
use std::path::PathBuf;
use std::process::ExitCode;

const VIROLAI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "virolai",
    version,
    about = "VIROLAI resource scheduling CLI for experiments and result publishing",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a single scheduling problem (delegates to the `schedulers` binary).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Run {
        /// Forwarded as-is to the `schedulers` binary.
        args: Vec<String>,
    },
    /// Run a sweep / matrix experiment (delegates to `lab`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Matrix {
        /// Forwarded as-is to the `lab` binary.
        args: Vec<String>,
    },
    /// Run a sweep and store results in SQLite.
    Sweep {
        /// Path to the experiment spec JSON (same format as `lab run --spec`).
        #[arg(long, value_name = "FILE")]
        spec: PathBuf,
        /// Path to the registry SQLite database (default: `.lab/runs.sqlite`).
        #[arg(long, value_name = "PATH")]
        run_db: Option<PathBuf>,
        /// Override parallelism (threads). Defaults to spec's `max_parallel`.
        #[arg(long, value_name = "N")]
        parallel: Option<usize>,
        /// Re-execute cells that are already present in the DB and update their row.
        #[arg(long = "override")]
        override_existing: bool,
    },
    /// Dataset utilities and format adapters.
    Dataset {
        #[command(subcommand)]
        cmd: DatasetCmd,
    },
    /// Upload schedule JSONs from a directory to a webapp workspace.
    Publish(PublishArgs),
}

#[derive(Subcommand, Debug)]
enum DatasetCmd {
    /// Run the optional CTAO dataset adapter (delegates to `lab-ctao-adapter`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Adapt { args: Vec<String> },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("virolai: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: Cmd) -> Result<ExitCode, String> {
    match cmd {
        Cmd::Run { args } => process::exec_sibling("schedulers", &args),
        Cmd::Matrix { args } => process::exec_sibling("lab", &args),
        Cmd::Sweep {
            spec,
            run_db,
            parallel,
            override_existing,
        } => sweep::sweep(&spec, run_db.as_deref(), parallel, override_existing),
        Cmd::Dataset {
            cmd: DatasetCmd::Adapt { args },
        } => process::exec_sibling("lab-ctao-adapter", &args),
        Cmd::Publish(args) => publish::publish(args),
    }
}

#[allow(dead_code)]
fn virolai_version() -> &'static str {
    VIROLAI_VERSION
}
