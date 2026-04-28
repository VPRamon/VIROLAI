/**
 * Aggregate hook: combine `useSchedules`, per-schedule `useInsights` and
 * `useAlgorithmTrace` into a normalised "run matrix" that the EST
 * Intelligence panels render against.
 *
 * The trace iteration shape is algorithm-specific; we cast the opaque
 * `AlgorithmTraceIteration` into a richer EST iteration view here so each
 * panel can stay focused on rendering.
 */
import { useMemo } from 'react';
import { useQueries } from '@tanstack/react-query';
import { api } from '@/api';
import { queryKeys } from '@/hooks/useApi';
import type {
  AlgorithmTraceSummary,
  InsightsData,
  ScheduleInfo,
} from '@/api/types';

/**
 * EST-specific iteration shape (one beam-search round).  Forward
 * compatible: any unknown fields fall through.
 */
export interface EstTraceIteration {
  round: number;
  beam_scores: number[];
  best_score?: number | null;
  median_score?: number | null;
  worst_score?: number | null;
  scheduled_in_best?: number | null;
  wall_ms?: number | null;
  [extra: string]: unknown;
}

/** Run-level summary (algorithm-agnostic; algorithm-specific extras flow through). */
export type EstTraceSummary = AlgorithmTraceSummary & {
  total_rounds?: number;
  best_score?: number;
  best_scheduled_count?: number;
  wall_ms_total?: number;
};

export interface RunRow {
  schedule: ScheduleInfo;
  insights: InsightsData | undefined;
  traceSummary: EstTraceSummary | undefined;
  iterations: EstTraceIteration[] | undefined;
  algorithmConfig: Record<string, unknown> | undefined;
  insightsError: string | undefined;
  traceError: string | undefined;
}

export interface RunMatrix {
  runs: RunRow[];
  loading: boolean;
  hasAnyTrace: boolean;
}

const errorMessage = (e: unknown): string | undefined => {
  if (!e) return undefined;
  if (e instanceof Error) return e.message;
  return String(e);
};

export function useRunMatrix(schedules: ScheduleInfo[]): RunMatrix {
  const ids = schedules.map((s) => s.schedule_id);

  const insightsQueries = useQueries({
    queries: ids.map((id) => ({
      queryKey: queryKeys.insights(id),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.getInsights(id, { signal }),
      enabled: id > 0,
    })),
  });

  const traceQueries = useQueries({
    queries: ids.map((id) => ({
      queryKey: queryKeys.algorithmTrace(id),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.getAlgorithmTrace(id, { signal }),
      enabled: id > 0,
      retry: false,
    })),
  });

  return useMemo<RunMatrix>(() => {
    const runs: RunRow[] = schedules.map((schedule, i) => {
      const insightsQ = insightsQueries[i];
      const traceQ = traceQueries[i];
      const traceSummary = traceQ?.data?.summary as EstTraceSummary | undefined;
      const iterations = traceQ?.data?.iterations as EstTraceIteration[] | undefined;
      const algorithmConfig =
        (traceSummary?.algorithm_config as Record<string, unknown> | undefined) ??
        schedule.schedule_metadata?.algorithm_config;
      return {
        schedule,
        insights: insightsQ?.data,
        traceSummary,
        iterations,
        algorithmConfig,
        insightsError: errorMessage(insightsQ?.error),
        traceError: errorMessage(traceQ?.error),
      };
    });

    const loading =
      insightsQueries.some((q) => q.isFetching) || traceQueries.some((q) => q.isFetching);
    const hasAnyTrace = runs.some((r) => !!r.iterations?.length);

    return { runs, loading, hasAnyTrace };
  }, [schedules, insightsQueries, traceQueries]);
}
