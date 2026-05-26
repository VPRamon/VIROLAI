//! Update operations for the run registry.

use rusqlite::params;

use super::super::store::Registry;

impl Registry {
    /// Attaches an already stored schedule hash to a run.
    pub fn attach_schedule(&self, run_key: &str, schedule_hash: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE runs SET schedule_hash = ?2 WHERE run_key = ?1",
                params![run_key, schedule_hash],
            )
            .map_err(|e| format!("failed to attach schedule to run {run_key}: {e}"))?;
        Ok(())
    }
}
