/**
 * useRunFocus — page-wide "focus set" of EST runs, persisted in the URL.
 *
 * Each EST panel composes this with {@link useRunRangeFilters}: range
 * filters drop runs that fall outside slider bounds; the focus set is a
 * strict subset of the filtered set, populated by user interactions
 * (lasso/box-select on a chart, table-row click, etc.).
 *
 * The set lives in a single shared URL parameter (`focus=1,7,12`), which
 * gives us three things at once:
 *   - it survives tab switches inside the EST workspace (the focus chip
 *     reappears on the new tab), and
 *   - it survives full page reloads, and
 *   - the URL is shareable as a deep link.
 *
 * When `focused.size === 0` the focus is treated as "not active" — every
 * filtered run is in scope, matching the panel's pre-focus behaviour.
 */
import { useCallback, useMemo } from 'react';
import { numberListCodec, useUrlState } from '../../../lib/useUrlState';
import type { RunRow } from './useRunMatrix';

const FOCUS_URL_KEY = 'focus';
const EMPTY_FOCUS: number[] = [];

export interface RunFocusState {
  /** Currently focused schedule ids; empty set ⇒ focus inactive. */
  focused: Set<number>;
  /** Replace the focus set wholesale. */
  setFocused: (ids: Iterable<number>) => void;
  /** Toggle a single id in/out of the focus set. */
  toggle: (id: number) => void;
  /** Clear the focus set ("show everything in the filter again"). */
  clear: () => void;
  /** True iff the id is in the focus set. */
  isFocused: (id: number) => boolean;
  /** True iff the focus is active (non-empty). */
  active: boolean;
  /**
   * Returns the subset of `runs` that respects the focus set: when the
   * focus is empty, returns `runs` unchanged; otherwise filters to runs
   * whose `schedule_id` is in the set.
   */
  apply: (runs: RunRow[]) => RunRow[];
}

export function useRunFocus(): RunFocusState {
  const [list, setList] = useUrlState<number[]>(FOCUS_URL_KEY, EMPTY_FOCUS, {
    codec: numberListCodec,
  });

  const focused = useMemo(() => new Set(list), [list]);

  const setFocused = useCallback(
    (ids: Iterable<number>) => {
      const next = [...new Set(ids)].filter((n) => Number.isFinite(n));
      setList(next);
    },
    [setList],
  );

  const toggle = useCallback(
    (id: number) => {
      setList((prev) => {
        const set = new Set(prev);
        if (set.has(id)) set.delete(id);
        else set.add(id);
        return [...set];
      });
    },
    [setList],
  );

  const clear = useCallback(() => {
    setList(EMPTY_FOCUS);
  }, [setList]);

  const isFocused = useCallback((id: number) => focused.has(id), [focused]);

  const apply = useCallback(
    (runs: RunRow[]): RunRow[] =>
      focused.size === 0 ? runs : runs.filter((r) => focused.has(r.schedule.schedule_id)),
    [focused],
  );

  return useMemo(
    () => ({
      focused,
      setFocused,
      toggle,
      clear,
      isFocused,
      active: focused.size > 0,
      apply,
    }),
    [focused, setFocused, toggle, clear, isFocused, apply],
  );
}

/**
 * Pull the schedule ids out of a Plotly `plotly_selected` event.  Each
 * trace built by the EST panels carries `customdata = scheduleId` per
 * point, so the event surfaces them via `pt.customdata`.
 */
export function focusIdsFromSelection(event: unknown): number[] {
  if (!event || typeof event !== 'object') return [];
  const points = (event as { points?: Array<{ customdata?: unknown }> }).points;
  if (!Array.isArray(points)) return [];
  const out = new Set<number>();
  for (const pt of points) {
    const cd = pt?.customdata;
    if (typeof cd === 'number' && Number.isFinite(cd)) out.add(cd);
  }
  return [...out];
}
