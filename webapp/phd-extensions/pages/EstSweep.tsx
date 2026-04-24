/**
 * EST Sweep — compare multiple EST runs that differ in e/k/b parameters.
 *
 * Schedule names are expected to encode their parameters using the pattern:
 *   e{e}_k{k}_b{b}   (e.g.  "est_e2_k5_b3_2024")
 *
 * Any schedule whose name does not match is shown with null values for
 * those parameters.
 */
import { useEffect, useMemo, useState } from 'react';
import Plot from 'react-plotly.js';
import {
  ChartPanel,
  DataTable,
  LoadingSpinner,
  PageContainer,
  PageHeader,
} from '@/components';
import type { TableColumn } from '@/components';
import { useInsights, usePlotlyTheme, useSchedules } from '@/hooks';
import type { InsightsData, ScheduleInfo } from '@/api/types';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

interface EstParams {
  e: number | null;
  k: number | null;
  b: number | null;
}

function parseEstParams(name: string): EstParams {
  const m = name.match(/e(\d+)[_\-\s]*k(\d+)[_\-\s]*b(\d+)/i);
  if (!m) return { e: null, k: null, b: null };
  return { e: Number(m[1]), k: Number(m[2]), b: Number(m[3]) };
}

type Axis = 'e' | 'k' | 'b';

const AXIS_LABELS: Record<Axis, string> = {
  e: 'e (endangered threshold)',
  k: 'k (beam width)',
  b: 'b (branching factor)',
};

// ---------------------------------------------------------------------------
// Sub-component: loads insights for one schedule and bubbles up via callback
// ---------------------------------------------------------------------------

function ScheduleInsightsLoader({
  scheduleId,
  onData,
}: {
  scheduleId: number;
  onData: (id: number, data: InsightsData) => void;
}) {
  const { data, isLoading } = useInsights(scheduleId);
  useEffect(() => {
    if (data && !isLoading) onData(scheduleId, data);
  }, [data, isLoading, scheduleId, onData]);
  return isLoading ? <LoadingSpinner size="sm" /> : null;
}

// ---------------------------------------------------------------------------
// Row shapes
// ---------------------------------------------------------------------------

type AnnotatedSchedule = ScheduleInfo & EstParams;

interface ResultRow {
  id: number;
  name: string;
  e: number | null;
  k: number | null;
  b: number | null;
  scheduled: string;
  rate: string;
  mean_priority: string;
}

interface ChartRow {
  scheduleId: number;
  e: number | null;
  k: number | null;
  b: number | null;
  insights: InsightsData | undefined;
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export default function EstSweep() {
  const { data: schedulesResp, isLoading: schedulesLoading } = useSchedules();
  const plotlyTheme = usePlotlyTheme();

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [axis, setAxis] = useState<Axis>('e');
  const [insightsCache, setInsightsCache] = useState<Record<number, InsightsData>>({});

  const handleInsightsData = useMemo(
    () => (id: number, data: InsightsData) => {
      setInsightsCache((prev) =>
        prev[id] === data ? prev : { ...prev, [id]: data },
      );
    },
    [],
  );

  const toggleSchedule = (id: number) => {
    setSelectedIds((prev: Set<number>) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const annotated = useMemo<AnnotatedSchedule[]>(
    () =>
      (schedulesResp?.schedules ?? []).map((s) => ({
        ...s,
        ...parseEstParams(s.schedule_name),
      })),
    [schedulesResp],
  );

  // Selection table columns — accessor is a render function when custom content needed
  const selectionColumns = useMemo<TableColumn<AnnotatedSchedule>[]>(
    () => [
      {
        header: '',
        srOnly: true,
        accessor: (row) => (
          <input
            type="checkbox"
            checked={selectedIds.has(row.schedule_id)}
            onChange={() => toggleSchedule(row.schedule_id)}
            className="h-4 w-4 cursor-pointer accent-primary-500"
            aria-label={`Select ${row.schedule_name}`}
          />
        ),
        width: 'w-8',
        align: 'center',
      },
      { header: 'Schedule', accessor: 'schedule_name' },
      { header: 'e', accessor: (r) => r.e ?? '—', align: 'center', width: 'w-12' },
      { header: 'k', accessor: (r) => r.k ?? '—', align: 'center', width: 'w-12' },
      { header: 'b', accessor: (r) => r.b ?? '—', align: 'center', width: 'w-12' },
    ],
    [selectedIds],
  );

  const resultRows = useMemo<ResultRow[]>(
    () =>
      annotated
        .filter((s: AnnotatedSchedule) => selectedIds.has(s.schedule_id))
        .map((s: AnnotatedSchedule) => {
          const ins = insightsCache[s.schedule_id];
          return {
            id: s.schedule_id,
            name: s.schedule_name,
            e: s.e,
            k: s.k,
            b: s.b,
            scheduled: ins ? String(ins.metrics.scheduled_count) : '…',
            rate: ins ? `${(ins.metrics.scheduling_rate * 100).toFixed(1)} %` : '…',
            mean_priority: ins
              ? ins.metrics.mean_priority_scheduled.toFixed(2)
              : '…',
          };
        }),
    [annotated, selectedIds, insightsCache],
  );

  const resultColumns = useMemo<TableColumn<ResultRow>[]>(
    () => [
      { header: 'Schedule', accessor: 'name' },
      { header: 'e', accessor: (r) => r.e ?? '—', align: 'center', width: 'w-12' },
      { header: 'k', accessor: (r) => r.k ?? '—', align: 'center', width: 'w-12' },
      { header: 'b', accessor: (r) => r.b ?? '—', align: 'center', width: 'w-12' },
      { header: 'Scheduled', accessor: 'scheduled', align: 'right' },
      { header: 'Rate', accessor: 'rate', align: 'right' },
      { header: 'Mean priority (sched.)', accessor: 'mean_priority', align: 'right' },
    ],
    [],
  );

  // ---------------------------------------------------------------------------
  // Chart traces
  // ---------------------------------------------------------------------------

  const chartRows = useMemo<ChartRow[]>(
    () =>
      annotated
        .filter((s: AnnotatedSchedule) => selectedIds.has(s.schedule_id))
        .map((s: AnnotatedSchedule) => ({
          scheduleId: s.schedule_id,
          e: s.e,
          k: s.k,
          b: s.b,
          insights: insightsCache[s.schedule_id],
        })),
    [annotated, selectedIds, insightsCache],
  );

  function groupLabel(row: ChartRow, varyAxis: Axis): string {
    const others = (['e', 'k', 'b'] as Axis[]).filter((a) => a !== varyAxis);
    return others.map((a) => `${a}=${row[a] ?? '?'}`).join(', ');
  }

  function buildTraces(
    metric: (ins: InsightsData) => number,
    varyAxis: Axis,
  ): Plotly.Data[] {
    const groups = new Map<string, ChartRow[]>();
    for (const row of chartRows) {
      const key = groupLabel(row, varyAxis);
      const arr = groups.get(key) ?? [];
      arr.push(row);
      groups.set(key, arr);
    }
    return Array.from(groups.entries()).map(([label, rows]) => {
      const sorted = [...rows].sort(
        (a, b) => (a[varyAxis] ?? 0) - (b[varyAxis] ?? 0),
      );
      return {
        type: 'scatter' as const,
        mode: 'lines+markers' as const,
        name: label || 'all',
        x: sorted.map((r) => r[varyAxis]),
        y: sorted.map((r) => (r.insights ? metric(r.insights) : null)),
        connectgaps: false,
      };
    });
  }

  const rateTraces = buildTraces((ins) => ins.metrics.scheduling_rate * 100, axis);
  const countTraces = buildTraces((ins) => ins.metrics.scheduled_count, axis);
  const priorityTraces = buildTraces(
    (ins) => ins.metrics.mean_priority_scheduled,
    axis,
  );

  const sharedLayout: Partial<Plotly.Layout> = {
    ...plotlyTheme.layout,
    xaxis: { title: { text: AXIS_LABELS[axis] } },
    legend: { orientation: 'h' as const, y: -0.2 },
    margin: { t: 20, b: 60, l: 50, r: 20 },
    height: 320,
  };

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  const selectedIdsArray = useMemo(
    () => Array.from(selectedIds) as number[],
    [selectedIds],
  );

  return (
    <PageContainer>
      <PageHeader
        title="EST Parameter Sweep"
        description="Compare scheduling outcomes across runs that differ in e, k, or b"
      />

      {/* Invisible loaders — mount one per selected schedule to fetch insights */}
      {selectedIdsArray.map((id: number) => (
        <ScheduleInsightsLoader key={id} scheduleId={id} onData={handleInsightsData} />
      ))}

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        {/* Left: schedule selection */}
        <div className="lg:col-span-1">
          <ChartPanel title="Select Schedules" className="h-full">
            {schedulesLoading ? (
              <LoadingSpinner />
            ) : (
              <DataTable
                data={annotated}
                columns={selectionColumns}
                keyAccessor={(r: AnnotatedSchedule) => r.schedule_id}
                caption="Available schedules"
                captionHidden
              />
            )}
          </ChartPanel>
        </div>

        {/* Right: axis selector + charts + results table */}
        <div className="space-y-6 lg:col-span-2">
          <ChartPanel title="Vary axis">
            <div className="flex gap-4">
              {(['e', 'k', 'b'] as Axis[]).map((a) => (
                <label key={a} className="flex cursor-pointer items-center gap-2 text-sm">
                  <input
                    type="radio"
                    name="sweep-axis"
                    value={a}
                    checked={axis === a}
                    onChange={() => setAxis(a)}
                    className="accent-primary-500"
                  />
                  <span className="font-mono font-semibold">{a}</span>
                  <span className="text-slate-400">— {AXIS_LABELS[a]}</span>
                </label>
              ))}
            </div>
          </ChartPanel>

          {selectedIds.size === 0 ? (
            <div className="rounded-lg border border-dashed border-slate-600 py-16 text-center text-slate-400">
              Select schedules on the left to begin comparison
            </div>
          ) : (
            <>
              <ChartPanel title={`Scheduling rate (%) vs ${axis}`}>
                <Plot
                  data={rateTraces}
                  layout={{ ...sharedLayout, yaxis: { title: { text: 'Rate (%)' } } }}
                  config={{ ...plotlyTheme.config, responsive: true }}
                  style={{ width: '100%' }}
                />
              </ChartPanel>

              <ChartPanel title={`Scheduled count vs ${axis}`}>
                <Plot
                  data={countTraces}
                  layout={{ ...sharedLayout, yaxis: { title: { text: 'Count' } } }}
                  config={{ ...plotlyTheme.config, responsive: true }}
                  style={{ width: '100%' }}
                />
              </ChartPanel>

              <ChartPanel title={`Mean priority (scheduled) vs ${axis}`}>
                <Plot
                  data={priorityTraces}
                  layout={{ ...sharedLayout, yaxis: { title: { text: 'Mean priority' } } }}
                  config={{ ...plotlyTheme.config, responsive: true }}
                  style={{ width: '100%' }}
                />
              </ChartPanel>

              <ChartPanel title="Summary table">
                <DataTable
                  data={resultRows}
                  columns={resultColumns}
                  keyAccessor={(r: ResultRow) => r.id}
                  caption="EST sweep results"
                  captionHidden
                />
              </ChartPanel>
            </>
          )}
        </div>
      </div>
    </PageContainer>
  );
}

