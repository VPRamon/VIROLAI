/**
 * Overview tab — KPI cards + completion/utilization/priority/fragmentation
 * distribution histograms. Reads from WorkspaceCtx (no extra fetches).
 */
import { useMemo } from 'react';
import Plot from 'react-plotly.js';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import {
  Card,
  EmptyState,
  MetricBadge,
  PLOTLY_DARK_LAYOUT,
  PLOTLY_DEFAULT_CONFIG,
  Skeleton,
  fmtNumber,
  fmtPercent,
} from '../../experiments/_ui';

export default function OverviewTab() {
  const { summaries, loading } = useWorkspaceContext();

  const completionVals = useMemo(
    () => summaries.map((s) => s.completion_ratio).filter((v): v is number => v !== null),
    [summaries],
  );
  const utilizationVals = useMemo(
    () => summaries.map((s) => s.utilization).filter((v): v is number => v !== null),
    [summaries],
  );
  const priorityVals = useMemo(
    () => summaries.map((s) => s.priority_sum).filter((v): v is number => v !== null),
    [summaries],
  );
  const fragmentationVals = useMemo(
    () => summaries.map((s) => s.fragmentation_index).filter((v): v is number => v !== null),
    [summaries],
  );

  const meanCompletion = mean(completionVals);
  const meanUtilization = mean(utilizationVals);
  const meanComposite = mean(
    summaries.map((s) => s.composite_rank_score).filter((v): v is number => v !== null),
  );
  const meanPriority = mean(priorityVals);

  if (loading && summaries.length === 0) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* KPI row */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <MetricBadge label="Manifests" value={summaries.length} />
        <MetricBadge
          label="Mean completion"
          value={fmtPercent(meanCompletion)}
          tone={meanCompletion !== null && meanCompletion >= 0.9 ? 'positive' : 'warning'}
        />
        <MetricBadge label="Mean utilization" value={fmtPercent(meanUtilization)} />
        <MetricBadge label="Mean composite" value={fmtNumber(meanComposite)} />
      </div>

      {summaries.length === 0 && (
        <EmptyState
          title="No manifests yet"
          description="Upload manifests or schedule JSONs in the Comparison tab."
        />
      )}

      {summaries.length > 0 && (
        <>
          {/* Distribution charts */}
          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            <HistChart
              title="Completion ratio"
              values={completionVals}
              color="#34d399"
              xLabel="Completion"
            />
            <HistChart
              title="Utilization"
              values={utilizationVals}
              color="#818cf8"
              xLabel="Utilization"
            />
            <HistChart
              title="Priority sum"
              values={priorityVals}
              color="#fbbf24"
              xLabel="Priority sum"
            />
            <HistChart
              title="Fragmentation index"
              values={fragmentationVals}
              color="#f87171"
              xLabel="Fragmentation index"
            />
          </div>
        </>
      )}
    </div>
  );
}

function HistChart({
  title,
  values,
  color,
  xLabel,
}: {
  title: string;
  values: number[];
  color: string;
  xLabel: string;
}) {
  return (
    <Card>
      <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
        {title}
      </h3>
      {values.length === 0 ? (
        <EmptyState title="No data" />
      ) : (
        <Plot
          data={[{ type: 'histogram', x: values, marker: { color }, nbinsx: 20 } as never]}
          layout={{
            ...PLOTLY_DARK_LAYOUT,
            height: 240,
            xaxis: { ...PLOTLY_DARK_LAYOUT.xaxis, title: { text: xLabel } } as never,
            yaxis: { ...PLOTLY_DARK_LAYOUT.yaxis, title: { text: 'Count' } } as never,
          }}
          config={PLOTLY_DEFAULT_CONFIG}
          style={{ width: '100%' }}
        />
      )}
    </Card>
  );
}

function mean(vals: number[]): number | null {
  if (vals.length === 0) return null;
  return vals.reduce((a, b) => a + b, 0) / vals.length;
}
