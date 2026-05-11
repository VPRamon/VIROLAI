/** TypeScript mirror of `webapp/scripts/workspaces/store.rs` records. */

export type WorkspaceStatus = 'active' | 'archived';

export interface WorkspaceRecord {
  id: string;
  name: string;
  description: string | null;
  status: WorkspaceStatus;
  created_at: string;
  updated_at: string;
  manifest_count: number;
}

export interface ManifestEntry {
  manifest_id: string;
  added_at: string;
  display_name: string;
  dataset_id: string;
  algorithm_id: string;
  idempotency_key: string;
}

export interface ManifestSummary {
  manifest_id: string;
  display_name: string;
  dataset_id: string;
  algorithm_id: string;
  scheduled_task_count: number | null;
  total_task_count: number | null;
  completion_ratio: number | null;
  utilization: number | null;
  composite_rank_score: number | null;
  priority_sum: number | null;
  fragmentation_index: number | null;
  stair_block_count: number;
  has_full_schedule: boolean;
  tsi_schedule_id: string | null;
  validation_status: string | null;
}

export interface ListWorkspacesResponse {
  workspaces: WorkspaceRecord[];
}

export interface WorkspaceDetailResponse {
  workspace: WorkspaceRecord;
  manifests: ManifestEntry[];
}

export interface ComparisonResponse {
  summaries: ManifestSummary[];
}

/** Stair metric as embedded in a manifest's `metrics.scheduled_priority_stair`. */
export interface PriorityStair {
  priority: number;
  start_index: number;
  end_index: number;
  count: number;
}
export interface ScheduledPriorityStair {
  metric: 'scheduled_priority_stair';
  sort: 'priority';
  direction: 'ascending' | 'descending';
  stairs: PriorityStair[];
  total_scheduled_items: number;
}

// ── Full metrics types (mirrors scheduler ScheduleMetrics) ──────────────────

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
  priority_sum: number;
  utilization: number;
}

export interface RankingWeights {
  completion: number;
  priority: number;
  utilization: number;
  fragmentation: number;
}

export interface FullScheduleMetrics {
  scheduled_task_count: number;
  total_task_count: number;
  completion_ratio: number;
  priority: PriorityStats;
  fragmentation: FragmentationStats;
  total_horizon_sec: number;
  available_time_sec: number;
  scheduled_time_sec: number;
  utilization: number;
  per_resource: ResourceMetrics[];
  composite_rank_score: number;
  ranking_weights: RankingWeights;
  scheduled_priority_stair?: ScheduledPriorityStair;
}

/** Full manifest body shape (subset relevant to the webapp). */
export interface ManifestBody {
  manifest_id: string;
  created_at: string;
  algorithm: { id: string; label: string };
  dataset: { id: string; name: string };
  metrics: FullScheduleMetrics;
}

// ── Per-group ranking (computed client-side) ────────────────────────────────

export interface GroupRankingEntry {
  key: string;
  n: number;
  mean_score: number;
  mean_completion: number;
  mean_priority_sum: number;
  mean_utilization: number;
  mean_fragmentation_index: number;
}

