/**
 * Per-dataset tab — for each dataset, ranked algorithm/config table
 * with weight controls. Backed by `GET /ranking?by=dataset` + the
 * ranking endpoint's optional weight query parameters.
 *
 * To keep the v1 surface manageable, the ranking is per dataset taken
 * as a *whole* (the backend already aggregates by dataset). For
 * intra-dataset breakdowns by algorithm/config the user can drill into
 * the matrix.
 */
import { useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';
import { getRanking } from '../../../lib/experiments/api';
import { useAsync } from '../../../lib/experiments/useAsync';
import type { RankingWeights } from '../../../lib/experiments/types';
import {
  Card,
  EmptyState,
  ErrorState,
  Skeleton,
  TextField,
  fmtNumber,
  fmtPercent,
} from '../_ui';

const DEFAULT_WEIGHTS: RankingWeights = {
  completion: 1.0,
  priority: 1.0,
  utilization: 1.0,
  fragmentation: 1.0,
};

export default function PerDatasetTab() {
  const { slug = '', runId = '' } = useParams();
  const [weights, setWeights] = useState<RankingWeights>(DEFAULT_WEIGHTS);

  const { data, error, loading, reload } = useAsync(
    () => getRanking(slug, runId, { by: 'dataset', weights }),
    [slug, runId, weights.completion, weights.priority, weights.utilization, weights.fragmentation],
  );

  const max = useMemo(() => {
    const ms = data?.entries ?? [];
    return {
      score: Math.max(0.0001, ...ms.map((e) => e.mean_score)),
      priority: Math.max(0.0001, ...ms.map((e) => e.mean_priority_sum)),
      util: Math.max(0.0001, ...ms.map((e) => e.mean_utilization)),
      frag: Math.max(0.0001, ...ms.map((e) => e.mean_fragmentation_index)),
    };
  }, [data]);

  return (
    <div className="space-y-4">
      <Card padded>
        <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-300">
          Composite weights
        </h3>
        <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
          {(['completion', 'priority', 'utilization', 'fragmentation'] as const).map((k) => (
            <TextField
              key={k}
              label={k}
              type="number"
              step="0.1"
              min="0"
              value={String(weights[k])}
              onChange={(e) => {
                const v = Number.parseFloat(e.target.value);
                setWeights((w) => ({ ...w, [k]: Number.isFinite(v) ? v : 0 }));
              }}
            />
          ))}
        </div>
      </Card>

      {error && <ErrorState error={error} onRetry={reload} />}
      {!error && loading && <Skeleton className="h-72 w-full" />}
      {!error && !loading && (data?.entries ?? []).length === 0 && (
        <EmptyState title="No ranked datasets yet" />
      )}
      {!error && !loading && (data?.entries ?? []).length > 0 && (
        <Card padded={false}>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="text-xs uppercase tracking-wide text-slate-500">
                <tr className="border-b border-slate-700">
                  <th className="px-4 py-3 text-left">Dataset</th>
                  <th className="px-4 py-3 text-right">Mean score</th>
                  <th className="px-4 py-3 text-right">Completion</th>
                  <th className="px-4 py-3 text-right">Priority Σ</th>
                  <th className="px-4 py-3 text-right">Utilization</th>
                  <th className="px-4 py-3 text-right">Fragmentation</th>
                  <th className="px-4 py-3 text-right">N</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-700">
                {(data?.entries ?? []).map((e) => (
                  <tr key={e.key} className="text-slate-200">
                    <td className="px-4 py-3 font-medium text-white">{e.key}</td>
                    <td className="px-4 py-3 text-right">
                      <Bar value={e.mean_score} max={max.score} format={(v) => fmtNumber(v)} tone="indigo" />
                    </td>
                    <td className="px-4 py-3 text-right tabular-nums">{fmtPercent(e.mean_completion)}</td>
                    <td className="px-4 py-3 text-right">
                      <Bar value={e.mean_priority_sum} max={max.priority} format={(v) => fmtNumber(v)} tone="emerald" />
                    </td>
                    <td className="px-4 py-3 text-right tabular-nums">{fmtPercent(e.mean_utilization)}</td>
                    <td className="px-4 py-3 text-right">
                      <Bar value={e.mean_fragmentation_index} max={max.frag} format={(v) => fmtNumber(v)} tone="amber" />
                    </td>
                    <td className="px-4 py-3 text-right tabular-nums text-slate-400">{e.n}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
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
    tone === 'indigo' ? 'bg-indigo-500/70' : tone === 'emerald' ? 'bg-emerald-500/70' : 'bg-amber-500/70';
  return (
    <div className="ml-auto flex w-32 items-center gap-2">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-slate-700">
        <div className={`h-full ${colour}`} style={{ width: `${pct}%` }} />
      </div>
      <span className="w-12 text-right tabular-nums text-slate-200">{format(value)}</span>
    </div>
  );
}
