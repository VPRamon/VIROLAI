import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { setExperimentsClient } from "./api";
import { useBulkCellMetrics } from "./useBulkCellMetrics";
import type { AxiosInstance } from "axios";

function mockBulk(handler: (ids: string[]) => unknown): {
  client: AxiosInstance;
  calls: string[][];
} {
  const calls: string[][] = [];
  const post = (_url: string, body: { cell_ids: string[] }) => {
    calls.push([...body.cell_ids]);
    return Promise.resolve({ data: handler(body.cell_ids) });
  };
  const client = { post } as unknown as AxiosInstance;
  return { client, calls };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  setExperimentsClient(null);
});

async function flushBulkRequest(ms = 60) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("useBulkCellMetrics", () => {
  it("issues exactly one request for many cell ids", async () => {
    const { client, calls } = mockBulk((ids) => ({
      items: ids.map((id) => ({ cell_id: id })),
    }));
    setExperimentsClient(client);
    const { result } = renderHook(() =>
      useBulkCellMetrics("s", "r", ["a", "b", "c"]),
    );
    await flushBulkRequest();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual(["a", "b", "c"]);
    expect(result.current.data.size).toBe(3);
  });

  it("de-duplicates input ids before requesting", async () => {
    const { client, calls } = mockBulk((ids) => ({
      items: ids.map((id) => ({ cell_id: id })),
    }));
    setExperimentsClient(client);
    renderHook(() => useBulkCellMetrics("s", "r", ["a", "b", "a", "c", "b"]));
    await flushBulkRequest();
    expect(calls[0]).toEqual(["a", "b", "c"]);
  });

  it("debounces rapid input changes into one request", async () => {
    const { client, calls } = mockBulk((ids) => ({
      items: ids.map((id) => ({ cell_id: id })),
    }));
    setExperimentsClient(client);
    const { rerender } = renderHook(
      ({ ids }: { ids: string[] }) => useBulkCellMetrics("s", "r", ids),
      { initialProps: { ids: ["a"] } },
    );
    rerender({ ids: ["a", "b"] });
    rerender({ ids: ["a", "b", "c"] });
    await flushBulkRequest();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual(["a", "b", "c"]);
  });

  it("does not refetch when contents are reordered", async () => {
    const { client, calls } = mockBulk((ids) => ({
      items: ids.map((id) => ({ cell_id: id })),
    }));
    setExperimentsClient(client);
    const { rerender } = renderHook(
      ({ ids }: { ids: string[] }) => useBulkCellMetrics("s", "r", ids),
      { initialProps: { ids: ["a", "b", "c"] } },
    );
    await flushBulkRequest();
    rerender({ ids: ["c", "b", "a"] });
    await flushBulkRequest();
    expect(calls).toHaveLength(1);
  });

  it("skips the request when disabled or empty", async () => {
    const { client, calls } = mockBulk(() => ({ items: [] }));
    setExperimentsClient(client);
    const { result: empty } = renderHook(() =>
      useBulkCellMetrics("s", "r", []),
    );
    const { result: disabled } = renderHook(() =>
      useBulkCellMetrics("s", "r", ["a"], { enabled: false }),
    );
    await flushBulkRequest(100);
    expect(calls).toHaveLength(0);
    expect(empty.current.data.size).toBe(0);
    expect(disabled.current.data.size).toBe(0);
  });
});
