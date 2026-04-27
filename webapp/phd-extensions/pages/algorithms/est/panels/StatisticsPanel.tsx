/**
 * Statistics panel — tabular summary across the run set.
 *
 * For each numeric outcome metric and for each numeric configuration
 * dimension, computes mean, std, min, max, and the Pearson correlation
 * between the dimension and the metric.
 */
import { useMemo } from 'react';
import { ChartPanel, DataTable, RangeFilterGroup, type TableColumn } from '@/components';
import { HelpPopover } from '@/components/charts';
import {
  METRIC_CUMULATIVE_PRIORITY,
  METRIC_MEAN_PRIORITY,
  METRIC_PRIORITY_CAPTURE,
  METRIC_SCHEDULING_RATE,
  extractDimensions,
  groupEquivalentSchedules,
  readDimension,
  type MetricSpec,
} from '@/features/schedules/analytics';
import type { ScheduleAnalysisData } from '@/features/schedules/hooks/useScheduleAnalysisData';
import type { RunRow } from '../useRunMatrix';
import { useRunRangeFilters } from '../useRunRangeFilters';
import { EST_FILTER_HELP, STATISTICS_HELP } from '../chartHelp';

interface Row {
  metric: string;
  mean: string;
  std: string;
  min: string;
  max: string;
  bestRun: string;
  correlations: string;
}

const fmt = (v: number, digits = 3) =>
  Number.isFinite(v) ? v.toFixed(digits) : '—';

function stats(values: number[]) {
  const n = values.length;
  if (n === 0) return { mean: NaN, std: NaN, min: NaN, max: NaN };
  const mean = values.reduce((a, b) => a + b, 0) / n;
  const variance = values.reduce((a, b) => a + (b - mean) ** 2, 0) / Math.max(1, n - 1);
  return { mean, std: Math.sqrt(variance), min: Math.min(...values), max: Math.max(...values) };
}

function pearson(xs: number[], ys: number[]): number {
  const n = Math.min(xs.length, ys.length);
  if (n < 2) return NaN;
  const mx = xs.reduce((a, b) => a + b, 0) / n;
  const my = ys.reduce((a, b) => a + b, 0) / n;
  let num = 0;
  let dx = 0;
  let dy = 0;
  for (let i = 0; i < n; i++) {
    const a = xs[i] - mx;
    const b = ys[i] - my;
    num += a * b;
    dx += a * a;
    dy += b * b;
  }
  const denom = Math.sqrt(dx * dy);
  return denom === 0 ? NaN : num / denom;
}

/**
 * Headline metrics surfaced in the statistics table. Sourced from the
 * shared registry so labels and formatting stay in lock-step with the
 * comparison/sweep panels. `scheduled_count` is intentionally omitted —
 * within an environment it carries the same information as the
 * scheduling rate.
 */
const METRICS: MetricSpec[] = [
  METRIC_SCHEDULING_RATE,
  METRIC_PRIORITY_CAPTURE,
  METRIC_CUMULATIVE_PRIORITY,
  METRIC_MEAN_PRIORITY,
];

function adapt(run: RunRow): ScheduleAnalysisData {
  return {
    id: run.schedule.schedule_id,
    name: run.schedule.schedule_name,
    insights: run.insights,
    fragmentation: undefined,
    isLoading: false,
    error: null,
    algorithm: run.schedule.schedule_metadata?.algorithm,
    algorithmConfig: run.algorithmConfig,
  };
}

export default function StatisticsPanel({ runs }: { runs: RunRow[] }) {
  const filters = useRunRangeFilters(runs);
  const filteredRuns = filters.filtered;
  const adapted = useMemo(() => filteredRuns.map(adapt), [filteredRuns]);
  const dims = useMemo(
    () => extractDimensions(adapted.map((s) => s.algorithmConfig)),
    [adapted],
  );

  const equivalence = useMemo(
    () => groupEquivalentSchedules(adapted, (s) => s.insights),
    [adapted],
  );
  const poolSize = useMemo(() => {
    for (const s of adapted) {
      const total = s.insights?.metrics.total_observations;
      if (typeof total === 'number' && Number.isFinite(total)) return total;
    }
    return null;
  }, [adapted]);

  const rows = useMemo<Row[]>(() => {
    return METRICS.map((m) => {
      const pairs = adapted
        .map((s) => ({ run: s, value: m.getValue(s) }))
        .filter((p): p is { run: ScheduleAnalysisData; value: number } =>
          p.value !== null && Number.isFinite(p.value),
        );
      const values = pairs.map((p) => p.value);
      const s = stats(values);
      const best = pairs.reduce<{ name: string; value: number } | undefined>(
        (acc, cur) => {
          if (!acc) return { name: cur.run.name, value: cur.value };
          if (m.direction === 'max' && cur.value > acc.value)
            return { name: cur.run.name, value: cur.value };
          if (m.direction === 'min' && cur.value < acc.value)
            return { name: cur.run.name, value: cur.value };
          return acc;
        },
        undefined,
      );

      const correlations = dims.numeric
        .map((d) => {
          const xs: number[] = [];
          const ys: number[] = [];
          for (const p of pairs) {
            const v = readDimension(p.run.algorithmConfig, d);
            if (typeof v === 'number') {
              xs.push(v);
              ys.push(p.value);
            }
          }
          const r = pearson(xs, ys);
          return Number.isFinite(r) ? `${d.key}: r=${r.toFixed(2)}` : null;
        })
        .filter((s): s is string => s !== null)
        .join('  ·  ');

      return {
        metric: m.label,
        mean: fmt(s.mean),
        std: fmt(s.std),
        min: fmt(s.min),
        max: fmt(s.max),
        bestRun: best ? `${m.format(best.value)} (${best.name})` : '—',
        correlations: correlations || '—',
      };
    });
  }, [adapted, dims]);

  const columns: TableColumn<Row>[] = useMemo(
    () => [
      { header: 'Metric', accessor: 'metric' },
      { header: 'Mean', accessor: 'mean', align: 'right' },
      { header: 'Std', accessor: 'std', align: 'right' },
      { header: 'Min', accessor: 'min', align: 'right' },
      { header: 'Max', accessor: 'max', align: 'right' },
      { header: 'Best', accessor: 'bestRun' },
      { header: 'Correlation with config dims', accessor: 'correlations' },
    ],
    [],
  );

  return (
    <div className="space-y-5">
      {(poolSize != null || equivalence.groups.length > 0) && (
        <div className="rounded border border-slate-700 bg-slate-900/40 px-3 py-2 text-xs text-slate-300">
          {poolSize != null && (
            <>
              Task pool:{' '}
              <span className="font-semibold text-slate-100">
                {poolSize.toLocaleString()}
              </span>
            </>
          )}
          {equivalence.groups.length > 0 && (
            <span className="ml-3 text-emerald-300">
              · {equivalence.groups.length} equivalent group
              {equivalence.groups.length === 1 ? '' : 's'}
            </span>
          )}
        </div>
      )}
      {filters.specs.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <span className="text-xs font-semibold uppercase tracking-wider text-slate-400">
              Configuration filters
            </span>
            <HelpPopover content={EST_FILTER_HELP} ariaLabel="Help: configuration filters" />
          </div>
          <RangeFilterGroup
            specs={filters.specs}
            values={filters.values}
            onChange={filters.setValues}
          />
        </div>
      )}
      <ChartPanel
        title="Statistics report"
        headerActions={
          <HelpPopover content={STATISTICS_HELP} ariaLabel="Help: statistics report" />
        }
      >
        <DataTable
          data={rows}
          columns={columns}
          keyAccessor={(r) => r.metric}
          caption="Per-metric summary statistics and config correlations"
          captionHidden
        />
      </ChartPanel>
    </div>
  );
}
