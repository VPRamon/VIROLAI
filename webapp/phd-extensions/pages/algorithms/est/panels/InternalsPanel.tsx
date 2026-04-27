/**
 * Internals panel — visualises EST algorithm internals from the per-iteration
 * trace stored alongside each run.
 *
 * Charts:
 *   - Best/median/worst FOM per iteration (multi-run overlay).
 *   - Beam fitness ridge (best beam evolution as filled area per run).
 *   - Wall-time per iteration.
 */
import { useMemo } from 'react';
import { ChartPanel, PlotlyChart } from '@/components';
import { usePlotlyChartChrome, usePlotlyTheme } from '@/hooks';
import type { Data, Layout } from 'plotly.js';
import type { EstTraceIteration, RunRow } from '../useRunMatrix';
import {
  INTERNALS_FOM_HELP,
  INTERNALS_HEATMAP_HELP,
  INTERNALS_WALL_HELP,
} from '../chartHelp';

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

export default function InternalsPanel({ runs }: { runs: RunRow[] }) {
  const plotlyTheme = usePlotlyTheme();

  const series = useMemo<Series[]>(
    () => runs.filter((r) => r.iterations?.length).map((r) => buildSeries(r.schedule.schedule_name, r.iterations!)),
    [runs],
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

  const fomChrome = usePlotlyChartChrome({
    label: 'Score trajectory',
    help: INTERNALS_FOM_HELP,
  });
  const wallChrome = usePlotlyChartChrome({
    label: 'Wall time per round',
    help: INTERNALS_WALL_HELP,
  });

  if (series.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-slate-600 py-16 text-center text-sm text-slate-400">
        No EST traces available for the selected runs.
        <div className="mt-2 text-xs">
          Re-run the experiment with <code className="rounded bg-slate-800 px-1">--trace</code> to
          enable per-iteration tracing.
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-5">
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

      <BeamHeatmap runs={runs} />

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
