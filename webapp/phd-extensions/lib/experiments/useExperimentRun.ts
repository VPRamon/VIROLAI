/**
 * `useExperimentRun(slug, runId)` — composes the run summary fetched
 * from `GET /runs/:run_id` with a live event stream from `/events`.
 *
 * SSE strategy:
 *   - Open `EventSource` once per (slug, runId).
 *   - Buffer `state` events into a `Map<cell_id, StateEvent>` and
 *     recompute the {pending, started, completed, failed} counters
 *     on every event — this is O(n) but trivially cheap and means a
 *     late re-emission of an existing cell doesn't double-count it.
 *   - On `error`, close the source and reconnect with capped
 *     exponential backoff (1s → 2s → 4s, max 30s). EventSource will
 *     also auto-reconnect on transport errors but we still need to
 *     handle 5xx-style stream termination explicitly.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { eventsUrl, getRun } from './api';
import type {
  CellStatus,
  ExperimentDetail,
  StateEvent,
} from './types';
import { useAsync } from './useAsync';

export interface RunCounters {
  total: number;
  started: number;
  completed: number;
  failed: number;
  /** completed / total in [0, 1]; 0 when total == 0. */
  progress: number;
}

export interface UseExperimentRunResult {
  data: ExperimentDetail | undefined;
  error: Error | undefined;
  loading: boolean;
  reload: () => void;
  counters: RunCounters;
  /** Latest StateEvent observed per cell_id (live). */
  latestEvents: ReadonlyMap<string, StateEvent>;
  /** True while the SSE channel is connected. */
  connected: boolean;
}

const RECONNECT_MIN_MS = 1000;
const RECONNECT_MAX_MS = 30000;

export function useExperimentRun(slug: string, runId: string): UseExperimentRunResult {
  const run = useAsync(() => getRun(slug, runId), [slug, runId]);

  const [latestEvents, setLatestEvents] = useState<Map<string, StateEvent>>(() => new Map());
  const [connected, setConnected] = useState(false);
  const sourceRef = useRef<EventSource | null>(null);
  const backoffRef = useRef(RECONNECT_MIN_MS);

  useEffect(() => {
    setLatestEvents(new Map());
    let cancelled = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const connect = () => {
      if (cancelled) return;
      // EventSource is undefined in SSR / some test envs; bail out gracefully.
      if (typeof EventSource === 'undefined') return;
      const es = new EventSource(eventsUrl(slug, runId));
      sourceRef.current = es;
      es.addEventListener('open', () => {
        backoffRef.current = RECONNECT_MIN_MS;
        setConnected(true);
      });
      es.addEventListener('state', (raw) => {
        try {
          const ev = JSON.parse((raw as MessageEvent).data) as StateEvent;
          setLatestEvents((prev) => {
            const next = new Map(prev);
            next.set(ev.cell_id, ev);
            return next;
          });
        } catch {
          // Malformed event — server logs the original; drop on the floor.
        }
      });
      es.addEventListener('error', () => {
        setConnected(false);
        es.close();
        sourceRef.current = null;
        if (cancelled) return;
        const delay = backoffRef.current;
        backoffRef.current = Math.min(backoffRef.current * 2, RECONNECT_MAX_MS);
        reconnectTimer = setTimeout(connect, delay);
      });
    };

    connect();
    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      sourceRef.current?.close();
      sourceRef.current = null;
      setConnected(false);
    };
  }, [slug, runId]);

  const counters = useMemo<RunCounters>(() => {
    const exp = run.data?.experiment;
    // Prefer the catalog totals; fall back to live events when the
    // initial fetch hasn't returned yet.
    const total = exp?.total_cells ?? latestEvents.size;
    const buckets: Record<CellStatus, number> = { started: 0, completed: 0, failed: 0 };
    for (const ev of latestEvents.values()) buckets[ev.status]++;
    const completed = buckets.completed || exp?.completed_cells || 0;
    const failed = buckets.failed || exp?.failed_cells || 0;
    const started = buckets.started || exp?.running_cells || 0;
    const progress = total > 0 ? Math.min(1, (completed + failed) / total) : 0;
    return { total, started, completed, failed, progress };
  }, [run.data, latestEvents]);

  return {
    data: run.data?.experiment,
    error: run.error,
    loading: run.loading,
    reload: () => void run.reload(),
    counters,
    latestEvents,
    connected,
  };
}
