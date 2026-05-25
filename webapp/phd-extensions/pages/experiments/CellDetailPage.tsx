/**
 * `/experiments/:slug/:runId/cells/:cellId` — full breakdown of one
 * scheduled cell. Loaded as a route (rather than a side panel) so the
 * URL is shareable.
 */
import { Link, useParams } from 'react-router-dom';
import { getCell } from '../../lib/experiments/api';
import { useAsync } from '../../lib/experiments/useAsync';
import type { CellDetail, ScheduleMetrics } from '../../lib/experiments/types';
import {
  Button,
  Card,
  EmptyState,
  ErrorState,
  MetricBadge,
  Skeleton,
  StatusPill,
  fmtDate,
  fmtDuration,
  fmtNumber,
  fmtPercent,
} from './_ui';

export default function CellDetailPage() {
  const { slug = '', runId = '', cellId = '' } = useParams();
  const { data, error, loading, reload } = useAsync(
    () => getCell(slug, runId, cellId),
    [slug, runId, cellId],
  );
  const cell = data?.cell;
  const back = `/experiments/${encodeURIComponent(slug)}/${encodeURIComponent(runId)}/matrix`;

  if (loading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-8 w-1/2" />
        <Card>
          <Skeleton className="h-32 w-full" />
        </Card>
      </div>
    );
  }
  if (error) return <ErrorState error={error} onRetry={reload} />;
  if (!cell) return <EmptyState title="Cell not found" />;

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <div className="text-xs text-slate-500">
            <Link to={back} className="hover:text-indigo-300">
              ← Back to matrix
            </Link>
          </div>
          <h2 className="mt-1 text-xl font-semibold text-white">{cell.cell_id}</h2>
          <div className="mt-1 text-xs text-slate-400">
            {cell.dataset_id ?? '—'} · {cell.algorithm ?? '—'}
            {cell.config_slug ? ` · ${cell.config_slug}` : ''}
          </div>
        </div>
        <StatusPill kind={cell.status ?? 'unknown'} />
      </div>

      {cell.error && (
        <ErrorState title="Cell failed" error={cell.error} />
      )}

      {cell.metrics ? (
        <MetricsPanels metrics={cell.metrics} />
      ) : (
        <EmptyState
          title="No metrics yet"
          description="The cell hasn't produced a metrics file. This is expected while it's still running."
        />
      )}

      <SchedulePreview detail={cell} slug={slug} runId={runId} />
    </div>
  );
}

function MetricsPanels({ metrics }: { metrics: ScheduleMetrics }) {
  return (
    <>
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <MetricBadge
          label="Composite score"
          value={fmtNumber(metrics.composite_rank_score)}
          hint="Weighted across all axes"
          tone="positive"
        />
        <MetricBadge
          label="Completion"
          value={fmtPercent(metrics.scheduled_task_ratio)}
          hint={`${metrics.scheduled_task_count} / ${metrics.total_task_count} tasks`}
        />
        <MetricBadge
          label="Utilization"
          value={fmtPercent(metrics.utilization)}
          hint={`${fmtDuration(metrics.scheduled_time_sec)} of ${fmtDuration(metrics.available_time_sec)}`}
        />
        <MetricBadge
          label="Fragmentation"
          value={fmtNumber(metrics.fragmentation.fragmentation_index)}
          hint={`${metrics.fragmentation.gap_count} gaps`}
          tone="warning"
        />
      </div>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Card>
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
            Priority distribution
          </h3>
          <KvTable
            rows={[
              ['count', fmtNumber(metrics.scheduled_priority.count, 0)],
              ['sum', fmtNumber(metrics.scheduled_priority.sum)],
              ['mean', fmtNumber(metrics.scheduled_priority.mean)],
              ['std', fmtNumber(metrics.scheduled_priority.std)],
              ['min', fmtNumber(metrics.scheduled_priority.min)],
              ['p25', fmtNumber(metrics.scheduled_priority.p25)],
              ['p50', fmtNumber(metrics.scheduled_priority.p50)],
              ['p75', fmtNumber(metrics.scheduled_priority.p75)],
              ['p90', fmtNumber(metrics.scheduled_priority.p90)],
              ['max', fmtNumber(metrics.scheduled_priority.max)],
            ]}
          />
        </Card>
        <Card>
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
            Fragmentation
          </h3>
          <KvTable
            rows={[
              ['gap_count', fmtNumber(metrics.fragmentation.gap_count, 0)],
              ['gap_total', fmtDuration(metrics.fragmentation.gap_total_sec)],
              ['largest_gap', fmtDuration(metrics.fragmentation.largest_gap_sec)],
              ['fragmentation_index', fmtNumber(metrics.fragmentation.fragmentation_index)],
            ]}
          />
        </Card>
      </div>

      {metrics.per_resource.length > 0 && (
        <Card>
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
            Per resource
          </h3>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="text-xs uppercase tracking-wide text-slate-500">
                <tr>
                  <th className="px-2 py-2 text-left">Resource</th>
                  <th className="px-2 py-2 text-right">Tasks</th>
                  <th className="px-2 py-2 text-right">Time</th>
                  <th className="px-2 py-2 text-right">Priority Σ</th>
                  <th className="px-2 py-2 text-right">Util</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-700">
                {metrics.per_resource.map((r) => (
                  <tr key={r.resource_id} className="text-slate-200">
                    <td className="px-2 py-2 font-medium">{r.resource_id}</td>
                    <td className="px-2 py-2 text-right tabular-nums">{r.scheduled_task_count}</td>
                    <td className="px-2 py-2 text-right tabular-nums">{fmtDuration(r.scheduled_time_sec)}</td>
                    <td className="px-2 py-2 text-right tabular-nums">{fmtNumber(r.scheduled_priority_sum)}</td>
                    <td className="px-2 py-2 text-right tabular-nums">{fmtPercent(r.utilization)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </>
  );
}

function SchedulePreview({
  detail,
  slug,
  runId,
}: {
  detail: CellDetail;
  slug: string;
  runId: string;
}) {
  // We don't fetch the schedule body here (could be MB-scale); we just
  // expose links so the user can open / download artifacts.
  const baseUrl = `/api/v1/experiments/${encodeURIComponent(slug)}/runs/${encodeURIComponent(runId)}/cells/${encodeURIComponent(detail.cell_id)}`;
  return (
    <Card>
      <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
        Artifacts
      </h3>
      <div className="flex flex-wrap gap-2">
        {detail.schedule_path && (
          <a href={`${baseUrl}/schedule`} target="_blank" rel="noreferrer">
            <Button variant="secondary">View schedule.json</Button>
          </a>
        )}
        {detail.trace_path && (
          <a href={`${baseUrl}/trace`} target="_blank" rel="noreferrer">
            <Button variant="secondary">View trace.jsonl</Button>
          </a>
        )}
      </div>
      {!(detail.schedule_path || detail.trace_path) && (
        <p className="text-sm text-slate-500">No artifacts emitted for this cell.</p>
      )}
    </Card>
  );
}

function KvTable({ rows }: { rows: ReadonlyArray<readonly [string, string]> }) {
  return (
    <div className="grid grid-cols-2 gap-y-1.5 text-sm">
      {rows.map(([k, v]) => (
        <div key={k} className="contents">
          <div className="text-slate-400">{k}</div>
          <div className="text-right font-medium tabular-nums text-slate-200">{v}</div>
        </div>
      ))}
    </div>
  );
}
