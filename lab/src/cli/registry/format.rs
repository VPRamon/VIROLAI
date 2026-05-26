//! Registry row serialization and table formatting.

use lab::registry::{RunRow, SortKey};

pub(super) fn print_rows(rows: &[RunRow], format: &str, sort: &[SortKey]) -> Result<(), String> {
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
    print!("{}", format_metric_rows(rows, sort));
    Ok(())
}

pub(super) fn row_json(row: &RunRow, score: Option<f64>) -> serde_json::Value {
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

pub(super) fn parse_metrics(metrics_json: &str) -> serde_json::Value {
    serde_json::from_str(metrics_json).unwrap_or(serde_json::Value::Null)
}

/// Extracts a metric from a parsed `metrics_json` value.
/// Returns `None` when the key is absent or null, never defaulting to 0.0.
fn metric_opt(mv: &serde_json::Value, metric: &str) -> Option<f64> {
    match metric {
        "task_ratio" | "scheduled_task_ratio" => mv["scheduled_task_ratio"].as_f64(),
        "scheduled_task_count" => mv["scheduled_task_count"].as_f64(),
        "scheduled_priority_sum" => mv["scheduled_priority_sum"].as_f64(),
        "priority_ratio" | "scheduled_priority_ratio" => mv["scheduled_priority_ratio"].as_f64(),
        "priority_density" => mv["priority_density"].as_f64(),
        "scheduled_time_sec" => mv["scheduled_time_sec"].as_f64(),
        "requested_time_sec" => mv["requested_time_sec"].as_f64(),
        "scheduled_time_ratio" => mv["scheduled_time_ratio"].as_f64(),
        "utilization" => mv["utilization"].as_f64(),
        "fragmentation_index" => mv["fragmentation"]["fragmentation_index"].as_f64(),
        "runtime_ms" | "scheduler_runtime_ms" => mv["scheduler_runtime_ms"].as_f64(),
        _ => None,
    }
}

/// Canonical metric columns shown in every table output.
/// Tuple: `(metric_name, column_header, column_width, decimal_places)`.
const METRIC_DISPLAY_COLS: &[(&str, &str, usize, usize)] = &[
    ("scheduled_priority_sum", "psum", 9, 2),
    ("scheduled_priority_ratio", "p_ratio", 8, 4),
    ("scheduled_task_ratio", "t_ratio", 8, 4),
    ("scheduled_time_ratio", "time_r", 8, 4),
    ("priority_density", "density", 8, 4),
    ("runtime_ms", "runtime", 10, 2),
];

/// Formats `val` right-aligned within `width` characters with `prec` decimal
/// places. Absent values are shown as `"-"` right-aligned in the same field.
fn fmt_f64_col(val: Option<f64>, width: usize, prec: usize) -> String {
    let s = match val {
        Some(f) => format!("{:.prec$}", f, prec = prec),
        None => "-".to_string(),
    };
    let pad = width.saturating_sub(s.len());
    format!("{}{}", " ".repeat(pad), s)
}

/// Normalizes a sort-metric name to the canonical key used in
/// `METRIC_DISPLAY_COLS`.
fn normalize_metric_for_display(m: &str) -> &str {
    match m {
        "task_ratio" | "scheduled_task_ratio" => "scheduled_task_ratio",
        "priority_ratio" | "scheduled_priority_ratio" => "scheduled_priority_ratio",
        "scheduler_runtime_ms" => "runtime_ms",
        other => other,
    }
}

/// Truncates `s` to `max` characters, appending `".."` if trimmed.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}..", &s[..max.saturating_sub(2)])
    }
}

/// Formats `rows` as a metric table string.
fn format_metric_rows(rows: &[RunRow], sort: &[SortKey]) -> String {
    let default_metrics: std::collections::HashSet<&str> =
        METRIC_DISPLAY_COLS.iter().map(|(m, _, _, _)| *m).collect();

    let mut extra_cols: Vec<&str> = vec![];
    for sk in sort {
        let norm = normalize_metric_for_display(&sk.metric);
        if !default_metrics.contains(norm) && !extra_cols.contains(&norm) {
            extra_cols.push(norm);
        }
    }

    let extra_col_width: usize = 10;
    let mut out = String::new();

    out.push_str(&format!(
        "{:<18}  {:<12}  {:<8}  {:<30}",
        "run_key (prefix)", "dataset", "algo", "config_slug"
    ));
    for (_, hdr, w, _) in METRIC_DISPLAY_COLS {
        out.push_str("  ");
        let pad = w.saturating_sub(hdr.len());
        out.push_str(&" ".repeat(pad));
        out.push_str(hdr);
    }
    for col in &extra_cols {
        out.push_str("  ");
        let pad = extra_col_width.saturating_sub(col.len());
        out.push_str(&" ".repeat(pad));
        out.push_str(col);
    }
    out.push_str("  created_at\n");

    let metric_width: usize = METRIC_DISPLAY_COLS
        .iter()
        .map(|(_, _, w, _)| w + 2)
        .sum::<usize>();
    let extra_width: usize = extra_cols.len() * (extra_col_width + 2);
    let sep_len = 74 + metric_width + extra_width + 2 + 19;
    out.push_str(&"-".repeat(sep_len));
    out.push('\n');

    for row in rows {
        let mv = parse_metrics(&row.metrics_json);
        out.push_str(&format!(
            "{:<18}  {:<12}  {:<8}  {:<30}",
            &row.run_key[..row.run_key.len().min(16)],
            truncate(&row.dataset_id, 12),
            truncate(&row.algorithm, 8),
            truncate(&row.config_slug, 30),
        ));
        for (metric, _, w, prec) in METRIC_DISPLAY_COLS {
            out.push_str("  ");
            out.push_str(&fmt_f64_col(metric_opt(&mv, metric), *w, *prec));
        }
        for col in &extra_cols {
            out.push_str("  ");
            out.push_str(&fmt_f64_col(metric_opt(&mv, col), extra_col_width, 4));
        }
        out.push_str("  ");
        out.push_str(&row.created_at);
        out.push('\n');
    }
    out.push_str(&format!("({} rows)\n", rows.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lab::registry::parse_sort_key;

    fn make_run_row(metrics_json: &str) -> RunRow {
        RunRow {
            run_key: "b50d151629d65018abcdef1234567890abcdef1234567890abcdef1234567890ab"
                .to_string(),
            dataset_id: "isdc_s".to_string(),
            dataset_path: "/data/isdc_s.json".to_string(),
            algorithm: "est".to_string(),
            config_slug: "e2-k1-b1-future_flexibility".to_string(),
            identity_json: "{}".to_string(),
            metrics_json: metrics_json.to_string(),
            schedule_json: None,
            created_at: "2024-05-26T10:00:00Z".to_string(),
            last_seen_at: "2024-05-26T10:00:00Z".to_string(),
            source_cell_id: None,
        }
    }

    const FULL_METRICS: &str = r#"{
        "scheduled_priority_sum": 1234.56,
        "scheduled_priority_ratio": 0.8123,
        "scheduled_task_ratio": 0.7000,
        "scheduled_time_ratio": 0.6421,
        "priority_density": 1.1604,
        "scheduler_runtime_ms": 98.2
    }"#;

    #[test]
    fn metric_columns_appear_in_header() {
        let row = make_run_row(FULL_METRICS);
        let out = format_metric_rows(&[row], &[]);
        let header_line = out.lines().next().unwrap();
        assert!(header_line.contains("psum"), "header missing psum");
        assert!(header_line.contains("p_ratio"), "header missing p_ratio");
        assert!(header_line.contains("t_ratio"), "header missing t_ratio");
        assert!(header_line.contains("time_r"), "header missing time_r");
        assert!(header_line.contains("density"), "header missing density");
        assert!(header_line.contains("runtime"), "header missing runtime");
        assert!(
            header_line.contains("created_at"),
            "header missing created_at"
        );
    }

    #[test]
    fn metric_values_appear_in_data_row() {
        let row = make_run_row(FULL_METRICS);
        let out = format_metric_rows(&[row], &[]);
        let data_line = out.lines().nth(2).unwrap();
        assert!(data_line.contains("1234.56"), "psum value missing");
        assert!(data_line.contains("0.8123"), "p_ratio value missing");
        assert!(data_line.contains("0.7000"), "t_ratio value missing");
        assert!(data_line.contains("0.6421"), "time_r value missing");
        assert!(data_line.contains("1.1604"), "density value missing");
        assert!(data_line.contains("98.20"), "runtime value missing");
        assert!(data_line.contains("2024-05-26"), "created_at missing");
    }

    #[test]
    fn missing_metrics_show_dash_not_zero() {
        let row = make_run_row("{}");
        let out = format_metric_rows(&[row], &[]);
        let data_line = out.lines().nth(2).unwrap();
        assert!(
            !data_line.contains("0.0000"),
            "missing ratio should be '-' not 0.0000: {data_line}"
        );
        assert!(
            !data_line.contains(" 0.00"),
            "missing psum/runtime should be '-' not 0.00: {data_line}"
        );
    }

    #[test]
    fn extra_sort_column_appended_for_non_default_metric() {
        let row = make_run_row(r#"{"utilization": 0.75}"#);
        let sort = vec![parse_sort_key("utilization:desc").unwrap()];
        let out = format_metric_rows(&[row], &sort);
        let header_line = out.lines().next().unwrap();
        assert!(
            header_line.contains("utilization"),
            "extra sort column missing from header: {header_line}"
        );
        let data_line = out.lines().nth(2).unwrap();
        assert!(
            data_line.contains("0.7500"),
            "utilization value missing from data row: {data_line}"
        );
    }

    #[test]
    fn default_sort_metrics_do_not_create_extra_columns() {
        let row = make_run_row(FULL_METRICS);
        let sort = vec![
            parse_sort_key("scheduled_priority_ratio:desc").unwrap(),
            parse_sort_key("runtime_ms:asc").unwrap(),
        ];
        let out = format_metric_rows(&[row], &sort);
        let header_line = out.lines().next().unwrap();
        assert_eq!(header_line.matches("p_ratio").count(), 1);
        assert_eq!(header_line.matches("runtime").count(), 1);
    }

    #[test]
    fn fmt_f64_col_right_aligns_to_width() {
        let s = fmt_f64_col(Some(1.5), 8, 4);
        assert_eq!(s.len(), 8, "unexpected len for '{s}'");
        assert!(s.starts_with(' '), "should be right-aligned: '{s}'");
    }

    #[test]
    fn fmt_f64_col_shows_dash_for_none() {
        let s = fmt_f64_col(None, 8, 4);
        assert_eq!(s.len(), 8);
        assert!(s.trim() == "-");
    }
}
