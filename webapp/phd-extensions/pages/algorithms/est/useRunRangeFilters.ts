/**
 * Shared EST configuration-range filter hook.
 *
 * Builds RangeFilterSpecs from the numeric dimensions discovered in a
 * batch of `algorithmConfig` blobs (via `extractDimensions`), tracks
 * the user's slider state, and returns a `runFilter` predicate the
 * panels can apply to drop runs outside the selected ranges.
 */
import { useMemo, useState } from 'react';
import type { RangeFilterSpec, RangeFilterValue } from '@/components';
import { initialRangeValues } from '@/components';
import { extractDimensions, readDimension, type Dimension } from '@/features/schedules/analytics/dimensions';
import type { RunRow } from './useRunMatrix';

export interface RunFilterState {
  specs: RangeFilterSpec[];
  values: Record<string, RangeFilterValue>;
  setValues: (next: Record<string, RangeFilterValue>) => void;
  /** Returns true when the run satisfies every active filter. */
  runFilter: (run: RunRow) => boolean;
  /** Subset of `runs` that pass the filter (memoised by the caller as needed). */
  filtered: RunRow[];
}

export function useRunRangeFilters(runs: RunRow[]): RunFilterState {
  const dims = useMemo(
    () => extractDimensions(runs.map((r) => r.algorithmConfig)),
    [runs],
  );

  const specs = useMemo<RangeFilterSpec[]>(
    () =>
      dims.numeric.map((d: Dimension) => ({
        key: d.key,
        label: d.key,
        values: d.values.filter((v): v is number => typeof v === 'number'),
      })),
    [dims],
  );

  const initial = useMemo(() => initialRangeValues(specs), [specs]);
  const [values, setValues] = useState<Record<string, RangeFilterValue>>(initial);

  // Re-seed when the spec set changes (e.g. selection swapped).
  const seedSignature = useMemo(
    () => specs.map((s) => `${s.key}:${s.values.length}`).join('|'),
    [specs],
  );
  const [lastSeed, setLastSeed] = useState(seedSignature);
  if (seedSignature !== lastSeed) {
    setLastSeed(seedSignature);
    setValues(initial);
  }

  const runFilter = (run: RunRow): boolean => {
    for (const dim of dims.numeric) {
      const range = values[dim.key];
      if (!range) continue;
      const v = readDimension(run.algorithmConfig, dim);
      if (typeof v !== 'number') continue;
      if (v < range.min || v > range.max) return false;
    }
    return true;
  };

  const filtered = useMemo(() => runs.filter(runFilter), [runs, values, dims]);

  return { specs, values, setValues, runFilter, filtered };
}
