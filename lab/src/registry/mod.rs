//! SQLite-backed run registry and query model.
//!
//! The registry persists successful scheduler runs, while query-time helpers
//! define how rows are filtered, sorted, ranked, or exported by callers.

mod identity;
mod query;
mod row;
mod store;

pub use identity::{METRICS_VERSION, RunIdentity, hash_file, scheduler_version};
pub use query::{
    BestOpts, DEFAULT_RUN_DB, ListOpts, SortDirection, SortKey, default_sort_keys, metric_col,
    parse_sort_key, registry_path,
};
pub use row::RunRow;
pub use store::Registry;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_identity(dataset_hash: &str, config_json: &str) -> RunIdentity {
        RunIdentity {
            dataset_id: "ds1".to_string(),
            dataset_path: "/data/ds1.json".to_string(),
            dataset_hash: dataset_hash.to_string(),
            algorithm: "est".to_string(),
            config_slug: "e1-k1-b1".to_string(),
            config_json: config_json.to_string(),
            horizon_json: None,
            scheduler_version: "v1".to_string(),
            metrics_version: METRICS_VERSION.to_string(),
        }
    }

    #[test]
    fn same_inputs_produce_same_run_key() {
        let id = make_identity("aabbcc", r#"{"k_beams":1}"#);
        assert_eq!(id.run_key(), id.run_key());
    }

    #[test]
    fn different_dataset_hash_changes_run_key() {
        let id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let id2 = make_identity("hash2", r#"{"k_beams":1}"#);
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_config_changes_run_key() {
        let id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let id2 = make_identity("hash1", r#"{"k_beams":2}"#);
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_horizon_changes_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.horizon_json = None;
        id2.horizon_json = Some(r#"{"start_mjd":62000,"end_mjd":62001}"#.to_string());
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_scheduler_version_changes_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.scheduler_version = "v1".to_string();
        id2.scheduler_version = "v2".to_string();
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_metrics_version_changes_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.metrics_version = "schedule_metrics/1".to_string();
        id2.metrics_version = "schedule_metrics/2".to_string();
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn dataset_path_is_not_part_of_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.dataset_path = "/path/a/ds1.json".to_string();
        id2.dataset_path = "/path/b/ds1.json".to_string();
        assert_eq!(id1.run_key(), id2.run_key());
    }

    fn open_temp_registry() -> (Registry, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.sqlite");
        let reg = Registry::open(&path).unwrap();
        (reg, dir)
    }

    const SAMPLE_METRICS: &str = r#"{
        "scheduled_task_ratio": 0.8,
        "scheduled_priority_ratio": 0.9,
        "priority_density": 1.1,
        "utilization": 0.75,
        "fragmentation": {"fragmentation_index": 0.2, "gap_count": 2, "gap_total_sec": 100.0, "largest_gap_sec": 60.0},
        "composite_rank_score": 0.85,
        "scheduler_runtime_ms": 42.0
    }"#;

    #[test]
    fn schema_initialization_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.sqlite");
        let _ = Registry::open(&path).unwrap();
        let _ = Registry::open(&path).unwrap();
    }

    #[test]
    fn insert_and_lookup_by_key() {
        let (reg, _dir) = open_temp_registry();
        let id = make_identity("deadbeef", r#"{"k_beams":1}"#);
        let key = id.run_key();

        assert!(!reg.contains(&key).unwrap());
        reg.upsert(&id, SAMPLE_METRICS, None, Some("ds1__est__e1-k1-b1"))
            .unwrap();
        assert!(reg.contains(&key).unwrap());

        let row = reg.get_row(&key).unwrap().unwrap();
        assert_eq!(row.dataset_id, "ds1");
        assert_eq!(row.algorithm, "est");
    }

    #[test]
    fn upsert_refreshes_existing_record() {
        let (reg, _dir) = open_temp_registry();
        let id = make_identity("deadbeef", r#"{"k_beams":1}"#);
        reg.upsert(&id, SAMPLE_METRICS, None, None).unwrap();
        reg.upsert(&id, SAMPLE_METRICS, None, None).unwrap();

        let rows = reg
            .list(&ListOpts {
                dataset: Some("ds1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn list_filters_by_dataset() {
        let (reg, _dir) = open_temp_registry();

        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.dataset_id = "ds1".to_string();
        id1.config_slug = "e1-k1-b1".to_string();
        let mut id2 = make_identity("hash2", r#"{"k_beams":2}"#);
        id2.dataset_id = "ds2".to_string();
        id2.config_slug = "e1-k2-b1".to_string();

        reg.upsert(&id1, SAMPLE_METRICS, None, None).unwrap();
        reg.upsert(&id2, SAMPLE_METRICS, None, None).unwrap();

        let rows = reg
            .list(&ListOpts {
                dataset: Some("ds1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].dataset_id, "ds1");
    }

    #[test]
    fn best_returns_ordered_by_metric() {
        let (reg, _dir) = open_temp_registry();

        let metrics_a = r#"{"scheduled_task_ratio":0.6,"scheduled_priority_ratio":0.6,"priority_density":1.0,"utilization":0.5,"fragmentation":{"fragmentation_index":0.3,"gap_count":1,"gap_total_sec":50.0,"largest_gap_sec":50.0},"composite_rank_score":0.5,"scheduler_runtime_ms":10.0}"#;
        let metrics_b = r#"{"scheduled_task_ratio":0.9,"scheduled_priority_ratio":0.95,"priority_density":1.05,"utilization":0.85,"fragmentation":{"fragmentation_index":0.1,"gap_count":0,"gap_total_sec":0.0,"largest_gap_sec":0.0},"composite_rank_score":0.9,"scheduler_runtime_ms":20.0}"#;

        let mut id_a = make_identity("hash1", r#"{"k_beams":1}"#);
        id_a.config_slug = "e1-k1-b1".to_string();
        let mut id_b = make_identity("hash1", r#"{"k_beams":2}"#);
        id_b.config_slug = "e1-k2-b1".to_string();

        reg.upsert(&id_a, metrics_a, None, None).unwrap();
        reg.upsert(&id_b, metrics_b, None, None).unwrap();

        let rows = reg
            .best(&BestOpts {
                dataset_id: "ds1".to_string(),
                algorithm: None,
                sort: vec![parse_sort_key("scheduled_priority_ratio:desc").unwrap()],
                limit: Some(2),
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].config_slug, "e1-k2-b1");
    }

    #[test]
    fn composite_score_is_not_a_registry_sort_metric() {
        let err = parse_sort_key("composite_score:desc").unwrap_err();
        assert!(err.contains("query-time score"));
    }

    #[test]
    fn prefix_resolution_succeeds_for_unique_prefix() {
        let (reg, _dir) = open_temp_registry();
        let id = make_identity("deadbeef", r#"{"k_beams":1}"#);
        let key = id.run_key();
        reg.upsert(&id, SAMPLE_METRICS, None, None).unwrap();

        let prefix = &key[..16];
        let resolved = reg.resolve_prefix(prefix).unwrap();
        assert_eq!(resolved, key);
    }

    #[test]
    fn prefix_resolution_errors_on_missing() {
        let (reg, _dir) = open_temp_registry();
        let result = reg.resolve_prefix("nonexistentprefix");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no run found"));
    }
}
