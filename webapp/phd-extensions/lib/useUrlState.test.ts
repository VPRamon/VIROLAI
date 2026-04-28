import { afterEach, describe, expect, it } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import {
  booleanCodec,
  flushUrlState,
  numberListCodec,
  stringCodec,
  useUrlState,
} from './useUrlState';

function reset() {
  window.history.replaceState({}, '', '/');
  flushUrlState();
}

afterEach(reset);

describe('useUrlState', () => {
  it('returns the default value when the key is absent', () => {
    const { result } = renderHook(() => useUrlState('axis', 'x', { codec: stringCodec }));
    expect(result.current[0]).toBe('x');
  });

  it('reads an existing value from the URL', () => {
    window.history.replaceState({}, '', '/?axis=y');
    flushUrlState();
    const { result } = renderHook(() => useUrlState('axis', 'x', { codec: stringCodec }));
    expect(result.current[0]).toBe('y');
  });

  it('writes new values back to the URL', () => {
    const { result } = renderHook(() => useUrlState('axis', 'x', { codec: stringCodec }));
    act(() => result.current[1]('z'));
    flushUrlState();
    expect(window.location.search).toBe('?axis=z');
  });

  it('removes the key when set back to the default sentinel (empty string)', () => {
    window.history.replaceState({}, '', '/?axis=z');
    flushUrlState();
    const { result } = renderHook(() => useUrlState('axis', 'x', { codec: stringCodec }));
    act(() => result.current[1](''));
    flushUrlState();
    expect(window.location.search).toBe('');
  });

  it('shares state between two hooks under the same key', () => {
    const a = renderHook(() => useUrlState('flag', false, { codec: booleanCodec }));
    const b = renderHook(() => useUrlState('flag', false, { codec: booleanCodec }));
    act(() => a.result.current[1](true));
    flushUrlState();
    expect(a.result.current[0]).toBe(true);
    expect(b.result.current[0]).toBe(true);
  });

  it('roundtrips numberListCodec sorted-uniqued', () => {
    const { result } = renderHook(() =>
      useUrlState<number[]>('focus', [], { codec: numberListCodec }),
    );
    act(() => result.current[1]([7, 1, 1, 12]));
    flushUrlState();
    expect(window.location.search).toBe('?focus=1%2C7%2C12');
    expect(result.current[0]).toEqual([1, 7, 12]);
  });

  it('functional updates receive the latest value', () => {
    const { result } = renderHook(() =>
      useUrlState<number[]>('focus', [], { codec: numberListCodec }),
    );
    act(() => result.current[1]([1, 2]));
    flushUrlState();
    act(() => result.current[1]((prev) => [...prev, 3]));
    flushUrlState();
    expect(result.current[0]).toEqual([1, 2, 3]);
  });
});
