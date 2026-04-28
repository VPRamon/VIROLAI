/**
 * Pareto panel — configurable multi-objective view of the run set.
 *
 * Replaces the previous fixed 3D (rate × priority × fragmentation) chart.
 * Users now pick any 2 or 3 metrics from the shared metric registry, and
 * may colour the markers by an arbitrary configuration knob to expose
 * which knob value drives the front.  Equivalent runs (identical
 * scheduled-task set) can be collapsed to a single representative.
 *
 * Enhanced with:
 * - Focus set (cross-filter) integration via useRunFocus
 * - Lasso/box-select → focus set; 3D click-toggle
 * - Equivalent-run ghost traces for visual grouping
 * - Weighted-sum scalarization with per-metric sliders
 * - CSV export of the current point set
 */
import { useEffect, useMemo, useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import { api } from '@/api';
import { queryKeys } from '@/hooks/useApi';
import {
  ChartPanel,
  EmptyState,
  PlotlyChart,
  RangeFilterGroup,
  TableSkeleton,
} from '@/components';
import { useAsyncMemo } from '@/hooks/useAsyncMemo';
import {
  computeParetoFront as computeParetoFrontSync,
  type ObjectiveDirection,
} from '@/workers/aggregations';
import { getAggregationsClient } from '@/workers/aggregationsClient';
import {
  useUrlState,
  stringCodec,
  booleanCodec,
  jsonCodec,
  type UrlStateCodec,
} from '../../../../lib/useUrlState';
import { DownloadCsvButton, HelpPopover } from '@/components/charts';
import { usePlotlyChartChrome, usePlotlyTheme } from '@/hooks';
import {
  ALL_METRICS,
  DEFAULT_COMPARISON_METRIC_KEYS,
  extractDimensions,
  groupEquivalentSchedules,
  readDimension,
  type Dimension,
  type MetricSpec,
} from '@/features/schedules/analytics';
import type { ScheduleAnalysisData } from '@/features/schedules/hooks/useScheduleAnalysisData';
import type { Data, Layout } from 'plotly.js';
import type { CsvColumn } from '@/lib/csvExport';
import type { RunRow } from '../useRunMatrix';
import { useRunRangeFilters } from '../useRunRangeFilters';
import { useRunFocus, focusIdsFromSelection } from '../useRunFocus';
import { FocusBadge } from '../FocusBadge';
import { EST_FILTER_HELP, PARETO_HELP } from '../chartHelp';

type Mode = '2d' | '3d';

interface ParetoPoint {
  runId: number;
  name: string;
  values: Array<number>;
  config: Record<string, unknown> | undefined;
  /** Number of equivalent runs collapsed onto this representative. */
  equivalentCount: number;
  dominated: boolean;
  /** Equivalence fingerprint — present when equivalence groups exist. */
  fingerprint?: string;
}

const NONE_DIM_KEY = '__none__';

/**
 * Adapt a {@link RunRow} into the {@link ScheduleAnalysisData} shape that
 * the metric registry consumes.  Frees the panel from coupling to either
 * data source individually.
 */
function adapt(
  run: RunRow,
  fragmentation: ScheduleAnalysisData['fragmentation'],
): ScheduleAnalysisData {
  return {
    id: run.schedule.schedule_id,
    name: run.schedule.schedule_name,
    insights: run.insights,
    fragmentation,
    isLoading: false,
    error: null,
    algorithm: run.schedule.schedule_metadata?.algorithm,
    algorithmConfig: run.algorithmConfig,
  };
}

/** Returns true when a beats b on every selected metric and strictly on one. */
function dominates(a: number[], b: number[], metrics: MetricSpec[]): boolean {
  let strictlyBetter = false;
  for (let i = 0; i < metrics.length; i += 1) {
    const dir = metrics[i].direction;
    const av = a[i];
    const bv = b[i];
    if (dir === 'max') {
      if (av < bv) return false;
      if (av > bv) strictlyBetter = true;
    } else {
      if (av > bv) return false;
      if (av < bv) strictlyBetter = true;
    }
  }
  return strictlyBetter;
}

export default function ParetoPanel({ runs }: { runs: RunRow[] }) {
  const plotlyTheme = usePlotlyTheme();
  const filters = useRunRangeFilters(runs);
  const focus = useRunFocus();

  // Compose range filters with the focus set: effective is the intersection.
  const effective = useMemo(
    () => focus.apply(filters.filtered),
    [focus, filters.filtered],
  );

  const fragQueries = useQueries({
    queries: effective.map((r) => ({
      queryKey: queryKeys.fragmentation(r.schedule.schedule_id),
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api.getFragmentation(r.schedule.schedule_id, { signal }),
      enabled: r.schedule.schedule_id > 0,
    })),
  });

  const adapted = useMemo<ScheduleAnalysisData[]>(
    () => effective.map((r, i) => adapt(r, fragQueries[i]?.data)),
    [effective, fragQueries],
  );

  /** Metrics with at least one finite value — others are useless as axes. */
  const availableMetrics = useMemo<MetricSpec[]>(
    () =>
      ALL_METRICS.filter((m) =>
        adapted.some((s) => {
          const v = m.getValue(s);
          return v != null && Number.isFinite(v);
        }),
      ),
    [adapted],
  );

  const [mode, setMode] = useUrlState<Mode>('est_pareto_mode', '2d', {
    codec: stringCodec as UrlStateCodec<Mode>,
  });
  const [xKey, setXKey] = useUrlState<string>(
    'est_pareto_x',
    DEFAULT_COMPARISON_METRIC_KEYS[0],
    { codec: stringCodec },
  );
  const [yKey, setYKey] = useUrlState<string>(
    'est_pareto_y',
    DEFAULT_COMPARISON_METRIC_KEYS[1],
    { codec: stringCodec },
  );
  const [zKey, setZKey] = useUrlState<string>(
    'est_pareto_z',
    DEFAULT_COMPARISON_METRIC_KEYS[2],
    { codec: stringCodec },
  );
  const [colorDimKey, setColorDimKey] = useUrlState<string>(
    'est_pareto_color',
    NONE_DIM_KEY,
    { codec: stringCodec },
  );
  const [collapseEquivalents, setCollapseEquivalents] = useState(true);
  const [scalarize, setScalarize] = useUrlState<boolean>(
    'est_pareto_scalarize',
    false,
    { codec: booleanCodec },
  );
  const [weights, setWeights] = useUrlState<Record<string, number>>(
    'est_pareto_weights',
    {},
    { codec: jsonCodec<Record<string, number>>() },
  );

  /** When the available metric set changes, snap selectors back to the first valid one. */
  useEffect(() => {
    if (availableMetrics.length === 0) return;
    const keys = availableMetrics.map((m) => m.key);
    if (!keys.includes(xKey)) setXKey(keys[0]);
    if (!keys.includes(yKey)) setYKey(keys[Math.min(1, keys.length - 1)]);
    if (!keys.includes(zKey)) setZKey(keys[Math.min(2, keys.length - 1)]);
  }, [availableMetrics, xKey, yKey, zKey]);

  const xMetric = useMemo(
    () => availableMetrics.find((m) => m.key === xKey) ?? availableMetrics[0],
    [availableMetrics, xKey],
  );
  const yMetric = useMemo(
    () => availableMetrics.find((m) => m.key === yKey) ?? availableMetrics[Math.min(1, availableMetrics.length - 1)],
    [availableMetrics, yKey],
  );
  const zMetric = useMemo(
    () => availableMetrics.find((m) => m.key === zKey) ?? availableMetrics[Math.min(2, availableMetrics.length - 1)],
    [availableMetrics, zKey],
  );

  const activeMetrics = useMemo<MetricSpec[]>(
    () =>
      (mode === '3d' && zMetric ? [xMetric, yMetric, zMetric] : [xMetric, yMetric]).filter(
        (m): m is MetricSpec => m != null,
      ),
    [mode, xMetric, yMetric, zMetric],
  );

  /** Reset weights to equal 1/N whenever scalarize is enabled or active metrics change. */
  useEffect(() => {
    if (!scalarize || activeMetrics.length === 0) return;
    const n = activeMetrics.length;
    setWeights(Object.fromEntries(activeMetrics.map((m) => [m.key, 1 / n])));
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scalarize, activeMetrics]);

  const dims = useMemo(
    () => extractDimensions(adapted.map((s) => s.algorithmConfig)),
    [adapted],
  );
  const allDims = useMemo<Dimension[]>(
    () => [...dims.numeric, ...dims.categorical],
    [dims],
  );
  const colorDim = useMemo<Dimension | null>(
    () => allDims.find((d) => d.key === colorDimKey) ?? null,
    [allDims, colorDimKey],
  );

  const equivalence = useMemo(
    () => groupEquivalentSchedules(adapted, (s) => s.insights),
    [adapted],
  );

  /** Build the point set for the chosen axes. */
  const points = useMemo<ParetoPoint[]>(() => {
    if (!xMetric || !yMetric) return [];
    const activeMs: MetricSpec[] =
      mode === '3d' && zMetric ? [xMetric, yMetric, zMetric] : [xMetric, yMetric];

    const seen = new Set<string>();
    const raw: ParetoPoint[] = [];

    adapted.forEach((s) => {
      const vals = activeMs.map((m) => m.getValue(s));
      if (vals.some((v) => v == null || !Number.isFinite(v))) return;

      let key = String(s.id);
      let count = 1;
      const fp = equivalence.fingerprintOf.get(s) ?? undefined;
      if (collapseEquivalents) {
        if (fp) {
          if (seen.has(fp)) return;
          seen.add(fp);
          const idx = equivalence.groupIndex.get(fp);
          const group = idx != null ? equivalence.groups[idx] : null;
          if (group) count = group.members.length;
          key = fp;
        }
      }

      raw.push({
        runId: s.id,
        name: s.name,
        values: vals as number[],
        config: s.algorithmConfig,
        equivalentCount: count,
        dominated: false,
        fingerprint: fp,
      });
      void key;
    });

    return raw.map((p, i) => ({
      ...p,
      dominated: raw.some((q, j) => j !== i && dominates(q.values, p.values, activeMs)),
    }));
  }, [adapted, collapseEquivalents, equivalence, mode, xMetric, yMetric, zMetric]);

  /** Weighted-sum scalar per point — only computed when scalarize is on. */
  const scalars = useMemo<Map<number, number>>(() => {
    if (!scalarize || activeMetrics.length === 0 || points.length === 0) return new Map();
    const mins = activeMetrics.map((_, i) => Math.min(...points.map((p) => p.values[i])));
    const maxs = activeMetrics.map((_, i) => Math.max(...points.map((p) => p.values[i])));
    const result = new Map<number, number>();
    for (const p of points) {
      let scalar = 0;
      activeMetrics.forEach((m, i) => {
        const w = weights[m.key] ?? 1 / activeMetrics.length;
        const range = maxs[i] - mins[i];
        let norm = 0;
        if (range !== 0) {
          norm =
            m.direction === 'max'
              ? (p.values[i] - mins[i]) / range
              : (maxs[i] - p.values[i]) / range;
        }
        scalar += w * norm;
      });
      result.set(p.runId, scalar);
    }
    return result;
  }, [scalarize, activeMetrics, points, weights]);

  const traces = useMemo<Data[]>(() => {
    if (!xMetric || !yMetric || points.length === 0) return [];
    const activeMs: MetricSpec[] =
      mode === '3d' && zMetric ? [xMetric, yMetric, zMetric] : [xMetric, yMetric];

    /** Format hover text with all selected metrics and the colour dim if any. */
    const hoverText = (p: ParetoPoint): string => {
      const lines: string[] = [
        `<b>${p.name}</b>${p.equivalentCount > 1 ? ` <span>(×${p.equivalentCount})</span>` : ''}`,
      ];
      activeMs.forEach((m, i) => lines.push(`${m.label}: ${m.format(p.values[i])}`));
      if (colorDim && !scalarize) {
        const v = readDimension(p.config, colorDim);
        lines.push(`${colorDim.key}: ${v ?? '—'}`);
      }
      if (scalarize) {
        const s = scalars.get(p.runId);
        if (s !== undefined) lines.push(`Score: ${s.toFixed(3)}`);
      }
      if (p.dominated) lines.push('<i>dominated</i>');
      return lines.join('<br>');
    };

    /** Marker text label (compact: name + ×count when relevant). */
    const labelOf = (p: ParetoPoint): string =>
      p.equivalentCount > 1 ? `${p.name} ×${p.equivalentCount}` : p.name;

    const buildTrace = (
      rows: ParetoPoint[],
      fallbackColour: string,
      label: string,
      isFront: boolean,
    ): Data => {
      const x = rows.map((p) => p.values[0]);
      const y = rows.map((p) => p.values[1]);
      const z = mode === '3d' ? rows.map((p) => p.values[2]) : undefined;

      let markerColour: string | number[] | undefined = fallbackColour;
      let colorscale: unknown = undefined;
      let colorbar: unknown = undefined;
      let textColour: string | string[] = isFront ? '#22c55e' : '#94a3b8';

      if (scalarize && scalars.size > 0) {
        // Override colour with weighted-sum scalar; suppress colour-by-dim.
        markerColour = rows.map((p) => scalars.get(p.runId) ?? 0);
        colorscale = 'Viridis';
        colorbar = { title: { text: 'Score' }, thickness: 12 };
        textColour = isFront ? '#facc15' : '#94a3b8';
      } else if (colorDim) {
        if (colorDim.kind === 'numeric') {
          const numericColours = rows.map((p) => {
            const v = readDimension(p.config, colorDim);
            return typeof v === 'number' ? v : NaN;
          });
          markerColour = numericColours;
          colorscale = 'Viridis';
          colorbar = { title: { text: colorDim.key }, thickness: 12 };
          textColour = isFront ? '#facc15' : '#94a3b8';
        } else {
          const palette = ['#22c55e', '#f97316', '#a855f7', '#0ea5e9', '#ef4444', '#eab308', '#14b8a6'];
          const distinct = colorDim.values as string[];
          const lookup = new Map(distinct.map((v, i) => [v, palette[i % palette.length]]));
          const cols = rows.map((p) => {
            const v = readDimension(p.config, colorDim);
            return (v != null && lookup.get(String(v))) || fallbackColour;
          });
          markerColour = cols as unknown as string;
        }
      }

      const baseMarker = {
        size: mode === '3d' ? 7 : 11,
        color: markerColour,
        opacity: isFront ? 1 : 0.55,
        line: isFront
          ? { width: 1.5, color: '#a3e635' }
          : { width: 0.5, color: '#475569' },
        ...(colorscale ? { colorscale } : {}),
        ...(colorbar ? { colorbar } : {}),
      };

      const common = {
        name: label,
        text: rows.map(labelOf),
        hovertext: rows.map(hoverText),
        hoverinfo: 'text' as const,
        textposition: 'top center' as const,
        textfont: { color: textColour, size: 10 },
        customdata: rows.map((r) => r.runId),
      };

      if (mode === '3d') {
        return {
          type: 'scatter3d',
          mode: isFront ? 'text+markers' : 'markers',
          x,
          y,
          z,
          marker: baseMarker,
          ...common,
        } as Data;
      }
      return {
        type: 'scatter',
        mode: isFront ? 'text+markers' : 'markers',
        x,
        y,
        marker: baseMarker,
        ...common,
      } as Data;
    };

    const front = points.filter((p) => !p.dominated);
    const dominated = points.filter((p) => p.dominated);
    const out: Data[] = [];
    if (dominated.length) out.push(buildTrace(dominated, '#64748b', 'Dominated', false));
    if (front.length) out.push(buildTrace(front, '#22c55e', 'Pareto front', true));

    // Ghost traces: faint dotted lines connecting equivalent-run siblings.
    // Only meaningful when equivalents are not collapsed into representatives.
    if (!collapseEquivalents && equivalence.groups.length > 0) {
      const grouped = new Map<string, ParetoPoint[]>();
      for (const p of points) {
        if (p.fingerprint) {
          const arr = grouped.get(p.fingerprint) ?? [];
          arr.push(p);
          grouped.set(p.fingerprint, arr);
        }
      }
      for (const members of grouped.values()) {
        if (members.length < 2) continue;
        if (mode === '3d') {
          out.push({
            type: 'scatter3d',
            mode: 'lines',
            x: members.map((p) => p.values[0]),
            y: members.map((p) => p.values[1]),
            z: members.map((p) => p.values[2]),
            line: { color: 'rgba(148,163,184,0.25)', width: 2 },
            hoverinfo: 'skip',
            showlegend: false,
            name: '',
          } as Data);
        } else {
          out.push({
            type: 'scatter',
            mode: 'lines',
            x: members.map((p) => p.values[0]),
            y: members.map((p) => p.values[1]),
            line: { color: 'rgba(148,163,184,0.25)', dash: 'dot', width: 1.5 },
            hoverinfo: 'skip',
            showlegend: false,
            name: '',
          } as Data);
        }
      }
    }

    return out;
  }, [points, mode, xMetric, yMetric, zMetric, colorDim, scalarize, scalars, collapseEquivalents, equivalence]);

  const layout = useMemo<Partial<Layout>>(() => {
    if (!xMetric || !yMetric) return plotlyTheme.layout;
    if (mode === '3d' && zMetric) {
      return {
        ...plotlyTheme.layout,
        scene: {
          xaxis: { title: { text: xMetric.axisTitle } },
          yaxis: { title: { text: yMetric.axisTitle } },
          zaxis: { title: { text: zMetric.axisTitle } },
        },
        margin: { l: 0, r: 0, t: 10, b: 0 },
        legend: { orientation: 'h', y: -0.05 },
      };
    }
    return {
      ...plotlyTheme.layout,
      xaxis: { ...plotlyTheme.layout?.xaxis, title: { text: xMetric.axisTitle } },
      yaxis: { ...plotlyTheme.layout?.yaxis, title: { text: yMetric.axisTitle } },
      margin: { l: 60, r: 30, t: 20, b: 60 },
      legend: { orientation: 'h', y: -0.15 },
    };
  }, [plotlyTheme, mode, xMetric, yMetric, zMetric]);

  const chrome = usePlotlyChartChrome({ label: 'Pareto front', help: PARETO_HELP });

  const equivalentGroupCount = equivalence.groups.length;
  const collapsedSavings = equivalence.groups.reduce(
    (sum, g) => sum + (g.members.length - 1),
    0,
  );

  /** CSV column descriptors for the current point set. */
  const csvColumns = useMemo<CsvColumn<ParetoPoint>[]>(() => {
    const cols: CsvColumn<ParetoPoint>[] = [
      { header: 'Schedule', accessor: (p) => p.name },
      { header: 'Equivalent Count', accessor: (p) => p.equivalentCount },
      { header: 'Dominated', accessor: (p) => String(p.dominated) },
      ...activeMetrics.map((m, i) => ({
        header: m.label,
        accessor: (p: ParetoPoint) => m.format(p.values[i]),
      })),
    ];
    if (scalarize && scalars.size > 0) {
      cols.push({ header: 'scalar', accessor: (p) => scalars.get(p.runId) ?? null });
    }
    return cols;
  }, [activeMetrics, scalarize, scalars]);

  /** In 3D mode, clicking a point toggles its run in the focus set. */
  const handleClick = useMemo(
    () =>
      mode !== '3d'
        ? undefined
        : (ev: unknown) => {
            const pts = (ev as { points?: Array<{ customdata?: unknown }> }).points;
            if (!pts?.length) return;
            const cd = pts[0]?.customdata;
            if (typeof cd === 'number') focus.toggle(cd);
          },
    [mode, focus],
  );

  // Top-3 scalar-best runs for the scalarize caption.
  const top3Scalar =
    scalarize && scalars.size > 0
      ? [...points]
          .map((p) => ({ name: p.name, scalar: scalars.get(p.runId) ?? 0 }))
          .sort((a, b) => b.scalar - a.scalar)
          .slice(0, 3)
      : null;

  const renderConfigFilters = () =>
    filters.specs.length > 0 ? (
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
    ) : null;

  const isLoading =
    effective.length > 0 && fragQueries.some((q) => q.isLoading);

  if (isLoading) {
    return (
      <div className="space-y-4">
        {renderConfigFilters()}
        <TableSkeleton rows={8} columns={4} />
      </div>
    );
  }

  if (availableMetrics.length < 2 || points.length === 0) {
    return (
      <div className="space-y-4">
        {renderConfigFilters()}
        <EmptyState
          title="No solutions to plot"
          hint="Run an EST experiment with multiple objectives, or relax the filters."
        />
      </div>
    );
  }

  const metricSelect = (
    label: string,
    value: string,
    onChange: (next: string) => void,
    excludeKeys: string[] = [],
  ) => (
    <label className="flex items-center gap-2 text-xs text-slate-300">
      <span className="font-semibold uppercase tracking-wider text-slate-400">{label}</span>
      <select
        value={value}
        onChange={(ev) => onChange(ev.target.value)}
        className="rounded border border-slate-600 bg-slate-800 px-2 py-1 text-xs text-slate-200"
      >
        {availableMetrics
          .filter((m) => !excludeKeys.includes(m.key) || m.key === value)
          .map((m) => (
            <option key={m.key} value={m.key}>
              {m.label}
            </option>
          ))}
      </select>
    </label>
  );

  return (
    <div className="space-y-5">
      {renderConfigFilters()}

      {/* Focus indicator badge */}
      <FocusBadge tone="emerald" />

      <div className="flex flex-wrap items-center gap-3 rounded border border-slate-700 bg-slate-900/40 p-3">
        <div className="inline-flex overflow-hidden rounded border border-slate-600 text-xs">
          {(['2d', '3d'] as Mode[]).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              className={`px-3 py-1 ${
                mode === m
                  ? 'bg-sky-700 text-white'
                  : 'bg-slate-800 text-slate-300 hover:bg-slate-700'
              }`}
            >
              {m.toUpperCase()}
            </button>
          ))}
        </div>

        {metricSelect('X', xMetric?.key ?? '', setXKey, [yKey, mode === '3d' ? zKey : ''])}
        {metricSelect('Y', yMetric?.key ?? '', setYKey, [xKey, mode === '3d' ? zKey : ''])}
        {mode === '3d' &&
          metricSelect('Z', zMetric?.key ?? '', setZKey, [xKey, yKey])}

        {allDims.length > 0 && (
          <label className="flex items-center gap-2 text-xs text-slate-300">
            <span className="font-semibold uppercase tracking-wider text-slate-400">
              Color by
            </span>
            <select
              value={colorDimKey}
              onChange={(ev) => setColorDimKey(ev.target.value)}
              className="rounded border border-slate-600 bg-slate-800 px-2 py-1 text-xs text-slate-200"
            >
              <option value={NONE_DIM_KEY}>(none)</option>
              {allDims.map((d) => (
                <option key={d.key} value={d.key}>
                  {d.key} ({d.kind})
                </option>
              ))}
            </select>
          </label>
        )}

        <label className="inline-flex items-center gap-2 text-xs text-slate-300">
          <input
            type="checkbox"
            checked={scalarize}
            onChange={(ev) => setScalarize(ev.target.checked)}
            className="rounded border-slate-500 bg-slate-800 text-sky-500"
          />
          Scalarize
        </label>

        <label className="ml-auto inline-flex items-center gap-2 text-xs text-slate-300">
          <input
            type="checkbox"
            checked={collapseEquivalents}
            onChange={(ev) => setCollapseEquivalents(ev.target.checked)}
            className="rounded border-slate-500 bg-slate-800 text-sky-500"
          />
          Collapse equivalents
        </label>
      </div>

      {/* Weighted-sum scalarization sliders */}
      {scalarize && activeMetrics.length > 0 && (
        <div className="flex flex-wrap items-center gap-4 rounded border border-slate-700 bg-slate-900/40 p-3">
          <span className="text-xs font-semibold uppercase tracking-wider text-slate-400">
            Weights
          </span>
          {activeMetrics.map((m) => (
            <label key={m.key} className="flex items-center gap-2 text-xs text-slate-300">
              <span className="text-slate-400">{m.label}</span>
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={weights[m.key] ?? 1 / activeMetrics.length}
                onChange={(ev) =>
                  setWeights((prev) => ({ ...prev, [m.key]: Number(ev.target.value) }))
                }
                className="w-24 accent-sky-500"
              />
              <span className="w-8 text-right text-slate-200">
                {(weights[m.key] ?? 1 / activeMetrics.length).toFixed(2)}
              </span>
            </label>
          ))}
          {top3Scalar && (
            <span className="ml-auto text-xs text-slate-400">
              Top 3:{' '}
              {top3Scalar
                .map((r, i) => `${i + 1}. ${r.name} (${r.scalar.toFixed(3)})`)
                .join(' · ')}
            </span>
          )}
        </div>
      )}

      {equivalentGroupCount > 0 && (
        <div className="rounded border border-emerald-700/40 bg-emerald-950/30 px-3 py-2 text-xs text-emerald-200">
          {equivalentGroupCount} equivalent group
          {equivalentGroupCount === 1 ? '' : 's'} detected — collapsing saves{' '}
          {collapsedSavings} duplicate point
          {collapsedSavings === 1 ? '' : 's'}.
        </div>
      )}

      <ChartPanel
        title={
          mode === '3d' && zMetric
            ? `Pareto front · ${xMetric?.label} × ${yMetric?.label} × ${zMetric.label}`
            : `Pareto front · ${xMetric?.label} × ${yMetric?.label}`
        }
        headerActions={
          <>
            <DownloadCsvButton label="Pareto points" rows={points} columns={csvColumns} />
            {chrome.headerActions}
          </>
        }
      >
        <PlotlyChart
          data={traces}
          layout={layout}
          config={chrome.config}
          onInitialized={chrome.onInitialized}
          height={mode === '3d' ? '560px' : '480px'}
          ariaLabel="Configurable Pareto front across the selected EST runs"
          onSelected={(ev) => focus.setFocused(focusIdsFromSelection(ev))}
          onDeselect={() => focus.clear()}
          onClick={handleClick}
        />
        {chrome.fullscreenOverlay}
      </ChartPanel>
    </div>
  );
}
