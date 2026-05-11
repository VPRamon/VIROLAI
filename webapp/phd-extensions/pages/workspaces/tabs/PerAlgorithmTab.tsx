/**
 * Per-algorithm tab — group manifests by algorithm_id, show mean metrics ranked table.
 */
import { useMemo } from 'react';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import type { GroupRankingEntry } from '../../../lib/workspaces/types';
import type { ManifestSummary } from '../../../lib/workspaces/types';
import { RankingTable } from './PerDatasetTab';

function buildRanking(summaries: ManifestSummary[]): GroupRankingEntry[] {
  const map = new Map<string, ManifestSummary[]>();
  for (const s of summaries) {
    const k = s.algorithm_id ?? '(unknown)';
    const arr = map.get(k) ?? [];
    arr.push(s);
    map.set(k, arr);
  }
  const entries: GroupRankingEntry[] = [];
  for (const [key, rows] of map) {
    const mean = (fn: (r: ManifestSummary) => number | null) => {
      const vals = rows.map(fn).filter((v): v is number => v !== null);
      return vals.length === 0 ? 0 : vals.reduce((a, b) => a + b, 0) / vals.length;
    };
    entries.push({
      key,
      n: rows.length,
      mean_score: mean((r) => r.composite_rank_score),
      mean_completion: mean((r) => r.completion_ratio),
      mean_priority_sum: mean((r) => r.priority_sum),
      mean_utilization: mean((r) => r.utilization),
      mean_fragmentation_index: mean((r) => r.fragmentation_index),
    });
  }
  return entries.sort((a, b) => b.mean_score - a.mean_score);
}

export default function PerAlgorithmTab() {
  const { summaries, loading } = useWorkspaceContext();
  const entries = useMemo(() => buildRanking(summaries), [summaries]);

  return (
    <div className="space-y-4">
      <div className="text-sm text-slate-400">
        Manifests grouped by <code className="text-slate-300">algorithm_id</code>, ranked by mean composite score.
      </div>
      <RankingTable entries={entries} loading={loading} />
    </div>
  );
}
