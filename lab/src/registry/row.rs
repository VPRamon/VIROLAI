//! Registry row model and SQLite row mapping.

/// A registry row returned by query helpers.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub run_key: String,
    pub dataset_id: String,
    pub dataset_path: String,
    pub algorithm: String,
    pub config_slug: String,
    pub identity_json: String,
    pub metrics_json: String,
    /// Run-specific schedule metadata (`schedule_metadata` body), if stored.
    pub metadata_json: Option<String>,
    pub schedule_hash: Option<String>,
    /// Invariant schedule body shared across runs with the same `schedule_hash`.
    pub schedule_json: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub source_cell_id: Option<String>,
}

pub(super) fn row_to_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        run_key: row.get(0)?,
        dataset_id: row.get(1)?,
        dataset_path: row.get(2)?,
        algorithm: row.get(3)?,
        config_slug: row.get(4)?,
        identity_json: row.get(5)?,
        metrics_json: row.get(6)?,
        metadata_json: row.get(7)?,
        schedule_hash: row.get(8)?,
        schedule_json: row.get(9)?,
        created_at: row.get(10)?,
        last_seen_at: row.get(11)?,
        source_cell_id: row.get(12)?,
    })
}
