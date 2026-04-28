import { afterEach, describe, expect, it } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { focusIdsFromSelection, useRunFocus } from './useRunFocus';
import { flushUrlState } from '../../../lib/useUrlState';
import type { RunRow } from './useRunMatrix';

function row(id: number): RunRow {
  return {
    schedule: {
      schedule_id: id,
      schedule_name: `s${id}`,
      schedule_metadata: undefined,
    },
    insights: undefined,
    iterations: undefined,
    algorithmConfig: undefined,
    traceSummary: undefined,
  } as unknown as RunRow;
}

afterEach(() => {
  // useRunFocus is now URL-backed (so it persists across tabs); reset between tests.
  window.history.replaceState({}, '', '/');
  flushUrlState();
});

describe('useRunFocus', () => {
  it('starts inactive and treats apply() as identity', () => {
    const { result } = renderHook(() => useRunFocus());
    expect(result.current.active).toBe(false);
    expect(result.current.focused.size).toBe(0);
    const runs = [row(1), row(2), row(3)];
    expect(result.current.apply(runs)).toEqual(runs);
  });

  it('toggle adds/removes ids and updates active flag', () => {
    const { result } = renderHook(() => useRunFocus());
    act(() => result.current.toggle(7));
    expect(result.current.active).toBe(true);
    expect(result.current.isFocused(7)).toBe(true);
    act(() => result.current.toggle(7));
    expect(result.current.active).toBe(false);
  });

  it('apply() filters runs to focused ids', () => {
    const { result } = renderHook(() => useRunFocus());
    const runs = [row(1), row(2), row(3)];
    act(() => result.current.setFocused([2, 3]));
    expect(result.current.apply(runs).map((r) => r.schedule.schedule_id)).toEqual([2, 3]);
  });

  it('clear() returns to inactive', () => {
    const { result } = renderHook(() => useRunFocus());
    act(() => result.current.setFocused([1, 2]));
    act(() => result.current.clear());
    expect(result.current.active).toBe(false);
  });

  it('focusIdsFromSelection extracts numeric customdata', () => {
    const ev = {
      points: [
        { customdata: 1 },
        { customdata: 2 },
        { customdata: 'skip' },
        { customdata: 1 },
        {},
      ],
    };
    expect(focusIdsFromSelection(ev).sort()).toEqual([1, 2]);
    expect(focusIdsFromSelection(null)).toEqual([]);
    expect(focusIdsFromSelection({})).toEqual([]);
  });
});
