/**
 * useUrlState — mirror a small piece of UI state into the URL query string.
 *
 * State changes are written back to the URL via `history.replaceState` (no
 * navigation entries) on a 150 ms debounce so rapid slider/keystroke updates
 * don't spam the browser's history stack. All instances using the same `key`
 * share state through a module-level pub/sub backed by `useSyncExternalStore`,
 * which means values persist across tab switches and across every component
 * mount/unmount inside the same SPA session.
 *
 * Codecs:
 *   - default codec is JSON; pass a custom codec for compact representations
 *     (booleans, comma-separated lists, etc.).
 *   - When the URL is missing the key, `defaultValue` is returned without a
 *     write-back (so deep links stay clean).
 */
import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';

export interface UrlStateCodec<T> {
  parse: (raw: string) => T | undefined;
  serialize: (value: T) => string | null; // null ⇒ remove the key
}

/** Default JSON codec; fine for plain objects and primitive values. */
export const jsonCodec = <T,>(): UrlStateCodec<T> => ({
  parse(raw) {
    try {
      return JSON.parse(raw) as T;
    } catch {
      return undefined;
    }
  },
  serialize(value) {
    if (value === undefined || value === null) return null;
    return JSON.stringify(value);
  },
});

/** Codec for boolean flags (`?key=1` / `?key=0`). */
export const booleanCodec: UrlStateCodec<boolean> = {
  parse(raw) {
    if (raw === '1' || raw === 'true') return true;
    if (raw === '0' || raw === 'false') return false;
    return undefined;
  },
  serialize(value) {
    return value ? '1' : null;
  },
};

/** Codec for short strings (passed through; no quoting). */
export const stringCodec: UrlStateCodec<string> = {
  parse(raw) {
    return raw;
  },
  serialize(value) {
    return value === '' ? null : value;
  },
};

/** Codec for a sorted-unique list of finite numbers (`?key=1,7,12`). */
export const numberListCodec: UrlStateCodec<number[]> = {
  parse(raw) {
    const out: number[] = [];
    for (const piece of raw.split(',')) {
      const n = Number(piece);
      if (Number.isFinite(n)) out.push(n);
    }
    return out;
  },
  serialize(value) {
    if (!value || value.length === 0) return null;
    return [...new Set(value)].sort((a, b) => a - b).join(',');
  },
};

/** Codec for a number with a sane fallback when parsing fails. */
export const numberCodec: UrlStateCodec<number> = {
  parse(raw) {
    const n = Number(raw);
    return Number.isFinite(n) ? n : undefined;
  },
  serialize(value) {
    return Number.isFinite(value) ? String(value) : null;
  },
};

// ---------------------------------------------------------------------------
// Module-level store: keeps every consumer of the same key in sync (across
// tabs, across mount/unmount) without forcing all callers under one provider.
// ---------------------------------------------------------------------------

type Listener = () => void;
const listeners = new Set<Listener>();

function readSearchParams(): URLSearchParams {
  if (typeof window === 'undefined') return new URLSearchParams();
  return new URLSearchParams(window.location.search);
}

let cachedSearch = typeof window === 'undefined' ? '' : window.location.search;
let cachedParams = readSearchParams();

function refreshCache() {
  if (typeof window === 'undefined') return;
  if (window.location.search === cachedSearch) return;
  cachedSearch = window.location.search;
  cachedParams = readSearchParams();
}

function notify() {
  for (const listener of [...listeners]) listener();
}

function handlePopState() {
  // External URL change (back/forward); resync cache and notify subscribers.
  refreshCache();
  notify();
}

function subscribe(listener: Listener) {
  listeners.add(listener);
  if (typeof window !== 'undefined' && listeners.size === 1) {
    window.addEventListener('popstate', handlePopState);
  }
  return () => {
    listeners.delete(listener);
    if (typeof window !== 'undefined' && listeners.size === 0) {
      window.removeEventListener('popstate', handlePopState);
    }
  };
}

let pendingTimer: ReturnType<typeof setTimeout> | null = null;
let pendingParams: URLSearchParams | null = null;
const DEBOUNCE_MS = 150;

function commitPending() {
  if (!pendingParams || typeof window === 'undefined') return;
  const search = pendingParams.toString();
  pendingParams = null;
  const newUrl = `${window.location.pathname}${search ? '?' + search : ''}${window.location.hash}`;
  window.history.replaceState(window.history.state, '', newUrl);
}

function scheduleWrite(next: URLSearchParams) {
  pendingParams = next;
  if (pendingTimer) return;
  pendingTimer = setTimeout(() => {
    pendingTimer = null;
    commitPending();
    notify();
  }, DEBOUNCE_MS);
}

function getRaw(key: string): string | null {
  return cachedParams.get(key);
}

function setRaw(key: string, value: string | null) {
  const next = new URLSearchParams(cachedParams);
  if (value === null) next.delete(key);
  else next.set(key, value);
  // Optimistic in-memory commit so synchronous reads observe the new value
  // before the debounced history.replaceState fires.
  cachedParams = next;
  cachedSearch = next.toString() ? `?${next.toString()}` : '';
  scheduleWrite(next);
  notify();
}

/** Synchronously flush any pending URL write — useful in tests. */
export function flushUrlState() {
  if (pendingTimer) {
    clearTimeout(pendingTimer);
    pendingTimer = null;
  }
  commitPending();
  refreshCache();
  notify();
}

export interface UseUrlStateOptions<T> {
  codec?: UrlStateCodec<T>;
}

export function useUrlState<T>(
  key: string,
  defaultValue: T,
  options: UseUrlStateOptions<T> = {},
): [T, (next: T | ((prev: T) => T)) => void] {
  const codec = options.codec ?? jsonCodec<T>();
  const codecRef = useRef(codec);
  codecRef.current = codec;

  const getSnapshot = useCallback(() => getRaw(key), [key]);
  const raw = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  const value = useMemo<T>(() => {
    if (raw == null) return defaultValue;
    const parsed = codecRef.current.parse(raw);
    return parsed === undefined ? defaultValue : parsed;
  }, [raw, defaultValue]);

  const valueRef = useRef(value);
  useEffect(() => {
    valueRef.current = value;
  }, [value]);

  const setValue = useCallback(
    (next: T | ((prev: T) => T)) => {
      const resolved =
        typeof next === 'function' ? (next as (p: T) => T)(valueRef.current) : next;
      const serialized = codecRef.current.serialize(resolved);
      setRaw(key, serialized);
    },
    [key],
  );

  return [value, setValue];
}
