/**
 * Tiny async-data hook. We deliberately do NOT pull in
 * `@tanstack/react-query` here even though TSI ships it: the
 * extension contract bans imports of TSI internals, and react-query
 * is most useful when paired with a shared `QueryClient`. For the
 * few endpoints the experiments UI consumes, this hook is enough.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

export interface AsyncState<T> {
  data: T | undefined;
  error: Error | undefined;
  loading: boolean;
  /** Re-runs the loader, returning the in-flight promise. */
  reload: () => Promise<T | undefined>;
}

/**
 * Run `load()` whenever any item in `deps` changes. The returned
 * `reload` handle re-invokes `load()` against the latest closure and
 * cancels any in-flight result that resolves after a newer call has
 * started (so the UI never flashes stale data).
 */
export function useAsync<T>(load: () => Promise<T>, deps: unknown[]): AsyncState<T> {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<Error | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const seq = useRef(0);
  const loadRef = useRef(load);
  loadRef.current = load;

  const reload = useCallback(async () => {
    const id = ++seq.current;
    setLoading(true);
    setError(undefined);
    try {
      const value = await loadRef.current();
      if (id === seq.current) {
        setData(value);
        setLoading(false);
      }
      return value;
    } catch (err) {
      if (id === seq.current) {
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      }
      return undefined;
    }
    // deps controlled by caller; lint disabled at call-site via deps array
  }, []);

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return { data, error, loading, reload };
}
