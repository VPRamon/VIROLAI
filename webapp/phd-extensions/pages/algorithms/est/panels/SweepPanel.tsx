/**
 * Sweep panel — compare multiple EST runs that differ in e/k/b parameters.
 *
 * Selection is provided by the AlgorithmAnalysis shell via `useAlgorithm()`;
 * this panel just renders charts/tables for the schedules already picked.
 *
 * Parameters are read from the structured `schedule_metadata.algorithm_config`
 * field when available; for legacy schedules without metadata the schedule
 * name is parsed as a fallback (`e{e}_k{k}_b{b}`).
 */
import { useMemo } from 'react';
import {
  ChartPanel,
  DataTable,
  EmptyState,
  MetricCard,
  MetricsGrid,
  PlotlyChart,
  RangeFilterGroup,
  TableSkeleton,
  ToolbarRow,
} from '@/components';
import type { TableColumn } from '@/components';
import { DownloadCsvButton, HelpPopover } from '@/components/charts';
import { stringCodec, useUrlState, type UrlStateCodec } from '../../../../lib/useUrlState';
import { usePlotlyChartChrome, usePlotlyTheme } from '@/hooks';
import type { ScheduleInfo } from '@/api/types';
import {
  METRIC_SCHEDULING_RATE,
  METRIC_PRIORITY_CAPTURE,
  METRIC_CUMULATIVE_PRIORITY,
  METRIC_MEAN_PRIORITY,
  type MetricSpec,
} from '@/features/schedules/analytics';
import type { ScheduleAnalysisData } from '@/features/schedules/hooks/useScheduleAnalysisData';
import { useAlgorithm } from '@/pages/AlgorithmAnalysis';
import { useRunMatrix, type RunRow } from '../useRunMatrix';
import { useRunFocus, focusIdsFromSelection } from '../useRunFocus';
import { FocusBadge } from '../FocusBadge';
import { useRunRangeFilters } from '../useRunRangeFilters';
import {
  EST_FILTER_HELP,
  SWEEP_3D_HELP,
  SWEEP_LINE_HELP,
  SWEEP_TABLE_HELP,
} from '../chartHelp';

interface EstParams {
  e: number | null;
  k: number | null;
  b: number | null;
}

function extractEstParams(s: ScheduleInfo, cfg?: Record<string, unknown>): EstParams {
  if (cfg && (cfg.endangered_threshold !== undefined || cfg.k_beams !== undefined)) {
    const num = (v: unknown): number | null =>
      typeof v === 'number' && Number.isFinite(v) ? v : null;
    return {
      e: num(cfg.endangered_threshold),
      k: num(cfg.k_beams),
      b: num(cfg.branching_factor),
    };
  }
  // Legacy fallback: parse the schedule name (`e{e}_k{k}_b{b}`) when no
  // structured `algorithm_config` is available.
  const m = s.schedule_name.match(/e(\d+)[_\-\s]*k(\d+)[_\-\s]*b(\d+)/i);
  if (!m) return { e: null, k: null, b: null };
  return { e: Number(m[1]), k: Number(m[2]), b: Number(m[3]) };
}

type Axis = 'e' | 'k' | 'b';

const AXIS_LABELS: Record<Axis, string> = {
  e: 'e — endangered threshold',
  k: 'k — beam width',
  b: 'b — branching factor',
};

/**
 * Headline metrics offered by the sweep panel.  Sourced from the shared
 * registry so the same wording / formatting / direction is used everywhere.
 * `scheduled_count` is intentionally omitted — within an environment it
 * carries the same information as `scheduling_rate`.
 */
const SWEEP_METRICS: MetricSpec[] = [
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

interface ChartRow extends EstParams {
  scheduleId: number;
  name: string;
  schedule: ScheduleAnalysisData;
}

function RadioGroup<T extends string>({
  label,
  name,
  options,
  value,
  onChange,
}: {
  label: string;
  name: string;
  options: { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium uppercase tracking-wider text-slate-400">{label}</span>
      <div className="flex flex-wrap gap-3">
        {options.map((opt) => (
          <label
            key={opt.value}
            className="flex cursor-pointer items-center gap-1.5 text-sm text-slate-300"
          >
            <input
              type="radio"
              name={name}
              value={opt.value}
              checked={value === opt.value}
              onChange={() => onChange(opt.value)}
              className="accent-primary-500"
            />
            {opt.label}
          </label>
        ))}
      </div>
    </div>
  );
}

export default function SweepPanel() {
  const { selectedSchedules } = useAlgorithm();
  const plotlyTheme = usePlotlyTheme();
  const [axis, setAxis] = useUrlState<Axis>('est_sweep_axis', 'e', {
    codec: stringCodec as UrlStateCodec<Axis>,
  });
  const [metricKey, setMetricKey] = useUrlState<string>(
    'est_sweep_metric',
    SWEEP_METRICS[0].key,
    { codec: stringCodec },
  );
  const metric = useMemo<MetricSpec>(
    () => SWEEP_METRICS.find((m) => m.key === metricKey) ?? SWEEP_METRICS[0],
    [metricKey],
  );

  const { runs } = useRunMatrix(selectedSchedules);
  const filters = useRunRangeFilters(runs);
  const focus = useRunFocus();
  const filteredRuns = useMemo(() => focus.apply(filters.filtered), [focus, filters.filtered]);

  const chartRows = useMemo<ChartRow[]>(
    () =>
      filteredRuns.map((r: RunRow) => {
        const params = extractEstParams(r.schedule, r.algorithmConfig);
        return {
          scheduleId: r.schedule.schedule_id,
          name: r.schedule.schedule_name,
          schedule: adapt(r),
          ...params,
        };
      }),
    [filteredRuns],
  );

  const loadedRows = useMemo(() => chartRows.filter((r) => r.schedule.insights), [chartRows]);

  function bestRun(m: MetricSpec): { name: string; value: string } | null {
    if (loadedRows.length === 0) return null;
    let best: ChartRow | null = null;
    let bestVal = m.direction === 'max' ? -Infinity : Infinity;
    for (const row of loadedRows) {
      const v = m.getValue(row.schedule);
      if (v == null || !Number.isFinite(v)) continue;
      if ((m.direction === 'max' && v > bestVal) || (m.direction === 'min' && v < bestVal)) {
        bestVal = v;
        best = row;
      }
    }
    if (!best) return null;
    return { name: best.name, value: m.format(bestVal) };
  }

  const bestRate = bestRun(METRIC_SCHEDULING_RATE);
  const bestCapture = bestRun(METRIC_PRIORITY_CAPTURE);
  const bestCumulative = bestRun(METRIC_CUMULATIVE_PRIORITY);

  function groupLabel(row: ChartRow, varyAxis: Axis): string {
    const others = (['e', 'k', 'b'] as Axis[]).filter((a) => a !== varyAxis);
    return others.map((a) => `${a}=${row[a] ?? '?'}`).join(', ');
  }

  const lineTraces = useMemo<Plotly.Data[]>(() => {
    const groups = new Map<string, ChartRow[]>();
    for (const row of chartRows) {
      const key = groupLabel(row, axis);
      const arr = groups.get(key) ?? [];
      arr.push(row);
      groups.set(key, arr);
    }
    return Array.from(groups.entries()).map(([label, rows]) => {
      const sorted = [...rows].sort((a, b) => (a[axis] ?? 0) - (b[axis] ?? 0));
      return {
        type: 'scatter' as const,
        mode: 'lines+markers' as const,
        name: label || 'all',
        x: sorted.map((r) => r[axis]),
        y: sorted.map((r) => metric.getValue(r.schedule)),
        customdata: sorted.map((r) => r.scheduleId),
        connectgaps: false,
      };
    });
  }, [chartRows, axis, metric]);

  const lineLayout = useMemo(
    (): Partial<Plotly.Layout> => ({
      ...plotlyTheme.layout,
      xaxis: { title: { text: AXIS_LABELS[axis] } },
      yaxis: { title: { text: metric.axisTitle } },
      legend: { orientation: 'h' as const, y: -0.25 },
      margin: { t: 10, b: 70, l: 55, r: 20 },
    }),
    [plotlyTheme.layout, axis, metric],
  );

  const scatter3dTrace = useMemo<Plotly.Data[]>(() => {
    if (loadedRows.length === 0) return [];
    return [
      {
        type: 'scatter3d' as const,
        mode: 'text+markers' as const,
        x: loadedRows.map((r) => r.e),
        y: loadedRows.map((r) => r.k),
        z: loadedRows.map((r) => r.b),
        text: loadedRows.map((r) => r.name),
        customdata: loadedRows.map((r) => r.scheduleId),
        textposition: 'top center' as const,
        textfont: { size: 9 },
        marker: {
          size: 9,
          color: loadedRows.map((r) => metric.getValue(r.schedule) ?? 0),
          colorscale: 'Viridis' as const,
          showscale: true,
          colorbar: { title: { text: metric.axisTitle }, thickness: 14, len: 0.7 },
        },
      },
    ];
  }, [loadedRows, metric]);

  const scatter3dLayout = useMemo(
    (): Partial<Plotly.Layout> => ({
      ...plotlyTheme.layout,
      scene: {
        xaxis: { title: { text: 'e' } },
        yaxis: { title: { text: 'k' } },
        zaxis: { title: { text: 'b' } },
      },
      margin: { t: 10, b: 10, l: 10, r: 10 },
    }),
    [plotlyTheme.layout],
  );

  const resultColumns = useMemo<TableColumn<ChartRow>[]>(
    () => [
      { header: 'Schedule', accessor: 'name' },
      { header: 'e', accessor: (r) => r.e ?? '—', align: 'center', width: 'w-10' },
      { header: 'k', accessor: (r) => r.k ?? '—', align: 'center', width: 'w-10' },
      { header: 'b', accessor: (r) => r.b ?? '—', align: 'center', width: 'w-10' },
      {
        header: 'Rate',
        accessor: (r) => METRIC_SCHEDULING_RATE.format(METRIC_SCHEDULING_RATE.getValue(r.schedule)),
        align: 'right',
      },
      {
        header: 'Priority capture',
        accessor: (r) =>
          METRIC_PRIORITY_CAPTURE.format(METRIC_PRIORITY_CAPTURE.getValue(r.schedule)),
        align: 'right',
      },
      {
        header: 'Σ priority',
        accessor: (r) =>
          METRIC_CUMULATIVE_PRIORITY.format(METRIC_CUMULATIVE_PRIORITY.getValue(r.schedule)),
        align: 'right',
      },
      {
        header: 'Mean priority',
        accessor: (r) =>
          METRIC_MEAN_PRIORITY.format(METRIC_MEAN_PRIORITY.getValue(r.schedule)),
        align: 'right',
      },
    ],
    [],
  );

  const lineChrome = usePlotlyChartChrome({
    label: `${metric.label} vs ${axis}`,
    help: SWEEP_LINE_HELP,
  });
  const scatter3dChrome = usePlotlyChartChrome({
    label: '3D parameter space',
    help: SWEEP_3D_HELP,
  });

  return (
    <div className="space-y-5">
      <FocusBadge />
      <MetricsGrid columns={3}>
        <MetricCard
          label="Best scheduling rate"
          value={bestRate?.value ?? '—'}
          trend={bestRate ? 'up' : undefined}
          trendValue={bestRate?.name}
        />
        <MetricCard
          label="Best priority capture"
          value={bestCapture?.value ?? '—'}
          trend={bestCapture ? 'up' : undefined}
          trendValue={bestCapture?.name}
        />
        <MetricCard
          label="Best cumulative priority"
          value={bestCumulative?.value ?? '—'}
          trend={bestCumulative ? 'up' : undefined}
          trendValue={bestCumulative?.name}
        />
      </MetricsGrid>

      <ToolbarRow>
        <RadioGroup<Axis>
          label="X axis"
          name="sweep-axis"
          value={axis}
          onChange={setAxis}
          options={[
            { value: 'e', label: 'e' },
            { value: 'k', label: 'k' },
            { value: 'b', label: 'b' },
          ]}
        />
        <div className="h-8 w-px bg-slate-600" aria-hidden />
        <RadioGroup<string>
          label="Metric"
          name="sweep-metric"
          value={metric.key}
          onChange={setMetricKey}
          options={SWEEP_METRICS.map((m) => ({ value: m.key, label: m.label }))}
        />
      </ToolbarRow>

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

      <div className="grid gap-5 xl:grid-cols-2">
        <ChartPanel
          title={`${metric.label} vs ${axis}`}
          headerActions={lineChrome.headerActions}
        >
          <PlotlyChart
            data={lineTraces}
            layout={lineLayout}
            config={lineChrome.config}
            onInitialized={lineChrome.onInitialized}
            onSelected={(ev) => focus.setFocused(focusIdsFromSelection(ev))}
            onDeselect={() => focus.clear()}
            height="340px"
            ariaLabel={`Line chart: ${metric.label} vs ${axis}`}
          />
          {lineChrome.fullscreenOverlay}
        </ChartPanel>

        <ChartPanel title="3D parameter space" headerActions={scatter3dChrome.headerActions}>
          {loadedRows.length === 0 ? (
            <TableSkeleton rows={5} columns={4} />
          ) : (
            <PlotlyChart
              data={scatter3dTrace}
              layout={scatter3dLayout}
              config={scatter3dChrome.config}
              onInitialized={scatter3dChrome.onInitialized}
              onClick={(ev) => {
                const id = ev.points?.[0]?.customdata as unknown;
                if (typeof id === 'number') focus.toggle(id);
              }}
              height="520px"
              ariaLabel="3D scatter: e / k / b axes, colour = selected metric"
            />
          )}
          {scatter3dChrome.fullscreenOverlay}
        </ChartPanel>
      </div>

      <ChartPanel
        title="Summary"
        headerActions={
          <div className="flex items-center gap-2">
            <DownloadCsvButton
              label="Sweep summary"
              rows={chartRows}
              columns={[
                { header: 'Schedule', accessor: (r: ChartRow) => r.name },
                { header: 'e', accessor: (r: ChartRow) => r.e },
                { header: 'k', accessor: (r: ChartRow) => r.k },
                { header: 'b', accessor: (r: ChartRow) => r.b },
                {
                  header: 'Rate',
                  accessor: (r: ChartRow) => METRIC_SCHEDULING_RATE.getValue(r.schedule),
                },
                {
                  header: 'Priority capture',
                  accessor: (r: ChartRow) => METRIC_PRIORITY_CAPTURE.getValue(r.schedule),
                },
                {
                  header: 'Cumulative priority',
                  accessor: (r: ChartRow) => METRIC_CUMULATIVE_PRIORITY.getValue(r.schedule),
                },
                {
                  header: 'Mean priority',
                  accessor: (r: ChartRow) => METRIC_MEAN_PRIORITY.getValue(r.schedule),
                },
              ]}
            />
            <HelpPopover content={SWEEP_TABLE_HELP} ariaLabel="Help: sweep summary" />
          </div>
        }
      >
        {chartRows.length === 0 ? (
          <EmptyState
            title="No data to display"
            hint="Adjust the filters or run more EST experiments."
          />
        ) : (
          <DataTable
            data={chartRows}
            columns={resultColumns}
            keyAccessor={(r) => r.scheduleId}
            caption="EST sweep results"
            captionHidden
          />
        )}
      </ChartPanel>
    </div>
  );
}
