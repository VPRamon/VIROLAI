import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  bulkCells,
  cancelRun,
  ExperimentsApiError,
  EXPERIMENTS_BASE,
  eventsUrl,
  getPareto,
  getRanking,
  getRun,
  listCells,
  listExperiments,
  resumeRun,
  setExperimentsClient,
  submitExperiment,
  summaryCsvUrl,
} from './api';
import type { AxiosInstance } from 'axios';
import { AxiosError } from 'axios';

interface Call {
  method: string;
  url: string;
  data?: unknown;
  params?: unknown;
}

function makeMockClient(impl?: (call: Call) => unknown): {
  client: AxiosInstance;
  calls: Call[];
} {
  const calls: Call[] = [];
  const handle = (method: string) => (url: string, a?: unknown, b?: unknown) => {
    const isWrite = method === 'post' || method === 'put' || method === 'patch';
    const data = isWrite ? a : undefined;
    const config = (isWrite ? b : a) as { params?: unknown } | undefined;
    const call: Call = { method, url, data, params: config?.params };
    calls.push(call);
    const result = impl?.(call);
    return Promise.resolve({ data: result ?? {} });
  };
  const client = {
    get: handle('get'),
    post: handle('post'),
  } as unknown as AxiosInstance;
  return { client, calls };
}

afterEach(() => {
  setExperimentsClient(null);
  vi.restoreAllMocks();
});

describe('experiments api request shapes', () => {
  it('listExperiments hits the index URL', async () => {
    const { client, calls } = makeMockClient(() => ({ experiments: [] }));
    setExperimentsClient(client);
    await listExperiments();
    expect(calls).toEqual([{ method: 'get', url: EXPERIMENTS_BASE, data: undefined, params: undefined }]);
  });

  it('getRun encodes path components', async () => {
    const { client, calls } = makeMockClient();
    setExperimentsClient(client);
    await getRun('slug with space', 'run/42');
    expect(calls[0].url).toBe(`${EXPERIMENTS_BASE}/slug%20with%20space/runs/run%2F42`);
  });

  it('listCells forwards query params', async () => {
    const { client, calls } = makeMockClient();
    setExperimentsClient(client);
    await listCells('s', 'r', { status: 'completed', limit: 10, offset: 5 });
    expect(calls[0].method).toBe('get');
    expect(calls[0].params).toEqual({ status: 'completed', limit: 10, offset: 5 });
  });

  it('bulkCells sends cell_ids in body', async () => {
    const { client, calls } = makeMockClient(() => ({ items: [] }));
    setExperimentsClient(client);
    await bulkCells('s', 'r', ['a', 'b', 'c']);
    expect(calls[0]).toMatchObject({
      method: 'post',
      url: `${EXPERIMENTS_BASE}/s/runs/r/cells/bulk`,
      data: { cell_ids: ['a', 'b', 'c'] },
    });
  });

  it('getPareto serialises axis params', async () => {
    const { client, calls } = makeMockClient();
    setExperimentsClient(client);
    await getPareto('s', 'r', { x: 'priority_sum', y: 'fragmentation_index', xmax: true, ymax: false });
    expect(calls[0].params).toEqual({ x: 'priority_sum', y: 'fragmentation_index', xmax: true, ymax: false });
  });

  it('getRanking flattens weights into params', async () => {
    const { client, calls } = makeMockClient();
    setExperimentsClient(client);
    await getRanking('s', 'r', {
      by: 'dataset',
      weights: { completion: 1, priority: 2, utilization: 0.5, fragmentation: 1 },
    });
    expect(calls[0].params).toEqual({
      by: 'dataset',
      completion: 1,
      priority: 2,
      utilization: 0.5,
      fragmentation: 1,
    });
  });

  it('submitExperiment posts the spec body', async () => {
    const { client, calls } = makeMockClient(() => ({ slug: 's', run_id: 'r' }));
    setExperimentsClient(client);
    const spec = { slug: 's', datasets: [], algorithms: ['est'] };
    const result = await submitExperiment(spec);
    expect(calls[0]).toMatchObject({ method: 'post', url: EXPERIMENTS_BASE, data: spec });
    expect(result).toEqual({ slug: 's', run_id: 'r' });
  });

  it('cancelRun and resumeRun POST to the right URL', async () => {
    const { client, calls } = makeMockClient();
    setExperimentsClient(client);
    await cancelRun('s', 'r');
    await resumeRun('s', 'r');
    expect(calls.map((c) => c.url)).toEqual([
      `${EXPERIMENTS_BASE}/s/runs/r/cancel`,
      `${EXPERIMENTS_BASE}/s/runs/r/resume`,
    ]);
  });

  it('exposes deterministic CSV and SSE URLs', () => {
    expect(summaryCsvUrl('s', 'r')).toBe(`${EXPERIMENTS_BASE}/s/runs/r/summary.csv`);
    expect(eventsUrl('s', 'r')).toBe(`${EXPERIMENTS_BASE}/s/runs/r/events`);
  });

  it('translates AxiosError into ExperimentsApiError', async () => {
    const client = {
      get: () =>
        Promise.reject(
          Object.assign(new AxiosError('boom'), {
            response: { status: 422, data: { error: 'bad spec' } },
          }),
        ),
    } as unknown as AxiosInstance;
    setExperimentsClient(client);
    await expect(listExperiments()).rejects.toMatchObject({
      name: 'ExperimentsApiError',
      status: 422,
      message: 'bad spec',
    });
    expect(new ExperimentsApiError('x')).toBeInstanceOf(Error);
  });
});
