/**
 * Overview tab — KPIs + top-3 cells + distribution charts.
 */
import { useMemo } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import Plot from 'react-plotly.js';
import { useExperimentRunContext } from '../ExperimentDetailPage';
import { useBulkCellMetrics } from '../../../lib/experiments/useBulkCellMetrics';
import {
  Card,
  EmptyState,
  MetricBadge,
  PLOTLY_DARK_LAYOUT,
  PLOTLY_DEFAULT_CONFIG,
  Skeleton,
  fmtNumber,
  fmtPercent,
} from '../_ui';

export default function OverviewTab() {
  const { slug = '', runId = '' } = useParams();
  const navigate = useNavigate();
  const { data, counters } = useExperimentRunContext();
  const cells = data?.cells ?? [];

  const cellIds = useMemo(
    () => cells.filter((c) => c.status === 'completed').map((c) => c.cell_id),
    [cells],
  );
  const { data: metrics, loading: metricsLoading } = useBulkCellMetrics(slug, runId, cellIds);

  const completed = useMemo(
    () =>
      cellIds
        .map((id) => ({ id, m: metrics.get(id)?.metrics }))
        .filter((x): x is { id: string; m: NonNullable<typeof x.m> } => !!x.m),
    [cellIds, metrics],
  );

  const top3 = useMemo(
    () =>
      [...completed]
        .sort((a, b) => b.m.composite_rank_score - a.m.composite_rank_score)
        .slice(0, 3),
    [completed],
  );

  const prioritySums = completed.map((c) => c.m.scheduled_priority.sum);
  const fragIndices = completed.map((c) => c.m.fragmentation.fragmentation_index);

  if (!data) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <MetricBadge label="Total cells" value={counters.total} />
        <MetricBadge label="Completed" value={counters.completed} tone="positive" />
        <MetricBadge
          label="Failed"
          value={counters.failed}
          tone={counters.failed > 0 ? 'negative' : 'default'}
        />
        <MetricBadge
          label="Progress"
          value={fmtPercent(counters.progress, 0)}
          tone={counters.progress >= 1 ? 'positive' : 'warning'}
        />
      </div>

      <Card>
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
          Top cells by composite score
        </h3>
        {metricsLoading && completed.length === 0 ? (
          <Skeleton className="h-20 w-full" />
        ) : top3.length === 0 ? (
          <EmptyState title="No completed cells yet" description="Charts populate as cells finish." />
        ) : (
          <ol className="divide-y divide-slate-700">
            {top3.map((c, idx) => (
              <li
                key={c.id}
                className="flex cursor-pointer items-center justify-between py-3 hover:text-indigo-200"
                onClick={() =>
                  navigate(
                    `/experiments/${encodeURIComponent(slug)}/${encodeURIComponent(runId)}/cells/${encodeURIComponent(c.id)}`,
                  )
                }
              >
                <div className="flex items-center gap-3">
                  <span className="inline-flex size-7 items-center justify-center rounded-full bg-indigo-500/15 text-xs font-semibold text-indigo-300">
                    {idx + 1}
                  </span>
                  <div>
                    <div className="text-sm font-medium text-white">{c.id}</div>
                    <div className="text-xs text-slate-400">
                      completion {fmtPercent(c.m.scheduled_task_ratio)} · util {fmtPercent(c.m.utilization)}
                    </div>
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-sm font-semibold tabular-nums text-emerald-300">
                    {fmtNumber(c.m.composite_rank_score)}
                  </div>
                  <div className="text-xs text-slate-500">composite</div>
                </div>
              </li>
            ))}
          </ol>
        )}
      </Card>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
            Priority sum distribution
          </h3>
          {prioritySums.length === 0 ? (
            <EmptyState title="No data yet" />
          ) : (
            <Plot
              data={[{ type: 'histogram', x: prioritySums, marker: { color: '#818cf8' }, nbinsx: 20 } as never]}
              layout={{
                ...PLOTLY_DARK_LAYOUT,
                height: 260,
                xaxis: { ...PLOTLY_DARK_LAYOUT.xaxis, title: { text: 'Priority sum' } } as never,
                yaxis: { ...PLOTLY_DARK_LAYOUT.yaxis, title: { text: 'Cells' } } as never,
              }}
              config={PLOTLY_DEFAULT_CONFIG}
              style={{ width: '100%' }}
            />
          )}
        </Card>
        <Card>
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
            Fragmentation index distribution
          </h3>
          {fragIndices.length === 0 ? (
            <EmptyState title="No data yet" />
          ) : (
            <Plot
              data={[{ type: 'histogram', x: fragIndices, marker: { color: '#fbbf24' }, nbinsx: 20 } as never]}
              layout={{
                ...PLOTLY_DARK_LAYOUT,
                height: 260,
                xaxis: { ...PLOTLY_DARK_LAYOUT.xaxis, title: { text: 'Fragmentation index' } } as never,
                yaxis: { ...PLOTLY_DARK_LAYOUT.yaxis, title: { text: 'Cells' } } as never,
              }}
              config={PLOTLY_DEFAULT_CONFIG}
              style={{ width: '100%' }}
            />
          )}
        </Card>
      </div>
    </div>
  );
}
