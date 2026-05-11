/**
 * Pareto tab — scatter of any two metrics, Pareto-front computed client-side,
 * colored by algorithm.
 */
import { useMemo, useState } from 'react';
import Plot from 'react-plotly.js';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import type { ManifestSummary } from '../../../lib/workspaces/types';
import {
  Card,
  EmptyState,
  PLOTLY_DARK_LAYOUT,
  PLOTLY_DEFAULT_CONFIG,
  Select,
  Skeleton,
} from '../../experiments/_ui';

type MetricKey =
  | 'completion_ratio'
  | 'utilization'
  | 'composite_rank_score'
  | 'priority_sum'
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

function metricValue(s: ManifestSummary, key: MetricKey): number | null {
  switch (key) {
    case 'completion_ratio': return s.completion_ratio;
    case 'utilization': return s.utilization;
    case 'composite_rank_score': return s.composite_rank_score;
    case 'priority_sum': return s.priority_sum;
    case 'fragmentation_index': return s.fragmentation_index;
    case 'scheduled_task_count': return s.scheduled_task_count;
  }
}

/** Returns the indices of the Pareto-front for the given points. */
function paretoFront(
  points: { x: number; y: number }[],
  xmax: boolean,
  ymax: boolean,
): Set<number> {
  const front = new Set<number>();
  for (let i = 0; i < points.length; i++) {
    let dominated = false;
    for (let j = 0; j < points.length; j++) {
      if (i === j) continue;
      const { x: xi, y: yi } = points[i];
      const { x: xj, y: yj } = points[j];
      const xBetter = xmax ? xj >= xi : xj <= xi;
      const yBetter = ymax ? yj >= yi : yj <= yi;
      const xStrictly = xmax ? xj > xi : xj < xi;
      const yStrictly = ymax ? yj > yi : yj < yi;
      if (xBetter && yBetter && (xStrictly || yStrictly)) {
        dominated = true;
        break;
      }
    }
    if (!dominated) front.add(i);
  }
  return front;
}

export default function ParetoTab() {
  const { summaries, loading } = useWorkspaceContext();
  const [x, setX] = useState<MetricKey>('completion_ratio');
  const [y, setY] = useState<MetricKey>('fragmentation_index');
  const [xmax, setXmax] = useState(true);
  const [ymax, setYmax] = useState(false);

  const points = useMemo(() => {
    return summaries
      .map((s) => ({
        id: s.manifest_id,
        label: s.display_name,
        algo: s.algorithm_id,
        x: metricValue(s, x),
        y: metricValue(s, y),
      }))
      .filter((p): p is typeof p & { x: number; y: number } => p.x !== null && p.y !== null);
  }, [summaries, x, y]);

  const frontIndices = useMemo(
    () => paretoFront(points, xmax, ymax),
    [points, xmax, ymax],
  );

  const algos = useMemo(() => [...new Set(points.map((p) => p.algo))].sort(), [points]);

  if (loading && summaries.length === 0) {
    return <Skeleton className="h-96 w-full" />;
  }

  const traces = algos.map((algo) => {
    const subset = points
      .map((p, i) => ({ ...p, i }))
      .filter((p) => p.algo === algo);
    return {
      type: 'scatter' as const,
      mode: 'markers' as const,
      name: algo,
      x: subset.map((p) => p.x),
      y: subset.map((p) => p.y),
      text: subset.map((p) => p.label),
      marker: {
        size: subset.map((p) => (frontIndices.has(p.i) ? 14 : 8)),
        line: subset.map((p) =>
          frontIndices.has(p.i) ? { width: 2, color: '#fbbf24' } : { width: 0 },
        ),
        opacity: subset.map((p) => (frontIndices.has(p.i) ? 1 : 0.7)),
      } as never,
      hovertemplate:
        '%{text}<br>x=%{x:.4g}<br>y=%{y:.4g}<extra>' + algo + '</extra>',
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
          Larger highlighted points are on the Pareto front (client-side). Each colour is one
          algorithm.
        </p>
      </Card>

      <Card>
        {points.length === 0 ? (
          <EmptyState title="Not enough data" description="Add manifests to compare." />
        ) : (
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
