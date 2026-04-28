/**
 * Sensitivity panel — visualise how a chosen outcome metric varies with the
 * EST configuration knobs.
 *
 * Renders:
 *   - A 3D scatter when ≥3 numeric dimensions are available, mapping
 *     user-selected x/y/z dims to axes and the metric to colour.
 *   - A 2D scatter (metric vs selected x dim).
 *   - Parallel coordinates over all numeric dims + metric.
 *   - Marginal box-plots per categorical dimension.
 *
 * Supports categorical faceting (small-multiples), lasso→focus cross-filter,
 * and per-panel CSV export.
 *
 * Generic over arbitrary `algorithm_config` keys via {@link extractDimensions}.
 */
import { useMemo } from 'react';
import {
  ChartPanel,
  EmptyState,
  PlotlyChart,
  RangeFilterGroup,
  TableSkeleton,
  ToolbarRow,
} from '@/components';
import { DownloadCsvButton, HelpPopover } from '@/components/charts';
import { usePlotlyChartChrome, usePlotlyTheme } from '@/hooks';
import { stringCodec, useUrlState } from '../../../../lib/useUrlState';
import type { Data, Layout } from 'plotly.js';
import type { InsightsData } from '@/api/types';
import type { RunRow } from '../useRunMatrix';
import { extractDimensions, readDimension, type Dimension } from '../dimensions';
import { useRunRangeFilters } from '../useRunRangeFilters';
import { useRunFocus, focusIdsFromSelection } from '../useRunFocus';
import { FocusBadge } from '../FocusBadge';
import {
  EST_FILTER_HELP,
  SENSITIVITY_2D_HELP,
  SENSITIVITY_3D_HELP,
  SENSITIVITY_PARCOORDS_HELP,
} from '../chartHelp';

type Metric = 'rate' | 'count' | 'capture';

const METRIC_LABELS: Record<Metric, string> = {
  rate: 'Scheduling rate (%)',
  count: 'Scheduled count',
  capture: 'Priority capture (%)',
};

function metricValue(ins: InsightsData, m: Metric): number {
  if (m === 'rate') return ins.metrics.scheduling_rate * 100;
  if (m === 'count') return ins.metrics.scheduled_count;
  return ins.metrics.priority_capture_ratio * 100;
}

interface Point {
  scheduleId: number;
  name: string;
  values: Record<string, number | string | null>;
  metric: number;
}

// ---------------------------------------------------------------------------
// Helpers for building individual chart specs
// ---------------------------------------------------------------------------

function buildScatter3d(
  pts: Point[],
  xKey: string,
  yKey: string,
  zKey: string,
  metricLabel: string,
  theme: ReturnType<typeof usePlotlyTheme>,
): { data: Data[]; layout: Partial<Layout> } {
  const data: Data[] = [
    {
      type: 'scatter3d',
      mode: 'text+markers',
      x: pts.map((p) => p.values[xKey] as number | null),
      y: pts.map((p) => p.values[yKey] as number | null),
      z: pts.map((p) => p.values[zKey] as number | null),
      text: pts.map((p) => p.name),
      textposition: 'top center',
      customdata: pts.map((p) => p.scheduleId),
      marker: {
        size: 8,
        color: pts.map((p) => p.metric),
        colorscale: 'Viridis',
        colorbar: { title: { text: metricLabel } },
        showscale: true,
      },
    },
  ];
  const layout: Partial<Layout> = {
    ...theme.layout,
    scene: {
      xaxis: { title: { text: xKey } },
      yaxis: { title: { text: yKey } },
      zaxis: { title: { text: zKey } },
    },
    margin: { l: 0, r: 0, t: 10, b: 0 },
  };
  return { data, layout };
}

function buildScatter2d(
  pts: Point[],
  xKey: string,
  metricLabel: string,
  theme: ReturnType<typeof usePlotlyTheme>,
): { data: Data[]; layout: Partial<Layout> } {
  const data: Data[] = [
    {
      type: 'scatter',
      mode: 'text+markers',
      x: pts.map((p) => p.values[xKey] as number | null),
      y: pts.map((p) => p.metric),
      text: pts.map((p) => p.name),
      textposition: 'top center',
      customdata: pts.map((p) => p.scheduleId),
      marker: { size: 10, color: pts.map((p) => p.metric), colorscale: 'Viridis' },
    },
  ];
  const layout: Partial<Layout> = {
    ...theme.layout,
    xaxis: { ...theme.layout.xaxis, title: { text: xKey } },
    yaxis: { ...theme.layout.yaxis, title: { text: metricLabel } },
    margin: { l: 60, r: 20, t: 20, b: 50 },
  };
  return { data, layout };
}

function buildParallelCoords(
  pts: Point[],
  numericDims: Dimension[],
  metricLabel: string,
  theme: ReturnType<typeof usePlotlyTheme>,
): { data: Data[]; layout: Partial<Layout> } | null {
  if (numericDims.length < 2) return null;
  const dimsForPC = [
    ...numericDims.map((d) => ({
      label: d.key,
      values: pts.map((p) => (p.values[d.key] as number | null) ?? Number.NaN),
    })),
    {
      label: metricLabel,
      values: pts.map((p) => p.metric),
    },
  ];
  const data: Data[] = [
    {
      type: 'parcoords',
      line: {
        color: pts.map((p) => p.metric),
        colorscale: 'Viridis',
        showscale: true,
        colorbar: { title: { text: metricLabel } },
      },
      customdata: pts.map((p) => p.scheduleId),
      dimensions: dimsForPC,
    } as Data,
  ];
  const layout: Partial<Layout> = {
    ...theme,
    margin: { l: 80, r: 80, t: 40, b: 40 },
  };
  return { data, layout };
}

// ---------------------------------------------------------------------------
// Sub-component: renders the three charts for a given subset of points
// ---------------------------------------------------------------------------

interface ChartsBlockProps {
  pts: Point[];
  xKey: string;
  yKey: string | null;
  zKey: string | null;
  numericDims: Dimension[];
  metric: Metric;
  theme: ReturnType<typeof usePlotlyTheme>;
  facetValue?: string;
  onFocusSelect: (ev: unknown) => void;
  onFocusDeselect: () => void;
  onFocusToggle: (id: number) => void;
}

function ChartsBlock({
  pts,
  xKey,
  yKey,
  zKey,
  numericDims,
  metric,
  theme,
  facetValue,
  onFocusSelect,
  onFocusDeselect,
  onFocusToggle,
}: ChartsBlockProps) {
  const metricLabel = METRIC_LABELS[metric];
  const suffix = facetValue !== undefined ? ` — ${facetValue}` : '';

  const scatter3d =
    yKey && zKey ? buildScatter3d(pts, xKey, yKey, zKey, metricLabel, theme) : null;
  const scatter2d = buildScatter2d(pts, xKey, metricLabel, theme);
  const parcoords = buildParallelCoords(pts, numericDims, metricLabel, theme);

  const s3chrome = usePlotlyChartChrome({
    label: `Configuration cube${suffix}`,
    help: SENSITIVITY_3D_HELP,
  });
  const s2chrome = usePlotlyChartChrome({
    label: `${metricLabel} vs ${xKey}${suffix}`,
    help: SENSITIVITY_2D_HELP,
  });
  const pcchrome = usePlotlyChartChrome({
    label: `Parallel coordinates${suffix}`,
    help: SENSITIVITY_PARCOORDS_HELP,
  });

  return (
    <div className="space-y-5">
      <div className="grid gap-5 xl:grid-cols-2">
        {scatter3d && (
          <ChartPanel
            title={`Configuration cube${suffix}`}
            headerActions={s3chrome.headerActions}
          >
            <PlotlyChart
              data={scatter3d.data}
              layout={scatter3d.layout}
              config={s3chrome.config}
              onInitialized={s3chrome.onInitialized}
              height="520px"
              ariaLabel="3D scatter of EST configuration vs metric"
              onClick={(ev) => {
                const id = (ev as { points?: Array<{ customdata?: unknown }> })?.points?.[0]
                  ?.customdata;
                if (typeof id === 'number') onFocusToggle(id);
              }}
            />
            {s3chrome.fullscreenOverlay}
          </ChartPanel>
        )}

        <ChartPanel
          title={`${metricLabel} vs ${xKey}${suffix}`}
          headerActions={s2chrome.headerActions}
        >
          <PlotlyChart
            data={scatter2d.data}
            layout={scatter2d.layout}
            config={s2chrome.config}
            onInitialized={s2chrome.onInitialized}
            height="320px"
            ariaLabel="Metric vs selected numeric dimension"
            onSelected={onFocusSelect}
            onDeselect={onFocusDeselect}
          />
          {s2chrome.fullscreenOverlay}
        </ChartPanel>
      </div>

      {parcoords && (
        <ChartPanel
          title={`Parallel coordinates${suffix}`}
          headerActions={pcchrome.headerActions}
        >
          <PlotlyChart
            data={parcoords.data}
            layout={parcoords.layout}
            config={pcchrome.config}
            onInitialized={pcchrome.onInitialized}
            height="380px"
            ariaLabel="Parallel coordinates linking configuration knobs to outcome metric"
            onSelected={onFocusSelect}
            onDeselect={onFocusDeselect}
          />
          {pcchrome.fullscreenOverlay}
        </ChartPanel>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Select helper
// ---------------------------------------------------------------------------

function DimSelect({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: string[];
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium uppercase tracking-wider text-slate-400">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-md border border-slate-600 bg-slate-800 px-2 py-1 text-sm text-slate-300 focus:outline-none focus:ring-2 focus:ring-primary-500"
      >
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export default function SensitivityPanel({ runs }: { runs: RunRow[] }) {
  const plotlyTheme = usePlotlyTheme();
  const [metricRaw, setMetricRaw] = useUrlState<string>('est_sens_metric', 'rate', {
    codec: stringCodec,
  });
  const metric: Metric = (['rate', 'count', 'capture'] as const).includes(metricRaw as Metric)
    ? (metricRaw as Metric)
    : 'rate';
  const setMetric = (m: Metric) => setMetricRaw(m);

  const filters = useRunRangeFilters(runs);
  const focus = useRunFocus();

  // Compose filters + focus: focus is applied on top of range filters
  const effectiveRuns = useMemo(
    () => focus.apply(filters.filtered),
    [focus, filters.filtered],
  );

  const dims = useMemo(
    () => extractDimensions(effectiveRuns.map((r) => r.algorithmConfig)),
    [effectiveRuns],
  );

  const points = useMemo<Point[]>(() => {
    const allDims: Dimension[] = [...dims.numeric, ...dims.categorical];
    return effectiveRuns
      .filter((r) => r.insights)
      .map((r) => {
        const values: Record<string, number | string | null> = {};
        for (const d of allDims) values[d.key] = readDimension(r.algorithmConfig, d);
        return {
          scheduleId: r.schedule.schedule_id,
          name: r.schedule.schedule_name,
          values,
          metric: metricValue(r.insights!, metric),
        };
      });
  }, [effectiveRuns, dims, metric]);

  const numericKeys = dims.numeric.map((d) => d.key);
  const categoricalKeys = dims.categorical.map((d) => d.key);

  // Dim selectors — default to first/second/third numeric dims
  const [xKey, setXKey] = useUrlState<string>('est_sens_x', '', { codec: stringCodec });
  const [yKey, setYKey] = useUrlState<string>('est_sens_y', '', { codec: stringCodec });
  const [zKey, setZKey] = useUrlState<string>('est_sens_z', '', { codec: stringCodec });
  const [facetKey, setFacetKey] = useUrlState<string>('est_sens_facet', '', {
    codec: stringCodec,
  });

  // Resolve effective keys (fall back to positional defaults when state is stale/empty)
  const resolvedX = numericKeys.includes(xKey) ? xKey : (numericKeys[0] ?? '');
  const resolvedY = numericKeys.includes(yKey) ? yKey : (numericKeys[1] ?? '');
  const resolvedZ =
    dims.numeric.length >= 3
      ? numericKeys.includes(zKey)
        ? zKey
        : (numericKeys[2] ?? '')
      : '';
  const resolvedFacet = categoricalKeys.includes(facetKey) ? facetKey : '';

  // Facet values: distinct values of the chosen categorical dim
  const facetValues = useMemo<string[]>(() => {
    if (!resolvedFacet) return [];
    const seen = new Set<string>();
    for (const p of points) {
      const v = p.values[resolvedFacet];
      if (v !== null && v !== undefined) seen.add(String(v));
    }
    return [...seen].sort();
  }, [resolvedFacet, points]);

  // Focus handlers
  const handleFocusSelect = (ev: unknown) => {
    focus.setFocused(focusIdsFromSelection(ev));
  };
  const handleFocusDeselect = () => focus.clear();
  const handleFocusToggle = (id: number) => focus.toggle(id);

  // Box-plot data for categorical impact section
  const boxPlotChrome = usePlotlyChartChrome({
    label: 'Categorical impact',
    help: undefined,
  });

  const boxPlots = useMemo<{ data: Data[]; layout: Partial<Layout> } | null>(() => {
    if (dims.categorical.length === 0) return null;
    const data: Data[] = dims.categorical.flatMap((catDim) => {
      const groups = new Map<string, Point[]>();
      for (const p of points) {
        const v = p.values[catDim.key];
        const key = v !== null && v !== undefined ? String(v) : '(none)';
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key)!.push(p);
      }
      return [...groups.entries()].map(([groupVal, groupPts]) => ({
        type: 'box' as const,
        name: `${catDim.key}=${groupVal}`,
        y: groupPts.map((p) => p.metric),
        customdata: groupPts.map((p) => p.scheduleId),
        boxpoints: 'all' as const,
        jitter: 0.3,
        pointpos: -1.8,
      }));
    });
    const layout: Partial<Layout> = {
      ...plotlyTheme.layout,
      yaxis: { ...plotlyTheme.layout.yaxis, title: { text: METRIC_LABELS[metric] } },
      margin: { l: 60, r: 20, t: 20, b: 80 },
    };
    return { data, layout };
  }, [dims.categorical, points, metric, plotlyTheme]);

  // CSV columns
  const csvColumns = useMemo(() => {
    const cols = [
      { header: 'schedule', accessor: (p: Point) => p.name },
      { header: METRIC_LABELS[metric], accessor: (p: Point) => p.metric },
      ...dims.numeric.map((d) => ({
        header: d.key,
        accessor: (p: Point) => p.values[d.key] as number | null,
      })),
      ...dims.categorical.map((d) => ({
        header: d.key,
        accessor: (p: Point) => p.values[d.key] as string | null,
      })),
    ];
    return cols;
  }, [dims, metric]);

  if (runs.length === 0) {
    return <TableSkeleton rows={6} columns={5} />;
  }

  if (points.length === 0 && !focus.active) {
    return <TableSkeleton rows={6} columns={5} />;
  }

  if (points.length === 0) {
    return (
      <EmptyState
        title="No runs match the current filters"
        hint="Adjust the filter sliders or clear the focus selection."
      />
    );
  }

  if (dims.numeric.length === 0) {
    return (
      <EmptyState
        title="No numeric configuration dimensions vary across the selected runs"
        hint="Pick a wider set of schedules or a different algorithm to see sensitivity."
      />
    );
  }

  const gridCols =
    facetValues.length >= 3 ? 'grid gap-5 xl:grid-cols-3' : 'grid gap-5 xl:grid-cols-2';

  return (
    <div className="space-y-5">
      {/* Focus badge */}
      <FocusBadge tone="primary" />

      {/* CSV export */}
      <div className="flex justify-end">
        <DownloadCsvButton label="Sensitivity points" rows={points} columns={csvColumns}>
          Export points
        </DownloadCsvButton>
      </div>

      <ToolbarRow>
        {/* Metric selector */}
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium uppercase tracking-wider text-slate-400">
            Metric
          </span>
          <div className="flex gap-3">
            {(Object.keys(METRIC_LABELS) as Metric[]).map((m) => (
              <label
                key={m}
                className="flex cursor-pointer items-center gap-1.5 text-sm text-slate-300"
              >
                <input
                  type="radio"
                  name="sens-metric"
                  value={m}
                  checked={metric === m}
                  onChange={() => setMetric(m)}
                  className="accent-primary-500"
                />
                {METRIC_LABELS[m]}
              </label>
            ))}
          </div>
        </div>

        {/* X dim selector */}
        {numericKeys.length > 1 && (
          <DimSelect
            label="X axis"
            value={resolvedX}
            options={numericKeys}
            onChange={(v) => setXKey(v)}
          />
        )}

        {/* Y / Z dim selectors (only meaningful for 3D, but shown for ≥2 numeric dims) */}
        {numericKeys.length >= 3 && (
          <>
            <DimSelect
              label="Y axis (3D)"
              value={resolvedY}
              options={numericKeys}
              onChange={(v) => setYKey(v)}
            />
            <DimSelect
              label="Z axis (3D)"
              value={resolvedZ}
              options={numericKeys}
              onChange={(v) => setZKey(v)}
            />
          </>
        )}

        {/* Facet selector */}
        {categoricalKeys.length > 0 && (
          <div className="flex flex-col gap-1">
            <span className="text-xs font-medium uppercase tracking-wider text-slate-400">
              Facet by
            </span>
            <select
              value={resolvedFacet}
              onChange={(e) => setFacetKey(e.target.value)}
              className="rounded-md border border-slate-600 bg-slate-800 px-2 py-1 text-sm text-slate-300 focus:outline-none focus:ring-2 focus:ring-primary-500"
            >
              <option value="">(none)</option>
              {categoricalKeys.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </div>
        )}
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

      {/* Charts — faceted or single */}
      {resolvedFacet && facetValues.length > 0 ? (
        <div className={gridCols}>
          {facetValues.map((fv) => {
            const facetPts = points.filter((p) => String(p.values[resolvedFacet]) === fv);
            return (
              <div key={fv} className="space-y-5 rounded-lg border border-slate-700 p-4">
                <p className="text-xs font-semibold uppercase tracking-wider text-slate-400">
                  {resolvedFacet} = {fv}
                </p>
                <ChartsBlock
                  pts={facetPts}
                  xKey={resolvedX}
                  yKey={resolvedY || null}
                  zKey={resolvedZ || null}
                  numericDims={dims.numeric}
                  metric={metric}
                  theme={plotlyTheme}
                  facetValue={fv}
                  onFocusSelect={handleFocusSelect}
                  onFocusDeselect={handleFocusDeselect}
                  onFocusToggle={handleFocusToggle}
                />
              </div>
            );
          })}
        </div>
      ) : (
        <ChartsBlock
          pts={points}
          xKey={resolvedX}
          yKey={resolvedY || null}
          zKey={resolvedZ || null}
          numericDims={dims.numeric}
          metric={metric}
          theme={plotlyTheme}
          onFocusSelect={handleFocusSelect}
          onFocusDeselect={handleFocusDeselect}
          onFocusToggle={handleFocusToggle}
        />
      )}

      {/* Marginal box-plots per categorical dim */}
      {boxPlots && (
        <ChartPanel title="Categorical impact" headerActions={boxPlotChrome.headerActions}>
          <PlotlyChart
            data={boxPlots.data}
            layout={boxPlots.layout}
            config={boxPlotChrome.config}
            onInitialized={boxPlotChrome.onInitialized}
            height="360px"
            ariaLabel="Box plots of metric distribution per categorical dimension value"
          />
          {boxPlotChrome.fullscreenOverlay}
        </ChartPanel>
      )}
    </div>
  );
}
