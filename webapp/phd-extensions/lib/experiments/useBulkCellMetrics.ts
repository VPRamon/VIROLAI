/**
 * `useBulkCellMetrics(slug, runId, cellIds)` — fetch metrics for many
 * cells in ONE request via `POST /cells/bulk`.
 *
 * This is the perf-critical hook for the experiments UI. The legacy
 * EST tabs fan out one request per schedule; that pattern is what
 * the user described as "very slow when uploading many schedules at
 * once". Replace it with this hook everywhere multiple cells are
 * rendered together (matrix, per-dataset table, …).
 *
 * Behaviour:
 *   - Debounces input changes by 50 ms so rapid prop churn (e.g.
 *     while a user is dragging a slider that filters the cell set)
 *     coalesces into a single backend call.
 *   - De-duplicates cell IDs so a `cellIds` of length N with
 *     repeats issues a request of size <=N.
 *   - Re-fetches on `(slug, runId, sortedCellIds)` change only.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { bulkCells } from "./api";
import type { BulkCellMetricsItem } from "./types";

const DEFAULT_DEBOUNCE_MS = 50;

export interface BulkCellMetricsState {
  data: Map<string, BulkCellMetricsItem>;
  loading: boolean;
  error: Error | undefined;
}

export interface UseBulkCellMetricsOptions {
  debounceMs?: number;
  /** Skip fetching entirely (e.g. while parent is still loading). */
  enabled?: boolean;
}

export function useBulkCellMetrics(
  slug: string,
  runId: string,
  cellIds: readonly string[],
  options: UseBulkCellMetricsOptions = {},
): BulkCellMetricsState {
  const { debounceMs = DEFAULT_DEBOUNCE_MS, enabled = true } = options;

  // Stable, deduped key derived from the input. Sorting+joining lets us
  // cheaply skip refetches when the *contents* are unchanged regardless
  // of order — the matrix re-orders cells on every pivot change.
  const cellIdKey = useMemo(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const id of cellIds) {
      if (id && !seen.has(id)) {
        seen.add(id);
        out.push(id);
      }
    }
    out.sort();
    return out.join("\0");
  }, [cellIds]);
  const dedupedCellIds = useMemo(
    () => (cellIdKey ? cellIdKey.split("\0") : []),
    [cellIdKey],
  );

  const [data, setData] = useState<Map<string, BulkCellMetricsItem>>(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | undefined>(undefined);
  const seq = useRef(0);

  useEffect(() => {
    if (!enabled || dedupedCellIds.length === 0) {
      setData(new Map());
      setLoading(false);
      setError(undefined);
      return;
    }

    const id = ++seq.current;
    setLoading(true);
    setError(undefined);

    const handle = setTimeout(() => {
      bulkCells(slug, runId, dedupedCellIds)
        .then((resp) => {
          if (id !== seq.current) return;
          const map = new Map<string, BulkCellMetricsItem>();
          for (const item of resp.items) map.set(item.cell_id, item);
          setData(map);
          setLoading(false);
        })
        .catch((err: unknown) => {
          if (id !== seq.current) return;
          setError(err instanceof Error ? err : new Error(String(err)));
          setLoading(false);
        });
    }, debounceMs);

    return () => clearTimeout(handle);
  }, [slug, runId, dedupedCellIds, debounceMs, enabled]);

  return { data, loading, error };
}
