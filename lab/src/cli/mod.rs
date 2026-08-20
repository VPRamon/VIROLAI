//! `lab` binary entry point.
//!
//! The binary executes experiment matrices and queries the SQLite registry.

mod registry;
mod run;

use clap::{Parser, Subcommand};
use registry::RegistryArgs;
use run::RunArgs;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "lab",
    version,
    about = "Run VIROLAI parameter sweeps and query the experiment registry"
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

pub(crate) fn main() -> ExitCode {
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
        Cmd::Run(args) => run::run(args),
        Cmd::Registry(args) => registry::registry(args),
    }
}
