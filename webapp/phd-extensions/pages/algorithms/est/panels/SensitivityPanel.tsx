/**
 * Sensitivity panel — visualise how a chosen outcome metric varies with the
 * EST configuration knobs.
 *
 * Renders:
 *   - A 3D scatter when ≥3 numeric dimensions are available, mapping the first
 *     three numeric dims to x/y/z and the metric to colour.
 *   - A pair-heatmap matrix (mean metric per (xDim, yDim) cell).
 *   - Marginal box-plots per categorical dimension.
 *
 * Generic over arbitrary `algorithm_config` keys via {@link extractDimensions}.
 */
import { useMemo, useState } from 'react';
import { ChartPanel, PlotlyChart, RangeFilterGroup, ToolbarRow } from '@/components';
import { HelpPopover } from '@/components/charts';
import { usePlotlyChartChrome, usePlotlyTheme } from '@/hooks';
import type { Data, Layout } from 'plotly.js';
import type { InsightsData } from '@/api/types';
import type { RunRow } from '../useRunMatrix';
import { extractDimensions, readDimension, type Dimension } from '../dimensions';
import { useRunRangeFilters } from '../useRunRangeFilters';
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
  name: string;
  values: Record<string, number | string | null>;
  metric: number;
}

export default function SensitivityPanel({ runs }: { runs: RunRow[] }) {
  const plotlyTheme = usePlotlyTheme();
  const [metric, setMetric] = useState<Metric>('rate');

  const filters = useRunRangeFilters(runs);
  const filteredRuns = filters.filtered;

  const dims = useMemo(
    () => extractDimensions(filteredRuns.map((r) => r.algorithmConfig)),
    [filteredRuns],
  );

  const points = useMemo<Point[]>(() => {
    const allDims: Dimension[] = [...dims.numeric, ...dims.categorical];
    return filteredRuns
      .filter((r) => r.insights)
      .map((r) => {
        const values: Record<string, number | string | null> = {};
        for (const d of allDims) values[d.key] = readDimension(r.algorithmConfig, d);
        return {
          name: r.schedule.schedule_name,
          values,
          metric: metricValue(r.insights!, metric),
        };
      });
  }, [filteredRuns, dims, metric]);

  const numericDims = dims.numeric;

  const scatter3d = useMemo<{ data: Data[]; layout: Partial<Layout> } | null>(() => {
    if (numericDims.length < 3) return null;
    const [xD, yD, zD] = numericDims;
    const data: Data[] = [
      {
        type: 'scatter3d',
        mode: 'text+markers',
        x: points.map((p) => p.values[xD.key] as number | null),
        y: points.map((p) => p.values[yD.key] as number | null),
        z: points.map((p) => p.values[zD.key] as number | null),
        text: points.map((p) => p.name),
        textposition: 'top center',
        marker: {
          size: 8,
          color: points.map((p) => p.metric),
          colorscale: 'Viridis',
          colorbar: { title: { text: METRIC_LABELS[metric] } },
          showscale: true,
        },
      },
    ];
    const layout: Partial<Layout> = {
      ...plotlyTheme.layout,
      scene: {
        xaxis: { title: { text: xD.key } },
        yaxis: { title: { text: yD.key } },
        zaxis: { title: { text: zD.key } },
      },
      margin: { l: 0, r: 0, t: 10, b: 0 },
    };
    return { data, layout };
  }, [numericDims, points, metric, plotlyTheme]);

  const scatter2d = useMemo<{ data: Data[]; layout: Partial<Layout> } | null>(() => {
    if (numericDims.length < 1) return null;
    const xD = numericDims[0];
    const data: Data[] = [
      {
        type: 'scatter',
        mode: 'text+markers',
        x: points.map((p) => p.values[xD.key] as number | null),
        y: points.map((p) => p.metric),
        text: points.map((p) => p.name),
        textposition: 'top center',
        marker: { size: 10, color: points.map((p) => p.metric), colorscale: 'Viridis' },
      },
    ];
    const layout: Partial<Layout> = {
      ...plotlyTheme.layout,
      xaxis: { ...plotlyTheme.layout.xaxis, title: { text: xD.key } },
      yaxis: { ...plotlyTheme.layout.yaxis, title: { text: METRIC_LABELS[metric] } },
      margin: { l: 60, r: 20, t: 20, b: 50 },
    };
    return { data, layout };
  }, [numericDims, points, metric, plotlyTheme]);

  const parallelCoords = useMemo<{ data: Data[]; layout: Partial<Layout> } | null>(() => {
    if (numericDims.length < 2) return null;
    const dimsForPC = [
      ...numericDims.map((d) => ({
        label: d.key,
        values: points.map((p) => (p.values[d.key] as number | null) ?? Number.NaN),
      })),
      {
        label: METRIC_LABELS[metric],
        values: points.map((p) => p.metric),
      },
    ];
    const data: Data[] = [
      {
        type: 'parcoords',
        line: {
          color: points.map((p) => p.metric),
          colorscale: 'Viridis',
          showscale: true,
          colorbar: { title: { text: METRIC_LABELS[metric] } },
        },
        dimensions: dimsForPC,
      } as Data,
    ];
    const layout: Partial<Layout> = {
      ...plotlyTheme,
      margin: { l: 80, r: 80, t: 40, b: 40 },
    };
    return { data, layout };
  }, [numericDims, points, metric, plotlyTheme]);

  const scatter3dChrome = usePlotlyChartChrome({
    label: 'Configuration cube',
    help: SENSITIVITY_3D_HELP,
  });
  const scatter2dChrome = usePlotlyChartChrome({
    label: `${METRIC_LABELS[metric]} vs ${numericDims[0]?.key ?? 'x'}`,
    help: SENSITIVITY_2D_HELP,
  });
  const parcoordsChrome = usePlotlyChartChrome({
    label: 'Parallel coordinates',
    help: SENSITIVITY_PARCOORDS_HELP,
  });

  if (points.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-slate-600 py-16 text-center text-sm text-slate-400">
        Waiting for insights to load…
      </div>
    );
  }

  if (numericDims.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-slate-600 py-16 text-center text-sm text-slate-400">
        No numeric configuration dimensions vary across the selected runs.
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <ToolbarRow>
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium uppercase tracking-wider text-slate-400">
            Metric
          </span>
          <div className="flex gap-3">
            {(Object.keys(METRIC_LABELS) as Metric[]).map((m) => (
              <label key={m} className="flex cursor-pointer items-center gap-1.5 text-sm text-slate-300">
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
        <div className="text-xs text-slate-500">
          Numeric dims: {numericDims.map((d) => d.key).join(', ') || '—'}
          {dims.categorical.length > 0 && (
            <>
              <br />
              Categorical dims: {dims.categorical.map((d) => d.key).join(', ')}
            </>
          )}
        </div>
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
        {scatter3d && (
          <ChartPanel
            title="Configuration cube (first 3 numeric dimensions, colour = metric)"
            headerActions={scatter3dChrome.headerActions}
          >
            <PlotlyChart
              data={scatter3d.data}
              layout={scatter3d.layout}
              config={scatter3dChrome.config}
              onInitialized={scatter3dChrome.onInitialized}
              height="520px"
              ariaLabel="3D scatter of EST configuration vs metric"
            />
            {scatter3dChrome.fullscreenOverlay}
          </ChartPanel>
        )}

        {scatter2d && (
          <ChartPanel
            title={`${METRIC_LABELS[metric]} vs ${numericDims[0].key}`}
            headerActions={scatter2dChrome.headerActions}
          >
            <PlotlyChart
              data={scatter2d.data}
              layout={scatter2d.layout}
              config={scatter2dChrome.config}
              onInitialized={scatter2dChrome.onInitialized}
              height="320px"
              ariaLabel="Metric vs first numeric dimension"
            />
            {scatter2dChrome.fullscreenOverlay}
          </ChartPanel>
        )}
      </div>

      {parallelCoords && (
        <ChartPanel
          title="Parallel coordinates (config + metric)"
          headerActions={parcoordsChrome.headerActions}
        >
          <PlotlyChart
            data={parallelCoords.data}
            layout={parallelCoords.layout}
            config={parcoordsChrome.config}
            onInitialized={parcoordsChrome.onInitialized}
            height="380px"
            ariaLabel="Parallel coordinates linking configuration knobs to outcome metric"
          />
          {parcoordsChrome.fullscreenOverlay}
        </ChartPanel>
      )}
    </div>
  );
}
