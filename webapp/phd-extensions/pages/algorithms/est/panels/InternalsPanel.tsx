/**
 * Internals panel — visualises EST algorithm internals from the per-iteration
 * trace stored alongside each run.
 *
 * Charts:
 *   - Best/median/worst FOM per iteration (multi-run overlay).
 *   - Beam fitness ridge (best beam evolution as filled area per run).
 *   - Wall-time per iteration.
 *   - Convergence summary card (rounds-to-best, final gap, improvement rate).
 *   - Normalized best-score trajectory (best_so_far / max).
 *   - Per-round beam-score diversity (std of beam_scores).
 */
import { useMemo } from 'react';
import { ChartPanel, EmptyState, PlotlyChart, TableSkeleton } from '@/components';
import { usePlotlyChartChrome, usePlotlyTheme } from '@/hooks';
import type { Data, Layout } from 'plotly.js';
import type { EstTraceIteration, RunRow } from '../useRunMatrix';
import { useRunFocus } from '../useRunFocus';
import { FocusBadge } from '../FocusBadge';
import { booleanCodec, stringCodec, useUrlState } from '../../../../lib/useUrlState';
import {
  INTERNALS_FOM_HELP,
  INTERNALS_HEATMAP_HELP,
  INTERNALS_WALL_HELP,
} from '../chartHelp';
import {
  bestSoFar,
  computeDiversityTrajectory,
  computeFinalGapToBest,
  computeImprovementRate,
  computeRoundsToBest,
} from './InternalsPanel.helpers';

export {
  bestSoFar,
  computeDiversityTrajectory,
  computeFinalGapToBest,
  computeImprovementRate,
  computeNormalizedTrajectory,
  computeRoundsToBest,
} from './InternalsPanel.helpers';

const num = (v: unknown): number | null => {
  if (typeof v === 'number' && Number.isFinite(v)) return v;
  return null;
};

interface Series {
  name: string;
  rounds: number[];
  bestScore: Array<number | null>;
  medianScore: Array<number | null>;
  worstScore: Array<number | null>;
  wallMs: Array<number | null>;
}

function buildSeries(name: string, iters: EstTraceIteration[]): Series {
  const rounds: number[] = [];
  const bestScore: Array<number | null> = [];
  const medianScore: Array<number | null> = [];
  const worstScore: Array<number | null> = [];
  const wallMs: Array<number | null> = [];
  for (const it of iters) {
    rounds.push(num(it.round) ?? rounds.length);
    bestScore.push(num(it.best_score));
    medianScore.push(num(it.median_score));
    worstScore.push(num(it.worst_score));
    wallMs.push(num(it.wall_ms));
  }
  return { name, rounds, bestScore, medianScore, worstScore, wallMs };
}

type NormMetric = 'best' | 'median' | 'worst';
const NORM_METRIC_LABELS: Record<NormMetric, string> = {
  best: 'Best score',
  median: 'Median score',
  worst: 'Worst score',
};

function pickMetric(it: EstTraceIteration, metric: NormMetric): number | null {
  if (metric === 'best') return num(it.best_score);
  if (metric === 'median') return num(it.median_score);
  return num(it.worst_score);
}

function bestSoFarOf(
  iters: EstTraceIteration[],
  metric: NormMetric,
): Array<{ round: number; value: number }> {
  if (metric === 'best') return bestSoFar(iters);
  const out: Array<{ round: number; value: number }> = [];
  let running = -Infinity;
  iters.forEach((it, i) => {
    const v = pickMetric(it, metric);
    const r = num(it.round) ?? i;
    if (v !== null && v > running) running = v;
    if (running !== -Infinity) out.push({ round: r, value: running });
  });
  return out;
}

function fmtNumber(v: number | null, digits = 3): string {
  if (v === null || !Number.isFinite(v)) return '—';
  return v.toFixed(digits);
}

export default function InternalsPanel({
  runs,
  loading = false,
}: {
  runs: RunRow[];
  loading?: boolean;
}) {
  const plotlyTheme = usePlotlyTheme();
  const focus = useRunFocus();
  const effectiveRuns = useMemo(() => focus.apply(runs), [focus, runs]);

  const [normMetric, setNormMetric] = useUrlState<string>('est_internals_norm_metric', 'best', {
    codec: stringCodec,
  });
  const [showDiversity, setShowDiversity] = useUrlState<boolean>(
    'est_internals_show_diversity',
    true,
    { codec: booleanCodec },
  );
  const safeNormMetric: NormMetric =
    normMetric === 'median' || normMetric === 'worst' ? normMetric : 'best';

  const series = useMemo<Series[]>(
    () =>
      effectiveRuns
        .filter((r) => r.iterations?.length)
        .map((r) => buildSeries(r.schedule.schedule_name, r.iterations!)),
    [effectiveRuns],
  );

  const tracedRuns = useMemo(
    () => effectiveRuns.filter((r) => r.iterations && r.iterations.length > 0),
    [effectiveRuns],
  );

  const fomTraces = useMemo<Data[]>(() => {
    const out: Data[] = [];
    series.forEach((s, i) => {
      const colour = `hsl(${(i * 60) % 360}, 70%, 60%)`;
      out.push({
        type: 'scatter',
        mode: 'lines',
        name: `${s.name} · best`,
        x: s.rounds,
        y: s.bestScore,
        line: { color: colour, width: 2 },
        legendgroup: s.name,
      });
      out.push({
        type: 'scatter',
        mode: 'lines',
        name: `${s.name} · median`,
        x: s.rounds,
        y: s.medianScore,
        line: { color: colour, width: 1, dash: 'dot' },
        legendgroup: s.name,
        showlegend: false,
      });
      out.push({
        type: 'scatter',
        mode: 'lines',
        name: `${s.name} · worst`,
        x: s.rounds,
        y: s.worstScore,
        line: { color: colour, width: 1, dash: 'dash' },
        legendgroup: s.name,
        showlegend: false,
        opacity: 0.5,
      });
    });
    return out;
  }, [series]);

  const fomLayout = useMemo<Partial<Layout>>(
    () => ({
      ...plotlyTheme.layout,
      xaxis: { ...plotlyTheme.layout.xaxis, title: { text: 'Round' } },
      yaxis: { ...plotlyTheme.layout.yaxis, title: { text: 'Score' } },
      margin: { l: 60, r: 20, t: 20, b: 50 },
      legend: { orientation: 'h', y: -0.18 },
    }),
    [plotlyTheme],
  );

  const wallTraces = useMemo<Data[]>(
    () =>
      series.map((s, i) => ({
        type: 'scatter',
        mode: 'lines',
        name: s.name,
        x: s.rounds,
        y: s.wallMs,
        line: { color: `hsl(${(i * 60) % 360}, 70%, 60%)`, width: 2 },
      })),
    [series],
  );

  const wallLayout = useMemo<Partial<Layout>>(
    () => ({
      ...plotlyTheme.layout,
      xaxis: { ...plotlyTheme.layout.xaxis, title: { text: 'Round' } },
      yaxis: { ...plotlyTheme.layout.yaxis, title: { text: 'Wall time (ms)' } },
      margin: { l: 60, r: 20, t: 20, b: 50 },
      legend: { orientation: 'h', y: -0.18 },
    }),
    [plotlyTheme],
  );

  const normalizedTraces = useMemo<Data[]>(
    () =>
      tracedRuns.map((r, i) => {
        const pts = bestSoFarOf(r.iterations!, safeNormMetric);
        const colour = `hsl(${(i * 60) % 360}, 70%, 60%)`;
        if (pts.length === 0) {
          return {
            type: 'scatter',
            mode: 'lines',
            name: r.schedule.schedule_name,
            x: [],
            y: [],
            line: { color: colour, width: 2 },
          } satisfies Data;
        }
        const max = pts[pts.length - 1].value;
        const safe = Number.isFinite(max) && max !== 0 ? max : 1;
        return {
          type: 'scatter',
          mode: 'lines',
          name: r.schedule.schedule_name,
          x: pts.map((p) => p.round),
          y: pts.map((p) => p.value / safe),
          line: { color: colour, width: 2 },
        } satisfies Data;
      }),
    [tracedRuns, safeNormMetric],
  );

  const normalizedLayout = useMemo<Partial<Layout>>(
    () => ({
      ...plotlyTheme.layout,
      xaxis: { ...plotlyTheme.layout.xaxis, title: { text: 'Round' } },
      yaxis: {
        ...plotlyTheme.layout.yaxis,
        title: { text: `${NORM_METRIC_LABELS[safeNormMetric]} / max` },
        range: [0, 1.05],
      },
      margin: { l: 60, r: 20, t: 20, b: 50 },
      legend: { orientation: 'h', y: -0.18 },
    }),
    [plotlyTheme, safeNormMetric],
  );

  const diversityTraces = useMemo<Data[]>(
    () =>
      tracedRuns.map((r, i) => {
        const { rounds, std } = computeDiversityTrajectory(r.iterations!);
        return {
          type: 'scatter',
          mode: 'lines',
          name: r.schedule.schedule_name,
          x: rounds,
          y: std,
          line: { color: `hsl(${(i * 60) % 360}, 70%, 60%)`, width: 2 },
          connectgaps: false,
        } satisfies Data;
      }),
    [tracedRuns],
  );

  const diversityLayout = useMemo<Partial<Layout>>(
    () => ({
      ...plotlyTheme.layout,
      xaxis: { ...plotlyTheme.layout.xaxis, title: { text: 'Round' } },
      yaxis: { ...plotlyTheme.layout.yaxis, title: { text: 'std(beam_scores)' } },
      margin: { l: 60, r: 20, t: 20, b: 50 },
      legend: { orientation: 'h', y: -0.18 },
    }),
    [plotlyTheme],
  );

  const fomChrome = usePlotlyChartChrome({
    label: 'Score trajectory',
    help: INTERNALS_FOM_HELP,
  });
  const wallChrome = usePlotlyChartChrome({
    label: 'Wall time per round',
    help: INTERNALS_WALL_HELP,
  });
  const normalizedChrome = usePlotlyChartChrome({ label: 'Normalized best-score trajectory' });
  const diversityChrome = usePlotlyChartChrome({ label: 'Per-round beam diversity' });

  if (loading) {
    return <TableSkeleton rows={6} columns={4} />;
  }

  if (series.length === 0) {
    return (
      <EmptyState
        title="No convergence data"
        hint="Run an EST experiment to populate iteration traces."
      />
    );
  }

  return (
    <div className="space-y-5">
      <FocusBadge />

      <ChartPanel title="Convergence summary">
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs text-slate-200">
            <thead className="text-[11px] uppercase tracking-wide text-slate-400">
              <tr>
                <th className="px-2 py-1">Run</th>
                <th className="px-2 py-1">Rounds to best</th>
                <th className="px-2 py-1">Final gap to best</th>
                <th className="px-2 py-1">Improvement rate (slope/round)</th>
              </tr>
            </thead>
            <tbody>
              {tracedRuns.map((r) => {
                const iters = r.iterations!;
                return (
                  <tr key={r.schedule.schedule_id} className="border-t border-slate-800/60">
                    <td className="px-2 py-1 font-medium text-slate-100">
                      {r.schedule.schedule_name}
                    </td>
                    <td className="px-2 py-1">{fmtNumber(computeRoundsToBest(iters), 0)}</td>
                    <td className="px-2 py-1">{fmtNumber(computeFinalGapToBest(iters), 4)}</td>
                    <td className="px-2 py-1">{fmtNumber(computeImprovementRate(iters), 4)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </ChartPanel>

      <ChartPanel
        title="Score trajectory (best / median / worst per round)"
        headerActions={fomChrome.headerActions}
      >
        <PlotlyChart
          data={fomTraces}
          layout={fomLayout}
          config={fomChrome.config}
          onInitialized={fomChrome.onInitialized}
          height="420px"
          ariaLabel="EST score trajectory across rounds"
        />
        {fomChrome.fullscreenOverlay}
      </ChartPanel>

      <ChartPanel
        title="Normalized best-score trajectory"
        headerActions={
          <div className="flex items-center gap-2 text-xs text-slate-300">
            <label className="flex items-center gap-1">
              <span className="text-slate-400">Metric</span>
              <select
                value={safeNormMetric}
                onChange={(e) => setNormMetric(e.target.value)}
                className="rounded border border-slate-600 bg-slate-800 px-1 py-0.5 text-xs"
              >
                <option value="best">Best</option>
                <option value="median">Median</option>
                <option value="worst">Worst</option>
              </select>
            </label>
            {normalizedChrome.headerActions}
          </div>
        }
      >
        <PlotlyChart
          data={normalizedTraces}
          layout={normalizedLayout}
          config={normalizedChrome.config}
          onInitialized={normalizedChrome.onInitialized}
          height="320px"
          ariaLabel="Normalized best-so-far trajectory across rounds"
        />
        {normalizedChrome.fullscreenOverlay}
      </ChartPanel>

      <BeamHeatmap runs={effectiveRuns} />

      <ChartPanel
        title="Diversity (per-round beam std)"
        headerActions={
          <div className="flex items-center gap-2 text-xs text-slate-300">
            <label className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={showDiversity}
                onChange={(e) => setShowDiversity(e.target.checked)}
              />
              <span>Show chart</span>
            </label>
            {diversityChrome.headerActions}
          </div>
        }
      >
        {showDiversity ? (
          <PlotlyChart
            data={diversityTraces}
            layout={diversityLayout}
            config={diversityChrome.config}
            onInitialized={diversityChrome.onInitialized}
            height="320px"
            ariaLabel="Per-round standard deviation of beam scores"
          />
        ) : (
          <div className="py-6 text-center text-xs text-slate-400">Diversity chart hidden.</div>
        )}
        {diversityChrome.fullscreenOverlay}
      </ChartPanel>

      <ChartPanel title="Wall time per round" headerActions={wallChrome.headerActions}>
        <PlotlyChart
          data={wallTraces}
          layout={wallLayout}
          config={wallChrome.config}
          onInitialized={wallChrome.onInitialized}
          height="320px"
          ariaLabel="Wall-time per EST round"
        />
        {wallChrome.fullscreenOverlay}
      </ChartPanel>
    </div>
  );
}

/** Heatmap of beam scores across rounds for the first run that has a trace. */
function BeamHeatmap({ runs }: { runs: RunRow[] }) {
  const plotlyTheme = usePlotlyTheme();
  const target = useMemo(() => runs.find((r) => r.iterations?.length), [runs]);
  const chrome = usePlotlyChartChrome({
    label: 'Beam-score heatmap',
    help: INTERNALS_HEATMAP_HELP,
  });

  const { data, layout } = useMemo<{ data: Data[]; layout: Partial<Layout> }>(() => {
    if (!target?.iterations?.length) return { data: [], layout: {} };
    const iters = target.iterations;
    const maxBeams = iters.reduce((m, it) => Math.max(m, it.beam_scores?.length ?? 0), 0);
    const z: Array<Array<number | null>> = Array.from({ length: maxBeams }, () => []);
    const x: number[] = [];
    iters.forEach((it, i) => {
      const round = num(it.round) ?? i;
      x.push(round);
      const sorted = (it.beam_scores ?? []).slice().sort((a, b) => b - a);
      for (let row = 0; row < maxBeams; row++) {
        z[row].push(row < sorted.length ? sorted[row] : null);
      }
    });
    return {
      data: [
        {
          type: 'heatmap',
          x,
          z,
          colorscale: 'Viridis',
          colorbar: { title: { text: 'Score' } },
        },
      ],
      layout: {
        ...plotlyTheme.layout,
        xaxis: { ...plotlyTheme.layout.xaxis, title: { text: 'Round' } },
        yaxis: { ...plotlyTheme.layout.yaxis, title: { text: 'Beam rank (best → worst)' } },
        margin: { l: 60, r: 20, t: 20, b: 50 },
      },
    };
  }, [target, plotlyTheme]);

  if (!target) {
    return (
      <ChartPanel title="Beam-score distribution per round">
        <div className="py-6 text-center text-sm text-slate-400">No trace available.</div>
      </ChartPanel>
    );
  }
  return (
    <ChartPanel
      title="Beam-score distribution per round (heatmap, first selected run)"
      headerActions={chrome.headerActions}
    >
      <PlotlyChart
        data={data}
        layout={layout}
        config={chrome.config}
        onInitialized={chrome.onInitialized}
        height="320px"
        ariaLabel={`Beam-score heatmap for ${target.schedule.schedule_name}`}
      />
      {chrome.fullscreenOverlay}
    </ChartPanel>
  );
}
