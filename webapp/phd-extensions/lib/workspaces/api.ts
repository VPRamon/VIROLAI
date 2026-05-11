/**
 * Thin axios client for the `/v1/workspaces` HTTP surface.
 *
 * Mirrors the experiments client; manifests are exchanged as bare JSON
 * (no schedule artifacts uploaded by default — see plan §12).
 */
import axios, { type AxiosInstance, AxiosError } from 'axios';
import type {
  ComparisonResponse,
  ListWorkspacesResponse,
  ManifestBody,
  ManifestEntry,
  ScheduledPriorityStair,
  WorkspaceDetailResponse,
  WorkspaceRecord,
} from './types';

export const WORKSPACES_BASE = '/api/v1/workspaces';

let client: AxiosInstance | null = null;

export function getWorkspacesClient(): AxiosInstance {
  if (!client) {
    client = axios.create({
      baseURL: '/',
      headers: { 'Content-Type': 'application/json' },
    });
  }
  return client;
}

export function setWorkspacesClient(instance: AxiosInstance | null): void {
  client = instance;
}

export class WorkspacesApiError extends Error {
  readonly status?: number;
  readonly serverMessage?: string;
  constructor(message: string, status?: number, serverMessage?: string) {
    super(message);
    this.name = 'WorkspacesApiError';
    this.status = status;
    this.serverMessage = serverMessage;
  }
}

function unwrap<T>(p: Promise<{ data: T }>): Promise<T> {
  return p
    .then((r) => r.data)
    .catch((err: AxiosError<{ error?: { message?: string } }>) => {
      const status = err.response?.status;
      const msg =
        err.response?.data?.error?.message ?? err.message ?? 'request failed';
      throw new WorkspacesApiError(msg, status, msg);
    });
}

export function listWorkspaces(opts?: {
  status?: 'active' | 'archived';
}): Promise<ListWorkspacesResponse> {
  const q = opts?.status ? `?status=${opts.status}` : '';
  return unwrap(getWorkspacesClient().get(`${WORKSPACES_BASE}${q}`));
}

export function createWorkspace(body: {
  name: string;
  description?: string | null;
}): Promise<{ workspace: WorkspaceRecord }> {
  return unwrap(getWorkspacesClient().post(WORKSPACES_BASE, body));
}

export function getWorkspace(id: string): Promise<WorkspaceDetailResponse> {
  return unwrap(getWorkspacesClient().get(`${WORKSPACES_BASE}/${encodeURIComponent(id)}`));
}

export function archiveWorkspace(
  id: string,
  status: 'active' | 'archived',
): Promise<{ workspace: WorkspaceRecord }> {
  return unwrap(getWorkspacesClient().patch(`${WORKSPACES_BASE}/${encodeURIComponent(id)}`, { status }));
}

export function deleteWorkspace(id: string): Promise<void> {
  return unwrap(getWorkspacesClient().delete(`${WORKSPACES_BASE}/${encodeURIComponent(id)}`));
}

export function addManifest(
  id: string,
  manifest: unknown,
  idempotencyKey?: string,
): Promise<{ manifest: ManifestEntry; created: boolean }> {
  return unwrap(
    getWorkspacesClient().post(`${WORKSPACES_BASE}/${encodeURIComponent(id)}/manifests`, {
      manifest,
      idempotency_key: idempotencyKey,
    }),
  );
}

export function addManifestBatch(
  id: string,
  items: { manifest: unknown; idempotency_key?: string }[],
): Promise<{
  summary: { created: number; deduplicated: number; failed: number };
  results: Array<
    | { ok: true; created: boolean; manifest: ManifestEntry }
    | { ok: false; error: { message: string } }
  >;
}> {
  return unwrap(
    getWorkspacesClient().post(
      `${WORKSPACES_BASE}/${encodeURIComponent(id)}/manifests/batch`,
      { items },
    ),
  );
}

export function removeManifest(
  id: string,
  manifestId: string,
  deleteArtifact = false,
): Promise<void> {
  const q = deleteArtifact ? '?delete_artifact=1' : '';
  return unwrap(
    getWorkspacesClient().delete(
      `${WORKSPACES_BASE}/${encodeURIComponent(id)}/manifests/${encodeURIComponent(manifestId)}${q}`,
    ),
  );
}

export function getManifest(
  id: string,
  manifestId: string,
): Promise<{ manifest: { body: { metrics?: { scheduled_priority_stair?: ScheduledPriorityStair } } } }> {
  return unwrap(
    getWorkspacesClient().get(
      `${WORKSPACES_BASE}/${encodeURIComponent(id)}/manifests/${encodeURIComponent(manifestId)}`,
    ),
  );
}

export function getFullManifestBody(
  id: string,
  manifestId: string,
): Promise<{ manifest: ManifestBody }> {
  return unwrap(
    getWorkspacesClient().get(
      `${WORKSPACES_BASE}/${encodeURIComponent(id)}/manifests/${encodeURIComponent(manifestId)}`,
    ),
  );
}

export function ingestSchedule(
  id: string,
  schedule: unknown,
  idempotencyKey?: string,
): Promise<{ manifest: ManifestEntry; created: boolean }> {
  return unwrap(
    getWorkspacesClient().post(`${WORKSPACES_BASE}/${encodeURIComponent(id)}/schedules`, {
      schedule,
      idempotency_key: idempotencyKey,
    }),
  );
}

export function ingestScheduleBatch(
  id: string,
  items: { schedule: unknown; idempotency_key?: string }[],
): Promise<{
  summary: { created: number; deduplicated: number; failed: number };
  results: Array<
    | { ok: true; created: boolean; manifest: ManifestEntry }
    | { ok: false; error: { message: string } }
  >;
}> {
  return unwrap(
    getWorkspacesClient().post(
      `${WORKSPACES_BASE}/${encodeURIComponent(id)}/schedules/batch`,
      { items },
    ),
  );
}

export function getScheduleForManifest(
  id: string,
  manifestId: string,
): Promise<{ schedule: unknown }> {
  return unwrap(
    getWorkspacesClient().get(
      `${WORKSPACES_BASE}/${encodeURIComponent(id)}/manifests/${encodeURIComponent(manifestId)}/schedule`,
    ),
  );
}

export function getComparison(id: string): Promise<ComparisonResponse> {
  return unwrap(
    getWorkspacesClient().get(`${WORKSPACES_BASE}/${encodeURIComponent(id)}/comparison`),
  );
}
