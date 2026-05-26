//! Sibling binary discovery and subprocess dispatch.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

pub(crate) fn exec_sibling(name: &str, args: &[String]) -> Result<ExitCode, String> {
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

pub(crate) fn locate_sibling(name: &str) -> PathBuf {
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
