/**
 * Matrix tab — heatmap of (dataset × algorithm/config) cells, coloured
 * by a user-selected metric. Uses the bulk-cells endpoint so a 50×50
 * matrix is a single round-trip.
 */
import { useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import Plot from 'react-plotly.js';
import { useExperimentRunContext } from '../ExperimentDetailPage';
import { useBulkCellMetrics } from '../../../lib/experiments/useBulkCellMetrics';
import type { ScheduleMetrics } from '../../../lib/experiments/types';
import {
  Card,
  EmptyState,
  PLOTLY_DARK_LAYOUT,
  PLOTLY_DEFAULT_CONFIG,
  Select,
  Skeleton,
  fmtNumber,
} from '../_ui';

type MetricKey =
  | 'composite_rank_score'
  | 'priority_sum'
  | 'fragmentation_index'
  | 'scheduled_task_count'
  | 'completion_ratio'
  | 'utilization';

const METRICS: ReadonlyArray<{ value: MetricKey; label: string }> = [
  { value: 'composite_rank_score', label: 'Composite score' },
  { value: 'completion_ratio', label: 'Completion ratio' },
  { value: 'priority_sum', label: 'Priority sum' },
  { value: 'utilization', label: 'Utilization' },
  { value: 'fragmentation_index', label: 'Fragmentation index' },
  { value: 'scheduled_task_count', label: 'Scheduled tasks' },
];

function extractMetric(metrics: ScheduleMetrics | undefined, key: MetricKey): number | null {
  if (!metrics) return null;
  switch (key) {
    case 'composite_rank_score':
      return metrics.composite_rank_score;
    case 'completion_ratio':
      return metrics.completion_ratio;
    case 'priority_sum':
      return metrics.priority.sum;
    case 'utilization':
      return metrics.utilization;
    case 'fragmentation_index':
      return metrics.fragmentation.fragmentation_index;
    case 'scheduled_task_count':
      return metrics.scheduled_task_count;
  }
}

export default function MatrixTab() {
  const { slug = '', runId = '' } = useParams();
  const navigate = useNavigate();
  const { data } = useExperimentRunContext();
  const cells = data?.cells ?? [];
  const [metric, setMetric] = useState<MetricKey>('composite_rank_score');

  const completedIds = useMemo(
    () => cells.filter((c) => c.status === 'completed').map((c) => c.cell_id),
    [cells],
  );
  const { data: bulk, loading } = useBulkCellMetrics(slug, runId, completedIds);

  const heatmap = useMemo(() => {
    const rows = new Set<string>();
    const cols = new Set<string>();
    type CellMeta = { id: string; value: number | null };
    const grid = new Map<string, Map<string, CellMeta>>();
    for (const cell of cells) {
      const row = cell.dataset_id ?? 'unknown';
      const col = `${cell.algorithm ?? 'unknown'}${cell.config_slug ? `/${cell.config_slug}` : ''}`;
      rows.add(row);
      cols.add(col);
      const m = bulk.get(cell.cell_id)?.metrics;
      const value = extractMetric(m, metric);
      let r = grid.get(row);
      if (!r) {
        r = new Map();
        grid.set(row, r);
      }
      r.set(col, { id: cell.cell_id, value });
    }
    const rowList = [...rows].sort();
    const colList = [...cols].sort();
    const z: (number | null)[][] = rowList.map((rk) =>
      colList.map((ck) => grid.get(rk)?.get(ck)?.value ?? null),
    );
    const idGrid: (string | null)[][] = rowList.map((rk) =>
      colList.map((ck) => grid.get(rk)?.get(ck)?.id ?? null),
    );
    const text: string[][] = z.map((row) => row.map((v) => (v == null ? '—' : fmtNumber(v))));
    return { rowList, colList, z, idGrid, text };
  }, [cells, bulk, metric]);

  if (!data) {
    return <Skeleton className="h-96 w-full" />;
  }

  if (cells.length === 0) {
    return <EmptyState title="No cells yet" description="The matrix populates once the runner emits state events." />;
  }

  const empty = heatmap.z.every((row) => row.every((v) => v == null));

  return (
    <div className="space-y-4">
      <Card padded>
        <div className="flex flex-wrap items-end gap-4">
          <div className="min-w-[220px]">
            <Select
              label="Color by"
              value={metric}
              onChange={(v) => setMetric(v)}
              options={METRICS}
            />
          </div>
          <p className="ml-auto text-xs text-slate-500">
            {completedIds.length} completed · {cells.length} total cells
          </p>
        </div>
      </Card>
      <Card>
        {empty && loading ? (
          <Skeleton className="h-96 w-full" />
        ) : empty ? (
          <EmptyState
            title="No completed metrics yet"
            description="The heatmap will fill in as cells finish."
          />
        ) : (
          <Plot
            data={[
              {
                type: 'heatmap',
                z: heatmap.z,
                x: heatmap.colList,
                y: heatmap.rowList,
                text: heatmap.text,
                texttemplate: '%{text}',
                hovertemplate: 'dataset=%{y}<br>config=%{x}<br>value=%{z}<extra></extra>',
                colorscale: 'Viridis',
                showscale: true,
              } as never,
            ]}
            layout={{
              ...PLOTLY_DARK_LAYOUT,
              height: Math.max(360, heatmap.rowList.length * 32 + 80),
              margin: { l: 140, r: 24, t: 24, b: 100 },
              xaxis: { ...PLOTLY_DARK_LAYOUT.xaxis, tickangle: -35, automargin: true },
              yaxis: { ...PLOTLY_DARK_LAYOUT.yaxis, automargin: true },
            }}
            config={PLOTLY_DEFAULT_CONFIG}
            style={{ width: '100%' }}
            onClick={(ev: { points?: ReadonlyArray<{ x?: unknown; y?: unknown }> }) => {
              const pt = ev.points?.[0];
              if (!pt) return;
              const x = pt.x as string;
              const y = pt.y as string;
              const ri = heatmap.rowList.indexOf(y);
              const ci = heatmap.colList.indexOf(x);
              const id = ri >= 0 && ci >= 0 ? heatmap.idGrid[ri][ci] : null;
              if (id) {
                navigate(
                  `/experiments/${encodeURIComponent(slug)}/${encodeURIComponent(runId)}/cells/${encodeURIComponent(id)}`,
                );
              }
            }}
          />
        )}
      </Card>
    </div>
  );
}
