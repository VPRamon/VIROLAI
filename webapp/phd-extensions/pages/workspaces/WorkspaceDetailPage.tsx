/**
 * `/workspaces/:id` — workspace detail with manifest table, comparison
 * summary, scheduled-priority-stair ribbons, and an "Open in TSI" link
 * for any manifest that carries a TSI schedule id.
 *
 * **Never fetches `/v1/schedules/...`** — that's the whole point.
 */
import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import {
  WorkspacesApiError,
  addManifest,
  addManifestBatch,
  getComparison,
  getManifest,
  getWorkspace,
  removeManifest,
} from '../../lib/workspaces/api';
import type {
  ComparisonResponse,
  ManifestEntry,
  ManifestSummary,
  PriorityStair,
  ScheduledPriorityStair,
  WorkspaceRecord,
} from '../../lib/workspaces/types';

export default function WorkspaceDetailPage() {
  const { id = '' } = useParams();
  const [ws, setWs] = useState<WorkspaceRecord | null>(null);
  const [manifests, setManifests] = useState<ManifestEntry[]>([]);
  const [comparison, setComparison] = useState<ComparisonResponse | null>(null);
  const [stairs, setStairs] = useState<Record<string, ScheduledPriorityStair | null>>({});
  const [error, setError] = useState<string | null>(null);

  async function reload() {
    setError(null);
    try {
      const detail = await getWorkspace(id);
      setWs(detail.workspace);
      setManifests(detail.manifests);
      const cmp = await getComparison(id);
      setComparison(cmp);
      // Lazily fetch stair payloads for each manifest (one-shot).
      const out: Record<string, ScheduledPriorityStair | null> = {};
      await Promise.all(
        detail.manifests.map(async (m) => {
          try {
            const r = await getManifest(id, m.manifest_id);
            out[m.manifest_id] = r.manifest.body.metrics?.scheduled_priority_stair ?? null;
          } catch {
            out[m.manifest_id] = null;
          }
        }),
      );
      setStairs(out);
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    }
  }
  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  async function onUpload(files: FileList | null) {
    if (!files || files.length === 0) return;
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
      void reload();
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    }
  }

  async function onRemove(mid: string) {
    if (!confirm(`Remove manifest "${mid}" from this workspace?`)) return;
    try {
      await removeManifest(id, mid);
      void reload();
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    }
  }

  if (error) {
    return (
      <div style={{ padding: '1.5rem' }}>
        <Link to="/workspaces">← Back</Link>
        <div style={{ background: '#fee', color: '#900', padding: 8, marginTop: 8 }}>
          {error}
        </div>
      </div>
    );
  }
  if (!ws) return <div style={{ padding: '1.5rem' }}>Loading…</div>;

  const summaryByMid: Record<string, ManifestSummary | undefined> = {};
  comparison?.summaries.forEach((s) => {
    summaryByMid[s.manifest_id] = s;
  });

  return (
    <div style={{ padding: '1.5rem', maxWidth: 1200, margin: '0 auto' }}>
      <Link to="/workspaces">← Workspaces</Link>
      <h1 style={{ marginTop: 4 }}>{ws.name}</h1>
      <div style={{ color: '#666' }}>
        <code>{ws.id}</code> · {ws.status} · {ws.manifest_count} manifest(s)
      </div>

      <section style={{ marginTop: 16 }}>
        <h2>Add manifests</h2>
        <input
          type="file"
          multiple
          accept="application/json,.json"
          onChange={(e) => onUpload(e.target.files)}
        />
        <p style={{ color: '#666', marginTop: 4 }}>
          Or publish from the terminal:{' '}
          <code>phd publish --workspace {ws.id} --manifest-dir &lt;dir&gt;</code>
        </p>
      </section>

      <section style={{ marginTop: 24 }}>
        <h2>Comparison ({manifests.length})</h2>
        {manifests.length === 0 && (
          <p style={{ color: '#666' }}>Empty — add manifests above.</p>
        )}
        {manifests.length > 0 && (
          <>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ background: '#f0f0f0' }}>
                  <th style={th}>Manifest</th>
                  <th style={th}>Algorithm</th>
                  <th style={th}>Dataset</th>
                  <th style={th}>Scheduled / Total</th>
                  <th style={th}>Completion</th>
                  <th style={th}>Utilization</th>
                  <th style={th}>Composite</th>
                  <th style={th}>Stairs</th>
                  <th style={th}>Actions</th>
                </tr>
              </thead>
              <tbody>
                {manifests.map((m) => {
                  const s = summaryByMid[m.manifest_id];
                  return (
                    <tr key={m.manifest_id} style={{ borderTop: '1px solid #eee' }}>
                      <td style={td}>
                        <div>{m.display_name}</div>
                        <div style={{ color: '#888', fontSize: 11 }}>{m.manifest_id}</div>
                      </td>
                      <td style={td}>{m.algorithm_id}</td>
                      <td style={td}>{m.dataset_id}</td>
                      <td style={td}>
                        {s?.scheduled_task_count ?? '—'} / {s?.total_task_count ?? '—'}
                      </td>
                      <td style={td}>{fmt(s?.completion_ratio)}</td>
                      <td style={td}>{fmt(s?.utilization)}</td>
                      <td style={td}>{fmt(s?.composite_rank_score)}</td>
                      <td style={td}>{s?.stair_block_count ?? 0}</td>
                      <td style={td}>
                        {s?.tsi_schedule_id && (
                          <a
                            href={`/schedules/${s.tsi_schedule_id}`}
                            style={{ marginRight: 8 }}
                          >
                            Open in TSI
                          </a>
                        )}
                        <button onClick={() => onRemove(m.manifest_id)}>Remove</button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>

            <h3 style={{ marginTop: 24 }}>Scheduled priority stair</h3>
            <p style={{ color: '#666' }}>
              Each ribbon is one manifest; segments are coloured by priority and
              proportional to count. Computed from the manifest body — no
              schedule loading.
            </p>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {manifests.map((m) => (
                <StairRow
                  key={m.manifest_id}
                  label={m.display_name}
                  stair={stairs[m.manifest_id] ?? null}
                />
              ))}
            </div>
          </>
        )}
      </section>
    </div>
  );
}

function StairRow({
  label,
  stair,
}: {
  label: string;
  stair: ScheduledPriorityStair | null;
}) {
  if (!stair || stair.total_scheduled_items === 0) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <div style={{ width: 200, color: '#444' }}>{label}</div>
        <div style={{ color: '#999' }}>(no scheduled items)</div>
      </div>
    );
  }
  const total = stair.total_scheduled_items;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
      <div style={{ width: 200, color: '#444', fontSize: 13 }}>{label}</div>
      <div
        style={{
          flex: 1,
          display: 'flex',
          height: 20,
          border: '1px solid #ddd',
          background: '#fafafa',
        }}
      >
        {stair.stairs.map((s, i) => (
          <div
            key={i}
            title={`priority ${s.priority} · count ${s.count} (${((s.count / total) * 100).toFixed(1)}%)`}
            style={{
              width: `${(s.count / total) * 100}%`,
              background: priorityColor(s.priority),
              borderRight: '1px solid white',
            }}
          />
        ))}
      </div>
      <div style={{ width: 60, textAlign: 'right', fontSize: 12, color: '#666' }}>
        {stair.stairs.length} blk
      </div>
    </div>
  );
}

function priorityColor(p: number): string {
  // Stable HSL palette keyed by priority value.
  const h = (p * 47) % 360;
  return `hsl(${h}, 65%, 55%)`;
}

function fmt(v: number | null | undefined): string {
  if (v === null || v === undefined) return '—';
  if (Math.abs(v) < 0.01) return v.toFixed(4);
  return v.toFixed(3);
}

const th: React.CSSProperties = {
  textAlign: 'left',
  padding: 6,
  fontWeight: 600,
  fontSize: 13,
};
const td: React.CSSProperties = { padding: 6, fontSize: 13 };

// `PriorityStair` re-exported only to keep the import alive in the type-only path.
export type _RowMarker = PriorityStair;
