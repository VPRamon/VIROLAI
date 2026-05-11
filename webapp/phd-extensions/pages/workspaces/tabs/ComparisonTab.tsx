/**
 * Comparison tab — full sortable metrics table for all manifests in this workspace.
 * Uploads are handled by the persistent panel in WorkspaceDetailPage.
 */
import { useState } from 'react';
import { useParams } from 'react-router-dom';
import { WorkspacesApiError, removeManifest } from '../../../lib/workspaces/api';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import {
  Button,
  Card,
  EmptyState,
  fmtNumber,
  fmtPercent,
} from '../../experiments/_ui';

type SortKey =
  | 'display_name'
  | 'completion_ratio'
  | 'utilization'
  | 'priority_sum'
  | 'fragmentation_index'
  | 'composite_rank_score';

export default function ComparisonTab() {
  const { id = '' } = useParams();
  const { summaries, reload } = useWorkspaceContext();
  const [removeError, setRemoveError] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>('composite_rank_score');
  const [sortAsc, setSortAsc] = useState(false);

  function toggleSort(key: SortKey) {
    if (sortKey === key) setSortAsc((a) => !a);
    else { setSortKey(key); setSortAsc(false); }
  }

  const sorted = [...summaries].sort((a, b) => {
    const av = a[sortKey] ?? '';
    const bv = b[sortKey] ?? '';
    const cmp = av < bv ? -1 : av > bv ? 1 : 0;
    return sortAsc ? cmp : -cmp;
  });

  async function onRemove(mid: string, name: string) {
    if (!confirm(`Remove "${name}" from this workspace?`)) return;
    try {
      await removeManifest(id, mid);
      reload();
    } catch (e) {
      setRemoveError(e instanceof WorkspacesApiError ? e.message : String(e));
    }
  }

  if (summaries.length === 0) {
    return (
      <EmptyState
        title="No manifests yet"
        description='Use the "Add data" panel above to upload manifest or schedule JSON files.'
      />
    );
  }

  return (
    <div className="space-y-4">
      {removeError && (
        <p className="rounded-lg bg-rose-950/50 px-4 py-2 text-xs text-rose-400">{removeError}</p>
      )}

      <Card padded={false}>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead className="text-xs uppercase tracking-wide text-slate-500">
              <tr className="border-b border-slate-700">
                <SortTh label="Manifest" col="display_name" active={sortKey} asc={sortAsc} onSort={toggleSort} align="left" />
                <th className="px-4 py-3 text-left">Algorithm</th>
                <th className="px-4 py-3 text-left">Dataset</th>
                <th className="px-4 py-3 text-right">Scheduled / Total</th>
                <SortTh label="Completion" col="completion_ratio" active={sortKey} asc={sortAsc} onSort={toggleSort} align="right" />
                <SortTh label="Utilization" col="utilization" active={sortKey} asc={sortAsc} onSort={toggleSort} align="right" />
                <SortTh label="Priority Σ" col="priority_sum" active={sortKey} asc={sortAsc} onSort={toggleSort} align="right" />
                <SortTh label="Frag." col="fragmentation_index" active={sortKey} asc={sortAsc} onSort={toggleSort} align="right" />
                <SortTh label="Composite" col="composite_rank_score" active={sortKey} asc={sortAsc} onSort={toggleSort} align="right" />
                <th className="px-4 py-3 text-right">Stairs</th>
                <th className="px-4 py-3 text-right">Valid</th>
                <th className="px-4 py-3 text-right">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-700">
              {sorted.map((s) => (
                <tr key={s.manifest_id} className="text-slate-200 hover:bg-slate-800/50">
                  <td className="px-4 py-3">
                    <div className="font-medium text-white">{s.display_name}</div>
                    <div className="mt-0.5 text-[10px] text-slate-500">{s.manifest_id}</div>
                  </td>
                  <td className="px-4 py-3 text-slate-300">{s.algorithm_id ?? '—'}</td>
                  <td className="px-4 py-3 text-slate-300">{s.dataset_id ?? '—'}</td>
                  <td className="px-4 py-3 text-right tabular-nums">
                    {s.scheduled_task_count ?? '—'} / {s.total_task_count ?? '—'}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums">
                    {fmtPercent(s.completion_ratio)}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums">
                    {fmtPercent(s.utilization)}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums">
                    {fmtNumber(s.priority_sum)}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums">
                    {fmtNumber(s.fragmentation_index)}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums font-medium text-emerald-300">
                    {fmtNumber(s.composite_rank_score)}
                  </td>
                  <td className="px-4 py-3 text-right tabular-nums text-slate-400">
                    {s.stair_block_count}
                  </td>
                  <td className="px-4 py-3 text-right">
                    <ValidationBadge status={s.validation_status} />
                  </td>
                  <td className="px-4 py-3 text-right">
                    <div className="flex items-center justify-end gap-2">
                      {s.tsi_schedule_id && (
                        <a
                          href={`/schedules/${s.tsi_schedule_id}`}
                          className="text-xs text-indigo-400 hover:text-indigo-300"
                        >
                          TSI
                        </a>
                      )}
                      <Button
                        variant="ghost"
                        className="!px-2 !py-1 text-xs text-rose-400 hover:text-rose-300"
                        onClick={() => onRemove(s.manifest_id, s.display_name)}
                      >
                        Remove
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>
    </div>
  );
}

function SortTh({
  label,
  col,
  active,
  asc,
  onSort,
  align,
}: {
  label: string;
  col: SortKey;
  active: SortKey;
  asc: boolean;
  onSort: (k: SortKey) => void;
  align: 'left' | 'right';
}) {
  const isActive = active === col;
  return (
    <th
      className={`cursor-pointer select-none px-4 py-3 text-${align} hover:text-slate-300 ${isActive ? 'text-slate-200' : ''}`}
      onClick={() => onSort(col)}
    >
      {label}
      {isActive && <span className="ml-1">{asc ? '↑' : '↓'}</span>}
    </th>
  );
}

function ValidationBadge({ status }: { status: string | null }) {
  if (!status) return <span className="text-slate-500">—</span>;
  const cls =
    status === 'valid'
      ? 'text-emerald-300'
      : status === 'warning'
        ? 'text-amber-300'
        : 'text-rose-400';
  return <span className={`text-xs font-medium ${cls}`}>{status}</span>;
}
