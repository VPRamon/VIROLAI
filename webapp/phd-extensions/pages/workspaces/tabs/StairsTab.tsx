/**
 * Stairs tab — priority stair ribbon for every manifest.
 * Full manifest bodies are fetched lazily (only the stair metric is used).
 */
import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import { getFullManifestBody } from '../../../lib/workspaces/api';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import type { ScheduledPriorityStair } from '../../../lib/workspaces/types';
import { Card, EmptyState, Skeleton } from '../../experiments/_ui';

export default function StairsTab() {
  const { id = '' } = useParams();
  const { summaries, loading } = useWorkspaceContext();
  const [stairs, setStairs] = useState<Record<string, ScheduledPriorityStair | null>>({});
  const [fetching, setFetching] = useState(false);

  useEffect(() => {
    if (summaries.length === 0) return;
    setFetching(true);
    Promise.all(
      summaries.map(async (s) => {
        try {
          const r = await getFullManifestBody(id, s.manifest_id);
          return {
            id: s.manifest_id,
            stair: r.manifest.metrics.scheduled_priority_stair ?? null,
          };
        } catch {
          return { id: s.manifest_id, stair: null };
        }
      }),
    ).then((results) => {
      const out: Record<string, ScheduledPriorityStair | null> = {};
      for (const r of results) out[r.id] = r.stair;
      setStairs(out);
      setFetching(false);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, summaries.length]);

  if ((loading || fetching) && summaries.length === 0) {
    return (
      <div className="space-y-3">
        {Array.from({ length: 4 }).map((_, i) => (
          <Skeleton key={i} className="h-8 w-full" />
        ))}
      </div>
    );
  }

  if (summaries.length === 0) {
    return (
      <EmptyState
        title="No manifests yet"
        description="Upload manifests to see their priority stair distribution."
      />
    );
  }

  return (
    <div className="space-y-4">
      <Card padded>
        <p className="text-sm text-slate-400">
          Each ribbon represents one manifest. Segments are coloured by priority value and
          proportional to the scheduled-item count at that priority level. Computed from the
          manifest body — no schedule loading required.
        </p>
      </Card>

      <Card padded>
        {fetching && summaries.length > 0 && (
          <p className="mb-4 text-xs text-amber-300">Loading stair data…</p>
        )}
        <div className="flex flex-col gap-4">
          {summaries.map((s) => (
            <StairRow
              key={s.manifest_id}
              label={s.display_name}
              sublabel={`${s.algorithm_id} · ${s.dataset_id}`}
              stair={stairs[s.manifest_id] ?? null}
              ready={s.manifest_id in stairs}
            />
          ))}
        </div>
      </Card>
    </div>
  );
}

function StairRow({
  label,
  sublabel,
  stair,
  ready,
}: {
  label: string;
  sublabel: string;
  stair: ScheduledPriorityStair | null;
  ready: boolean;
}) {
  return (
    <div className="flex items-center gap-3">
      <div className="w-48 shrink-0">
        <div className="truncate text-sm font-medium text-slate-200">{label}</div>
        <div className="mt-0.5 truncate text-[10px] text-slate-500">{sublabel}</div>
      </div>

      {!ready && <Skeleton className="h-5 flex-1" />}

      {ready && (!stair || stair.total_scheduled_items === 0) && (
        <div className="flex-1 text-xs text-slate-500 italic">no scheduled items</div>
      )}

      {ready && stair && stair.total_scheduled_items > 0 && (
        <>
          <div className="flex h-5 flex-1 overflow-hidden rounded border border-slate-700 bg-slate-900/40">
            {stair.stairs.map((seg, i) => (
              <div
                key={i}
                title={`priority ${seg.priority} · count ${seg.count} (${((seg.count / stair.total_scheduled_items) * 100).toFixed(1)}%)`}
                style={{
                  width: `${(seg.count / stair.total_scheduled_items) * 100}%`,
                  background: priorityColor(seg.priority),
                  borderRight: '1px solid rgba(0,0,0,0.15)',
                }}
              />
            ))}
          </div>
          <div className="w-14 shrink-0 text-right text-xs tabular-nums text-slate-400">
            {stair.stairs.length} blk
          </div>
        </>
      )}
    </div>
  );
}

function priorityColor(p: number): string {
  const h = (p * 47) % 360;
  return `hsl(${h}, 65%, 55%)`;
}
