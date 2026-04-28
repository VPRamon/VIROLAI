/**
 * Overview panel — KPI cards + run inventory table.
 *
 * Surfaces the headline metrics across the selected runs and a per-run table
 * showing the schedule, its algorithm-config snapshot, and outcome metrics.
 * Replaces the redundant "Most scheduled" KPI with cumulative priority and
 * tags equivalent runs (identical scheduled-task set) with a group badge.
 */
import { useMemo } from 'react';
import {
  ChartPanel,
  DataTable,
  EmptyState,
  MetricCard,
  MetricsGrid,
  RangeFilterGroup,
  TableSkeleton,
  type TableColumn,
} from '@/components';
import { DownloadCsvButton, HelpPopover } from '@/components/charts';
import {
  METRIC_CUMULATIVE_PRIORITY,
  METRIC_PRIORITY_CAPTURE,
  METRIC_SCHEDULING_RATE,
  groupEquivalentSchedules,
} from '@/features/schedules/analytics';
import type { ScheduleAnalysisData } from '@/features/schedules/hooks/useScheduleAnalysisData';
import type { RunRow } from '../useRunMatrix';
import { useRunFocus } from '../useRunFocus';
import { FocusBadge } from '../FocusBadge';
import { useRunRangeFilters } from '../useRunRangeFilters';
import { EST_FILTER_HELP, OVERVIEW_HELP } from '../chartHelp';

interface InventoryRow {
  id: number;
  name: string;
  algorithm: string;
  configSummary: string;
  rate: string;
  capture: string;
  cumulative: string;
  meanPriority: string;
  /** Empty when this row is unique; otherwise a short group label like "≡ A". */
  equivalence: string;
  /** Names of the other runs equivalent to this one, for the row tooltip. */
  equivalenceTooltip: string;
}

const fmtPct = (v: number | undefined | null) =>
  typeof v === 'number' && Number.isFinite(v) ? `${(v * 100).toFixed(1)}%` : '—';
const fmtNum = (v: number | undefined | null, digits = 2) =>
  typeof v === 'number' && Number.isFinite(v) ? v.toFixed(digits) : '—';

function summariseConfig(cfg: Record<string, unknown> | undefined): string {
  if (!cfg) return '—';
  return Object.entries(cfg)
    .map(([k, v]) => `${k}=${v}`)
    .join(', ');
}

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

export default function OverviewPanel({ runs }: { runs: RunRow[] }) {
  const filters = useRunRangeFilters(runs);
  const focus = useRunFocus();
  const filteredRuns = useMemo(() => focus.apply(filters.filtered), [focus, filters.filtered]);

  const adapted = useMemo(() => filteredRuns.map(adapt), [filteredRuns]);
  const equivalence = useMemo(
    () => groupEquivalentSchedules(adapted, (s) => s.insights),
    [adapted],
  );

  /** Within an environment every run shares the same task pool. */
  const poolSize = useMemo(() => {
    for (const s of adapted) {
      const total = s.insights?.metrics.total_observations;
      if (typeof total === 'number' && Number.isFinite(total)) return total;
    }
    return null;
  }, [adapted]);

  const inventory = useMemo<InventoryRow[]>(
    () =>
      adapted.map((s, i) => {
        const r = filteredRuns[i];
        const fp = equivalence.fingerprintOf.get(s);
        const idx = fp != null ? equivalence.groupIndex.get(fp) : undefined;
        const group = idx != null ? equivalence.groups[idx] : null;
        const isInGroup = group != null && group.members.length > 1;
        const otherNames = isInGroup
          ? group!.members.filter((m) => m.id !== s.id).map((m) => m.name)
          : [];
        return {
          id: s.id,
          name: s.name,
          algorithm:
            (r.traceSummary?.algorithm as string | undefined) ??
            r.schedule.schedule_metadata?.algorithm ??
            '—',
          configSummary: summariseConfig(r.algorithmConfig),
          rate: METRIC_SCHEDULING_RATE.format(METRIC_SCHEDULING_RATE.getValue(s)),
          capture: METRIC_PRIORITY_CAPTURE.format(METRIC_PRIORITY_CAPTURE.getValue(s)),
          cumulative: METRIC_CUMULATIVE_PRIORITY.format(
            METRIC_CUMULATIVE_PRIORITY.getValue(s),
          ),
          meanPriority: r.insights ? fmtNum(r.insights.metrics.mean_priority_scheduled) : '…',
          equivalence: isInGroup
            ? `≡ ${String.fromCharCode(65 + (idx ?? 0))} (×${group!.members.length})`
            : '',
          equivalenceTooltip: otherNames.join(', '),
        };
      }),
    [adapted, equivalence, filteredRuns],
  );

  const best = useMemo(() => {
    let bestRate: { name: string; value: number } | undefined;
    let bestCapture: { name: string; value: number } | undefined;
    let bestCumulative: { name: string; value: number } | undefined;
    for (const s of adapted) {
      const r = METRIC_SCHEDULING_RATE.getValue(s);
      const c = METRIC_PRIORITY_CAPTURE.getValue(s);
      const cum = METRIC_CUMULATIVE_PRIORITY.getValue(s);
      if (r != null && (!bestRate || r > bestRate.value))
        bestRate = { name: s.name, value: r };
      if (c != null && (!bestCapture || c > bestCapture.value))
        bestCapture = { name: s.name, value: c };
      if (cum != null && (!bestCumulative || cum > bestCumulative.value))
        bestCumulative = { name: s.name, value: cum };
    }
    return { bestRate, bestCapture, bestCumulative };
  }, [adapted]);

  const columns: TableColumn<InventoryRow>[] = useMemo(
    () => [
      {
        header: '',
        accessor: (r) => (
          <input
            type="checkbox"
            checked={focus.isFocused(r.id)}
            onChange={() => focus.toggle(r.id)}
            className="rounded border-slate-500 bg-slate-800 text-sky-500"
            aria-label={`Toggle focus on ${r.name}`}
          />
        ),
        align: 'center',
        width: 'w-8',
      },
      { header: 'Schedule', accessor: 'name' },
      { header: 'Algorithm', accessor: 'algorithm', align: 'center' },
      { header: 'Config', accessor: 'configSummary' },
      { header: 'Rate', accessor: 'rate', align: 'right' },
      { header: 'Priority capture', accessor: 'capture', align: 'right' },
      { header: 'Σ priority', accessor: 'cumulative', align: 'right' },
      { header: 'Mean priority', accessor: 'meanPriority', align: 'right' },
      {
        header: '≡',
        accessor: (r) =>
          r.equivalence ? (
            <span
              className="rounded bg-emerald-700/40 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-200"
              title={
                r.equivalenceTooltip
                  ? `Same scheduled task set as: ${r.equivalenceTooltip}`
                  : undefined
              }
            >
              {r.equivalence}
            </span>
          ) : (
            ''
          ),
        align: 'center',
      },
    ],
    [focus],
  );

  const collapsedSavings = equivalence.groups.reduce(
    (sum, g) => sum + (g.members.length - 1),
    0,
  );

  const csvColumns = useMemo(
    () => [
      { header: 'Schedule', accessor: (r: InventoryRow) => r.name },
      { header: 'Algorithm', accessor: (r: InventoryRow) => r.algorithm },
      { header: 'Config', accessor: (r: InventoryRow) => r.configSummary },
      { header: 'Rate', accessor: (r: InventoryRow) => r.rate },
      { header: 'Priority capture', accessor: (r: InventoryRow) => r.capture },
      { header: 'Cumulative priority', accessor: (r: InventoryRow) => r.cumulative },
      { header: 'Mean priority', accessor: (r: InventoryRow) => r.meanPriority },
      { header: 'Equivalence', accessor: (r: InventoryRow) => r.equivalence },
    ],
    [],
  );

  return (
    <div className="space-y-5">
      <FocusBadge />

      {poolSize != null && (
        <div className="rounded border border-slate-700 bg-slate-900/40 px-3 py-2 text-xs text-slate-300">
          Task pool: <span className="font-semibold text-slate-100">{poolSize.toLocaleString()}</span>
          {equivalence.groups.length > 0 && (
            <span className="ml-3 text-emerald-300">
              · {equivalence.groups.length} equivalent group
              {equivalence.groups.length === 1 ? '' : 's'}
              {' '}
              ({collapsedSavings} duplicate{collapsedSavings === 1 ? '' : 's'})
            </span>
          )}
        </div>
      )}

      <MetricsGrid columns={3}>
        <MetricCard
          label="Best scheduling rate"
          value={best.bestRate ? fmtPct(best.bestRate.value / 100) : '—'}
          trend={best.bestRate ? 'up' : undefined}
          trendValue={best.bestRate?.name}
        />
        <MetricCard
          label="Best priority capture"
          value={best.bestCapture ? fmtPct(best.bestCapture.value / 100) : '—'}
          trend={best.bestCapture ? 'up' : undefined}
          trendValue={best.bestCapture?.name}
        />
        <MetricCard
          label="Best cumulative priority"
          value={best.bestCumulative ? fmtNum(best.bestCumulative.value) : '—'}
          trend={best.bestCumulative ? 'up' : undefined}
          trendValue={best.bestCumulative?.name}
        />
      </MetricsGrid>

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
        title="Run inventory"
        headerActions={
          <div className="flex items-center gap-2">
            <DownloadCsvButton
              label="Run inventory"
              rows={inventory}
              columns={csvColumns}
            />
            <HelpPopover content={OVERVIEW_HELP} ariaLabel="Help: run inventory" />
          </div>
        }
      >
        {filteredRuns.length > 0 && filteredRuns.every((r) => !r.insights) ? (
          <TableSkeleton rows={5} columns={4} />
        ) : inventory.length === 0 ? (
          <EmptyState
            title="No data to display"
            hint="Adjust the filters or run more EST experiments."
          />
        ) : (
          <DataTable
            data={inventory}
            columns={columns}
            keyAccessor={(r) => r.id}
            caption="Selected EST runs"
            captionHidden
          />
        )}
      </ChartPanel>
    </div>
  );
}
