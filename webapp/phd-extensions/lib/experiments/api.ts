/**
 * Thin axios client for the `/v1/experiments` HTTP surface.
 *
 * Mirrors the convention used by `@/api/client` (baseURL = `/api`,
 * which is proxied to the backend in dev and rewritten by nginx in
 * prod), but is owned by the extensions pack so we don't take a hard
 * dependency on private TSI internals (per the v1 extension
 * contract).
 */
import axios, { type AxiosInstance, AxiosError } from 'axios';
import type {
  BulkCellsResponse,
  CellDetail,
  ExperimentSpec,
  ListCellsResponse,
  ListExperimentsResponse,
  ParetoResponse,
  RankingResponse,
  RankingWeights,
  RunDetailResponse,
} from './types';

export const EXPERIMENTS_BASE = '/api/v1/experiments';

/**
 * Allow tests to substitute a mock client without monkey-patching axios
 * globally. The default instance is constructed lazily so test setup
 * code can call `setExperimentsClient(...)` first.
 */
let client: AxiosInstance | null = null;

export function getExperimentsClient(): AxiosInstance {
  if (!client) {
    client = axios.create({
      baseURL: '/',
      headers: { 'Content-Type': 'application/json' },
    });
  }
  return client;
}

export function setExperimentsClient(instance: AxiosInstance | null): void {
  client = instance;
}

/** Translate axios errors into `Error`s the UI can render verbatim. */
export class ExperimentsApiError extends Error {
  readonly status?: number;
  readonly serverMessage?: string;
  constructor(message: string, status?: number, serverMessage?: string) {
    super(message);
    this.name = 'ExperimentsApiError';
    this.status = status;
    this.serverMessage = serverMessage;
  }
}

function unwrap<T>(p: Promise<{ data: T }>): Promise<T> {
  return p
    .then((r) => r.data)
    .catch((err: unknown) => {
      if (err instanceof AxiosError) {
        const status = err.response?.status;
        const data = err.response?.data as { error?: string; message?: string } | undefined;
        const serverMessage = data?.error ?? data?.message;
        throw new ExperimentsApiError(
          serverMessage ?? err.message ?? 'request failed',
          status,
          serverMessage,
        );
      }
      throw err;
    });
}

// ── reads ──────────────────────────────────────────────────────────────────

export function listExperiments(): Promise<ListExperimentsResponse> {
  return unwrap(getExperimentsClient().get(EXPERIMENTS_BASE));
}

export function getRun(slug: string, runId: string): Promise<RunDetailResponse> {
  return unwrap(
    getExperimentsClient().get(`${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}`),
  );
}

export interface ListCellsParams {
  status?: string;
  dataset_id?: string;
  algorithm?: string;
  limit?: number;
  offset?: number;
}

export function listCells(
  slug: string,
  runId: string,
  params: ListCellsParams = {},
): Promise<ListCellsResponse> {
  return unwrap(
    getExperimentsClient().get(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/cells`,
      { params },
    ),
  );
}

/**
 * Single round-trip metrics fetch for many cells. PREFERRED over
 * `getCell` in any UI that renders >1 cell — the matrix tab fetches
 * thousands of cells in a single request via this endpoint.
 */
export function bulkCells(
  slug: string,
  runId: string,
  cellIds: string[],
): Promise<BulkCellsResponse> {
  return unwrap(
    getExperimentsClient().post(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/cells/bulk`,
      { cell_ids: cellIds },
    ),
  );
}

export function getCell(slug: string, runId: string, cellId: string): Promise<{ cell: CellDetail }> {
  return unwrap(
    getExperimentsClient().get(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/cells/${encodeURIComponent(cellId)}`,
    ),
  );
}

export interface ParetoQuery {
  x: string;
  y: string;
  xmax?: boolean;
  ymax?: boolean;
}

export function getPareto(
  slug: string,
  runId: string,
  q: ParetoQuery,
): Promise<ParetoResponse> {
  return unwrap(
    getExperimentsClient().get(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/pareto`,
      { params: q },
    ),
  );
}

export interface RankingQuery {
  by: 'dataset' | 'algorithm';
  weights?: Partial<RankingWeights>;
}

export function getRanking(
  slug: string,
  runId: string,
  q: RankingQuery,
): Promise<RankingResponse> {
  return unwrap(
    getExperimentsClient().get(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/ranking`,
      { params: { by: q.by, ...q.weights } },
    ),
  );
}

// ── writes ─────────────────────────────────────────────────────────────────

export interface SubmitResult {
  slug?: string;
  run_id?: string;
  output_dir?: string;
  [k: string]: unknown;
}

export function submitExperiment(spec: ExperimentSpec | unknown): Promise<SubmitResult> {
  return unwrap(getExperimentsClient().post(EXPERIMENTS_BASE, spec));
}

export function cancelRun(slug: string, runId: string): Promise<unknown> {
  return unwrap(
    getExperimentsClient().post(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/cancel`,
    ),
  );
}

export function resumeRun(slug: string, runId: string): Promise<unknown> {
  return unwrap(
    getExperimentsClient().post(
      `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/resume`,
    ),
  );
}

/** URL for `<a href>` downloads of the per-run summary CSV. */
export function summaryCsvUrl(slug: string, runId: string): string {
  return `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/summary.csv`;
}

/** URL for the SSE stream consumed by `useExperimentRun`. */
export function eventsUrl(slug: string, runId: string): string {
  return `${EXPERIMENTS_BASE}/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/events`;
}
