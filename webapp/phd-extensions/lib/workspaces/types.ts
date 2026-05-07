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
