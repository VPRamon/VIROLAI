/**
 * Per-dataset tab — group manifests by dataset_id, show mean metrics ranked table.
 */
import { useMemo } from 'react';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import type { GroupRankingEntry } from '../../../lib/workspaces/types';
import type { ManifestSummary } from '../../../lib/workspaces/types';
import { Card, EmptyState, Skeleton, fmtNumber, fmtPercent } from '../../experiments/_ui';

function buildRanking(
  summaries: ManifestSummary[],
  groupKey: 'dataset_id' | 'algorithm_id',
): GroupRankingEntry[] {
  const map = new Map<string, ManifestSummary[]>();
  for (const s of summaries) {
    const k = s[groupKey] ?? '(unknown)';
    const arr = map.get(k) ?? [];
    arr.push(s);
    map.set(k, arr);
  }
  const entries: GroupRankingEntry[] = [];
  for (const [key, rows] of map) {
    entries.push({
      key,
      n: rows.length,
      mean_score: meanOf(rows, (r) => r.composite_rank_score),
      mean_completion: meanOf(rows, (r) => r.completion_ratio),
      mean_priority_sum: meanOf(rows, (r) => r.priority_sum),
      mean_utilization: meanOf(rows, (r) => r.utilization),
      mean_fragmentation_index: meanOf(rows, (r) => r.fragmentation_index),
    });
  }
  return entries.sort((a, b) => b.mean_score - a.mean_score);
}

function meanOf(
  rows: ManifestSummary[],
  fn: (r: ManifestSummary) => number | null,
): number {
  const vals = rows.map(fn).filter((v): v is number => v !== null);
  if (vals.length === 0) return 0;
  return vals.reduce((a, b) => a + b, 0) / vals.length;
}

export function RankingTable({
  entries,
  loading,
}: {
  entries: GroupRankingEntry[];
  loading: boolean;
}) {
  const max = useMemo(
    () => ({
      score: Math.max(0.0001, ...entries.map((e) => e.mean_score)),
      priority: Math.max(0.0001, ...entries.map((e) => e.mean_priority_sum)),
      util: Math.max(0.0001, ...entries.map((e) => e.mean_utilization)),
      frag: Math.max(0.0001, ...entries.map((e) => e.mean_fragmentation_index)),
    }),
    [entries],
  );

  if (loading && entries.length === 0) return <Skeleton className="h-72 w-full" />;

  if (entries.length === 0) {
    return (
      <EmptyState
        title="No data"
        description="Add manifests to compare."
      />
    );
  }

  return (
    <Card padded={false}>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="text-xs uppercase tracking-wide text-slate-500">
            <tr className="border-b border-slate-700">
              <th className="px-4 py-3 text-left">Group</th>
              <th className="px-4 py-3 text-right">Mean score</th>
              <th className="px-4 py-3 text-right">Completion</th>
              <th className="px-4 py-3 text-right">Priority Σ</th>
              <th className="px-4 py-3 text-right">Utilization</th>
              <th className="px-4 py-3 text-right">Fragmentation</th>
              <th className="px-4 py-3 text-right">N</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-700">
            {entries.map((e) => (
              <tr key={e.key} className="text-slate-200">
                <td className="px-4 py-3 font-medium text-white">{e.key}</td>
                <td className="px-4 py-3 text-right">
                  <Bar value={e.mean_score} max={max.score} format={fmtNumber} tone="indigo" />
                </td>
                <td className="px-4 py-3 text-right tabular-nums">
                  {fmtPercent(e.mean_completion)}
                </td>
                <td className="px-4 py-3 text-right">
                  <Bar value={e.mean_priority_sum} max={max.priority} format={fmtNumber} tone="emerald" />
                </td>
                <td className="px-4 py-3 text-right tabular-nums">
                  {fmtPercent(e.mean_utilization)}
                </td>
                <td className="px-4 py-3 text-right">
                  <Bar value={e.mean_fragmentation_index} max={max.frag} format={fmtNumber} tone="amber" />
                </td>
                <td className="px-4 py-3 text-right tabular-nums text-slate-400">{e.n}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Card>
  );
}

function Bar({
  value,
  max,
  format,
  tone,
}: {
  value: number;
  max: number;
  format: (v: number) => string;
  tone: 'indigo' | 'emerald' | 'amber';
}) {
  const pct = max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0;
  const colour =
    tone === 'indigo'
      ? 'bg-indigo-500/70'
      : tone === 'emerald'
        ? 'bg-emerald-500/70'
        : 'bg-amber-500/70';
  return (
    <div className="ml-auto flex w-32 items-center gap-2">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-slate-700">
        <div className={`h-full ${colour}`} style={{ width: `${pct}%` }} />
      </div>
      <span className="w-12 text-right tabular-nums text-slate-200">{format(value)}</span>
    </div>
  );
}

export default function PerDatasetTab() {
  const { summaries, loading } = useWorkspaceContext();
  const entries = useMemo(() => buildRanking(summaries, 'dataset_id'), [summaries]);

  return (
    <div className="space-y-4">
      <div className="text-sm text-slate-400">
        Manifests grouped by <code className="text-slate-300">dataset_id</code>, ranked by mean composite score.
      </div>
      <RankingTable entries={entries} loading={loading} />
    </div>
  );
}
