/**
 * TypeScript mirror of the Rust DTOs exposed by the
 * `/v1/experiments/...` HTTP surface. See:
 *   - `webapp/scripts/experiments/catalog.rs`     (DTO definitions)
 *   - `webapp/scripts/experiments/state_events.rs` (SSE event shape)
 *   - `src/metrics.rs`                            (ScheduleMetrics shape)
 *
 * Everything here is purely structural — only what the frontend needs.
 * Optional fields are typed `T | null | undefined` because Rust's
 * `Option<T>` becomes `null` in JSON, while `serde(skip_serializing_if)`
 * may omit the key entirely.
 */

export type RunStatus = 'running' | 'completed' | 'failed' | 'pending';
export type CellStatus = 'started' | 'completed' | 'failed';

export interface ExperimentSummary {
  experiment_slug: string;
  run_id: string;
  experiment_name: string;
  output_dir: string;
  created_at: string;
  updated_at: string;
  total_cells: number;
  completed_cells: number;
  failed_cells: number;
  running_cells: number;
  status: RunStatus;
}

export interface CellSummary {
  cell_id: string;
  dataset_id?: string | null;
  algorithm?: string | null;
  config_slug?: string | null;
  status?: CellStatus | null;
  error?: string | null;
  schedule_path?: string | null;
  metrics_path?: string | null;
  trace_path?: string | null;
  started_at?: string | null;
  finished_at?: string | null;
}

export interface PriorityStats {
  count: number;
  sum: number;
  min: number;
  max: number;
  mean: number;
  std: number;
  p25: number;
  p50: number;
  p75: number;
  p90: number;
}

export interface FragmentationStats {
  gap_count: number;
  gap_total_sec: number;
  largest_gap_sec: number;
  fragmentation_index: number;
}

export interface ResourceMetrics {
  resource_id: string;
  scheduled_task_count: number;
  scheduled_time_sec: number;
  scheduled_priority_sum: number;
  utilization: number;
}

export interface RankingWeights {
  scheduled_task: number;
  scheduled_priority: number;
  utilization: number;
  fragmentation: number;
}

export interface ScheduleMetrics {
  scheduled_task_count: number;
  total_task_count: number;
  scheduled_task_ratio: number;
  scheduled_priority: PriorityStats;
  scheduled_priority_sum: number;
  total_priority_sum: number;
  scheduled_priority_ratio: number;
  priority_density: number;
  fragmentation: FragmentationStats;
  total_horizon_sec: number;
  available_time_sec: number;
  scheduled_time_sec: number;
  utilization: number;
  per_resource: ResourceMetrics[];
  composite_rank_score: number;
  ranking_weights: RankingWeights;
  scheduled_priority_stair?: unknown;
}

export interface CellDetail {
  cell_id: string;
  dataset_id?: string | null;
  algorithm?: string | null;
  config_slug?: string | null;
  status?: CellStatus | null;
  metrics?: ScheduleMetrics | null;
  schedule_path?: string | null;
  trace_path?: string | null;
  error?: string | null;
}

export interface ExperimentDetail extends ExperimentSummary {
  spec: unknown;
  cells: CellSummary[];
}

export interface RunDetailResponse {
  experiment: ExperimentDetail;
  live_status: unknown;
}

export interface ParetoPoint {
  cell_id: string;
  x: number;
  y: number;
}

export interface ParetoResponse {
  x_field: string;
  y_field: string;
  maximize_x: boolean;
  maximize_y: boolean;
  front: ParetoPoint[];
}

export interface RankingEntry {
  key: string;
  mean_score: number;
  mean_completion: number;
  mean_priority_sum: number;
  mean_utilization: number;
  mean_fragmentation_index: number;
  n: number;
}

export interface RankingResponse {
  by: 'dataset' | 'algorithm';
  weights?: RankingWeights | null;
  entries: RankingEntry[];
}

export interface BulkCellMetricsItem {
  cell_id: string;
  metrics?: ScheduleMetrics;
  error?: string;
}

/**
 * Shape of an SSE `state` event emitted by `/events`. Mirrors
 * `webapp/scripts/experiments/state_events.rs::StateEvent`.
 */
export interface StateEvent {
  cell_id: string;
  status: CellStatus;
  schedule_path?: string | null;
  metrics_path?: string | null;
  trace_path?: string | null;
  error?: string | null;
  started_at: string;
  finished_at?: string | null;
}

export interface ExperimentSpec {
  slug: string;
  description?: string;
  datasets: Array<{ id?: string; path: string; description?: string }>;
  algorithms: string[];
  /**
   * Per-algorithm sweep axes / configuration.  The exact shape mirrors
   * `scripts/experiment_matrix/spec.rs::ExperimentSpec` — for v1 the
   * frontend just forwards an opaque JSON value.
   */
  sweeps?: Record<string, unknown>;
  output_dir?: string;
}

/** Listing envelope returned by `GET /v1/experiments`. */
export interface ListExperimentsResponse {
  experiments: ExperimentSummary[];
}

/** Listing envelope for cells. */
export interface ListCellsResponse {
  cells: CellSummary[];
}

/** Bulk metrics envelope. */
export interface BulkCellsResponse {
  items: BulkCellMetricsItem[];
}
