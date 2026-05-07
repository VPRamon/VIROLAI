/**
 * `/workspaces` — list, create, and navigate to workspace detail.
 *
 * Manifest-first: this page never fetches schedule artifacts.
 */
import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import {
  WorkspacesApiError,
  createWorkspace,
  deleteWorkspace,
  listWorkspaces,
} from '../../lib/workspaces/api';
import type { WorkspaceRecord } from '../../lib/workspaces/types';

export default function WorkspacesListPage() {
  const navigate = useNavigate();
  const [items, setItems] = useState<WorkspaceRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);

  async function reload() {
    setError(null);
    try {
      const r = await listWorkspaces();
      setItems(r.workspaces);
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    }
  }
  useEffect(() => {
    void reload();
  }, []);

  async function onCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    try {
      const { workspace } = await createWorkspace({ name: name.trim() });
      setName('');
      navigate(`/workspaces/${workspace.id}`);
    } catch (err) {
      setError(err instanceof WorkspacesApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onDelete(id: string) {
    if (!confirm(`Delete workspace "${id}"? This cannot be undone.`)) return;
    try {
      await deleteWorkspace(id);
      void reload();
    } catch (err) {
      setError(err instanceof WorkspacesApiError ? err.message : String(err));
    }
  }

  return (
    <div style={{ padding: '1.5rem', maxWidth: 960, margin: '0 auto' }}>
      <h1 style={{ marginTop: 0 }}>Workspaces</h1>
      <p style={{ color: '#666' }}>
        Group lightweight CLI manifests (no full schedules loaded by default) for
        side-by-side comparison. Publish from the terminal with{' '}
        <code>phd publish --workspace &lt;id&gt; --manifest &lt;file&gt;</code>.
      </p>

      <form onSubmit={onCreate} style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
        <input
          type="text"
          placeholder="New workspace name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          style={{ flex: 1, padding: 6 }}
        />
        <button type="submit" disabled={busy || !name.trim()}>
          Create
        </button>
      </form>

      {error && (
        <div style={{ background: '#fee', color: '#900', padding: 8, marginBottom: 8 }}>
          {error}
        </div>
      )}

      {items === null && <p>Loading…</p>}
      {items !== null && items.length === 0 && (
        <div
          style={{
            border: '1px dashed #ccc',
            padding: 24,
            textAlign: 'center',
            color: '#777',
          }}
        >
          No workspaces yet. Create one above, or run a CLI publish:
          <pre style={{ background: '#f6f6f6', padding: 8, textAlign: 'left' }}>
{`phd matrix --spec experiments/ctao_n_est.json
phd publish --workspace paper-comparisons --create-workspace \\
            --manifest-dir out/<run-dir>`}
          </pre>
        </div>
      )}
      {items !== null && items.length > 0 && (
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr style={{ background: '#f0f0f0' }}>
              <th style={th}>Name</th>
              <th style={th}>Status</th>
              <th style={th}>Manifests</th>
              <th style={th}>Updated</th>
              <th style={th}></th>
            </tr>
          </thead>
          <tbody>
            {items.map((w) => (
              <tr key={w.id} style={{ borderTop: '1px solid #eee' }}>
                <td style={td}>
                  <Link to={`/workspaces/${w.id}`}>{w.name}</Link>
                  <div style={{ color: '#888', fontSize: 12 }}>{w.id}</div>
                </td>
                <td style={td}>{w.status}</td>
                <td style={td}>{w.manifest_count}</td>
                <td style={td}>{new Date(w.updated_at).toLocaleString()}</td>
                <td style={td}>
                  <button onClick={() => onDelete(w.id)}>Delete</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

const th: React.CSSProperties = { textAlign: 'left', padding: 8, fontWeight: 600 };
const td: React.CSSProperties = { padding: 8 };
