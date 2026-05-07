/**
 * Pareto tab — scatter of any two metrics with maximize/minimize
 * toggles, highlighting the server-computed Pareto front.
 */
import { useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import Plot from 'react-plotly.js';
import { useExperimentRunContext } from '../ExperimentDetailPage';
import { getPareto } from '../../../lib/experiments/api';
import { useAsync } from '../../../lib/experiments/useAsync';
import { useBulkCellMetrics } from '../../../lib/experiments/useBulkCellMetrics';
import type { ScheduleMetrics } from '../../../lib/experiments/types';
import {
  Card,
  EmptyState,
  ErrorState,
  PLOTLY_DARK_LAYOUT,
  PLOTLY_DEFAULT_CONFIG,
  Select,
  Skeleton,
} from '../_ui';

type MetricKey =
  | 'composite_rank_score'
  | 'completion_ratio'
  | 'priority_sum'
  | 'utilization'
  | 'fragmentation_index'
  | 'scheduled_task_count';

const METRICS: ReadonlyArray<{ value: MetricKey; label: string }> = [
  { value: 'composite_rank_score', label: 'Composite score' },
  { value: 'completion_ratio', label: 'Completion ratio' },
  { value: 'priority_sum', label: 'Priority sum' },
  { value: 'utilization', label: 'Utilization' },
  { value: 'fragmentation_index', label: 'Fragmentation index' },
  { value: 'scheduled_task_count', label: 'Scheduled tasks' },
];

function metricValue(m: ScheduleMetrics | undefined, key: MetricKey): number | null {
  if (!m) return null;
  switch (key) {
    case 'composite_rank_score': return m.composite_rank_score;
    case 'completion_ratio': return m.completion_ratio;
    case 'priority_sum': return m.priority.sum;
    case 'utilization': return m.utilization;
    case 'fragmentation_index': return m.fragmentation.fragmentation_index;
    case 'scheduled_task_count': return m.scheduled_task_count;
  }
}

export default function ParetoTab() {
  const { slug = '', runId = '' } = useParams();
  const navigate = useNavigate();
  const { data } = useExperimentRunContext();
  const [x, setX] = useState<MetricKey>('priority_sum');
  const [y, setY] = useState<MetricKey>('fragmentation_index');
  const [xmax, setXmax] = useState(true);
  const [ymax, setYmax] = useState(false);

  const completedIds = useMemo(
    () => (data?.cells ?? []).filter((c) => c.status === 'completed').map((c) => c.cell_id),
    [data],
  );
  const { data: bulk } = useBulkCellMetrics(slug, runId, completedIds);

  const pareto = useAsync(
    () => getPareto(slug, runId, { x, y, xmax, ymax }),
    [slug, runId, x, y, xmax, ymax],
  );

  const algoOf = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of data?.cells ?? []) m.set(c.cell_id, c.algorithm ?? 'unknown');
    return m;
  }, [data]);

  const points = useMemo(() => {
    const rows: { id: string; x: number; y: number; algo: string }[] = [];
    for (const id of completedIds) {
      const m = bulk.get(id)?.metrics;
      const xv = metricValue(m, x);
      const yv = metricValue(m, y);
      if (xv == null || yv == null) continue;
      rows.push({ id, x: xv, y: yv, algo: algoOf.get(id) ?? 'unknown' });
    }
    return rows;
  }, [completedIds, bulk, x, y, algoOf]);

  const frontIds = useMemo(() => new Set((pareto.data?.front ?? []).map((p) => p.cell_id)), [pareto.data]);
  const algos = useMemo(() => [...new Set(points.map((p) => p.algo))].sort(), [points]);

  if (!data) return <Skeleton className="h-96 w-full" />;

  const traces = algos.map((algo) => {
    const subset = points.filter((p) => p.algo === algo);
    return {
      type: 'scatter' as const,
      mode: 'markers' as const,
      name: algo,
      x: subset.map((p) => p.x),
      y: subset.map((p) => p.y),
      text: subset.map((p) => p.id),
      marker: {
        size: subset.map((p) => (frontIds.has(p.id) ? 14 : 8)),
        line: subset.map((p) => (frontIds.has(p.id) ? { width: 2, color: '#fbbf24' } : { width: 0 })),
        opacity: subset.map((p) => (frontIds.has(p.id) ? 1 : 0.7)),
      } as never,
      hovertemplate: '%{text}<br>x=%{x:.4g}<br>y=%{y:.4g}<extra>' + algo + '</extra>',
    };
  });

  return (
    <div className="space-y-4">
      <Card padded>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-4">
          <Select label="X axis" value={x} onChange={(v) => setX(v)} options={METRICS} />
          <ToggleField label="Maximize X" value={xmax} onChange={setXmax} />
          <Select label="Y axis" value={y} onChange={(v) => setY(v)} options={METRICS} />
          <ToggleField label="Maximize Y" value={ymax} onChange={setYmax} />
        </div>
        <p className="mt-3 text-xs text-slate-500">
          Larger highlighted points are on the Pareto front. Click a point to inspect the cell.
        </p>
      </Card>
      <Card>
        {pareto.error && <ErrorState error={pareto.error} onRetry={pareto.reload} />}
        {!pareto.error && points.length === 0 && (
          <EmptyState title="No comparable cells yet" />
        )}
        {!pareto.error && points.length > 0 && (
          <Plot
            data={traces as never}
            layout={{
              ...PLOTLY_DARK_LAYOUT,
              height: 480,
              xaxis: { ...PLOTLY_DARK_LAYOUT.xaxis, title: { text: x } } as never,
              yaxis: { ...PLOTLY_DARK_LAYOUT.yaxis, title: { text: y } } as never,
              legend: { orientation: 'h', y: -0.18 },
            }}
            config={PLOTLY_DEFAULT_CONFIG}
            style={{ width: '100%' }}
            onClick={(ev: { points?: ReadonlyArray<{ text?: unknown }> }) => {
              const id = ev.points?.[0]?.text as string | undefined;
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

function ToggleField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-3 self-end pb-1">
      <input
        type="checkbox"
        checked={value}
        onChange={(e) => onChange(e.target.checked)}
        className="size-4 accent-indigo-500"
      />
      <span className="text-sm text-slate-200">{label}</span>
    </label>
  );
}
