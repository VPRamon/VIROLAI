/**
 * Comparison tab — full metrics table for all loaded manifests/schedules,
 * plus upload panels for both manifest JSONs and schedule JSONs.
 */
import { useRef, useState } from 'react';
import { useParams } from 'react-router-dom';
import {
  WorkspacesApiError,
  addManifest,
  addManifestBatch,
  ingestSchedule,
  removeManifest,
} from '../../../lib/workspaces/api';
import { useWorkspaceContext } from '../WorkspaceDetailPage';
import {
  Button,
  Card,
  EmptyState,
  fmtNumber,
  fmtPercent,
} from '../../experiments/_ui';

export default function ComparisonTab() {
  const { id = '' } = useParams();
  const { summaries, reload } = useWorkspaceContext();
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);
  const manifestRef = useRef<HTMLInputElement>(null);
  const scheduleRef = useRef<HTMLInputElement>(null);

  async function onManifestUpload(files: FileList | null) {
    if (!files || files.length === 0) return;
    setUploadError(null);
    setUploading(true);
    try {
      if (files.length === 1) {
        const text = await files[0].text();
        await addManifest(id, JSON.parse(text));
      } else {
        const items: { manifest: unknown }[] = [];
        for (const f of Array.from(files)) {
          items.push({ manifest: JSON.parse(await f.text()) });
        }
        await addManifestBatch(id, items);
      }
      if (manifestRef.current) manifestRef.current.value = '';
      reload();
    } catch (e) {
      setUploadError(e instanceof WorkspacesApiError ? e.message : String(e));
    } finally {
      setUploading(false);
    }
  }

  async function onScheduleUpload(files: FileList | null) {
    if (!files || files.length === 0) return;
    setUploadError(null);
    setUploading(true);
    try {
      for (const f of Array.from(files)) {
        const text = await f.text();
        await ingestSchedule(id, JSON.parse(text));
      }
      if (scheduleRef.current) scheduleRef.current.value = '';
      reload();
    } catch (e) {
      setUploadError(e instanceof WorkspacesApiError ? e.message : String(e));
    } finally {
      setUploading(false);
    }
  }

  async function onRemove(mid: string, name: string) {
    if (!confirm(`Remove "${name}" from this workspace?`)) return;
    try {
      await removeManifest(id, mid);
      reload();
    } catch (e) {
      setUploadError(e instanceof WorkspacesApiError ? e.message : String(e));
    }
  }

  return (
    <div className="space-y-6">
      {/* Upload panel */}
      <Card padded>
        <h3 className="mb-4 text-sm font-semibold uppercase tracking-wide text-slate-300">
          Add data
        </h3>
        <div className="grid grid-cols-1 gap-6 md:grid-cols-2">
          {/* Manifest upload */}
          <div>
            <div className="mb-1.5 text-sm font-medium text-slate-200">
              Upload manifests
            </div>
            <p className="mb-3 text-xs text-slate-500">
              JSON files produced by <code className="text-slate-300">phd manifest create</code>.
              Validates against the manifest schema.
            </p>
            <input
              ref={manifestRef}
              type="file"
              multiple
              accept="application/json,.json"
              disabled={uploading}
              onChange={(e) => onManifestUpload(e.target.files)}
              className="block w-full text-sm text-slate-300 file:mr-3 file:rounded-lg file:border file:border-slate-600 file:bg-slate-700 file:px-3 file:py-1.5 file:text-xs file:font-medium file:text-slate-200 hover:file:bg-slate-600 disabled:opacity-50"
            />
          </div>

          {/* Schedule upload */}
          <div>
            <div className="mb-1.5 text-sm font-medium text-slate-200">
              Upload schedules
            </div>
            <p className="mb-3 text-xs text-slate-500">
              Full schedule JSON files (with embedded{' '}
              <code className="text-slate-300">schedule_metrics</code>).
              A manifest is auto-built and stored — only metrics are kept.
            </p>
            <input
              ref={scheduleRef}
              type="file"
              multiple
              accept="application/json,.json"
              disabled={uploading}
              onChange={(e) => onScheduleUpload(e.target.files)}
              className="block w-full text-sm text-slate-300 file:mr-3 file:rounded-lg file:border file:border-slate-600 file:bg-slate-700 file:px-3 file:py-1.5 file:text-xs file:font-medium file:text-slate-200 hover:file:bg-slate-600 disabled:opacity-50"
            />
          </div>
        </div>

        {uploading && (
          <p className="mt-3 text-xs text-amber-300">Uploading…</p>
        )}
        {uploadError && (
          <p className="mt-3 text-xs text-rose-400">{uploadError}</p>
        )}

        <p className="mt-4 text-xs text-slate-500">
          Or publish from the terminal:{' '}
          <code className="text-slate-300">
            phd publish --workspace {id} --manifest-dir out/&lt;run&gt;
          </code>
        </p>
      </Card>

      {/* Comparison table */}
      {summaries.length === 0 ? (
        <EmptyState
          title="No manifests yet"
          description="Upload manifest or schedule JSON files above."
        />
      ) : (
        <Card padded={false}>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="text-xs uppercase tracking-wide text-slate-500">
                <tr className="border-b border-slate-700">
                  <th className="px-4 py-3 text-left">Manifest</th>
                  <th className="px-4 py-3 text-left">Algorithm</th>
                  <th className="px-4 py-3 text-left">Dataset</th>
                  <th className="px-4 py-3 text-right">Scheduled / Total</th>
                  <th className="px-4 py-3 text-right">Completion</th>
                  <th className="px-4 py-3 text-right">Utilization</th>
                  <th className="px-4 py-3 text-right">Priority Σ</th>
                  <th className="px-4 py-3 text-right">Fragmentation</th>
                  <th className="px-4 py-3 text-right">Composite</th>
                  <th className="px-4 py-3 text-right">Stairs</th>
                  <th className="px-4 py-3 text-right">Valid</th>
                  <th className="px-4 py-3 text-right">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-700">
                {summaries.map((s) => (
                  <tr key={s.manifest_id} className="text-slate-200 hover:bg-slate-800/50">
                    <td className="px-4 py-3">
                      <div className="font-medium text-white">{s.display_name}</div>
                      <div className="mt-0.5 text-[10px] text-slate-500">{s.manifest_id}</div>
                    </td>
                    <td className="px-4 py-3 text-slate-300">{s.algorithm_id}</td>
                    <td className="px-4 py-3 text-slate-300">{s.dataset_id}</td>
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
                            className="text-indigo-400 hover:text-indigo-300 text-xs"
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
      )}
    </div>
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
