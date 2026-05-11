/**
 * `/workspaces` — list, create, and navigate to workspace detail.
 *
 * Uses the same dark Tailwind design system as the Experiments section.
 */
import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import {
  WorkspacesApiError,
  createWorkspace,
  deleteWorkspace,
  listWorkspaces,
} from '../../lib/workspaces/api';
import type { WorkspaceRecord } from '../../lib/workspaces/types';
import {
  Button,
  Card,
  EmptyState,
  ErrorState,
  SectionHeader,
  Select,
  Skeleton,
  TextField,
  fmtDate,
} from '../experiments/_ui';

type SortKey = 'updated_at' | 'created_at' | 'name';

export default function WorkspacesListPage() {
  const navigate = useNavigate();
  const [items, setItems] = useState<WorkspaceRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<SortKey>('updated_at');

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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const filtered = useMemo<WorkspaceRecord[]>(() => {
    const all = items ?? [];
    const needle = query.trim().toLowerCase();
    let rows = needle
      ? all.filter(
          (w) =>
            w.name.toLowerCase().includes(needle) || w.id.toLowerCase().includes(needle),
        )
      : all;
    rows = [...rows].sort((a, b) => {
      if (sort === 'name') return a.name.localeCompare(b.name);
      const av = sort === 'updated_at' ? a.updated_at : a.created_at;
      const bv = sort === 'updated_at' ? b.updated_at : b.created_at;
      return bv.localeCompare(av);
    });
    return rows;
  }, [items, query, sort]);

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

  async function onDelete(id: string, wsName: string) {
    if (!confirm(`Delete workspace "${wsName}"? This cannot be undone.`)) return;
    try {
      await deleteWorkspace(id);
      void reload();
    } catch (err) {
      setError(err instanceof WorkspacesApiError ? err.message : String(err));
    }
  }

  return (
    <div className="mx-auto max-w-7xl px-6 py-8">
      <SectionHeader
        title="Workspaces"
        subtitle="Compare manifests and schedules side-by-side. Upload JSON files or publish from the CLI."
      />

      {/* Create form */}
      <Card className="mb-6" padded>
        <form onSubmit={onCreate} className="flex flex-wrap items-end gap-3">
          <div className="flex-1 min-w-48">
            <TextField
              label="New workspace name"
              placeholder="e.g. paper-comparisons"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <Button type="submit" variant="primary" disabled={busy || !name.trim()}>
            {busy ? 'Creating…' : 'Create'}
          </Button>
        </form>
        <p className="mt-3 text-xs text-slate-500">
          Or publish from the terminal:{' '}
          <code className="text-slate-300">
            phd publish --workspace &lt;id&gt; --manifest-dir out/&lt;run&gt;
          </code>
        </p>
      </Card>

      {error && <ErrorState error={error} onRetry={reload} />}

      {/* Filters */}
      {items !== null && items.length > 0 && (
        <Card className="mb-6" padded>
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <TextField
              label="Search"
              placeholder="Filter by name or id…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <Select
              label="Sort by"
              value={sort}
              onChange={(v) => setSort(v)}
              options={[
                { value: 'updated_at', label: 'Recently updated' },
                { value: 'created_at', label: 'Recently created' },
                { value: 'name', label: 'Name (A→Z)' },
              ]}
            />
          </div>
        </Card>
      )}

      {items === null && !error && <ListSkeleton />}

      {items !== null && filtered.length === 0 && (
        <EmptyState
          title={
            items.length === 0 ? 'No workspaces yet' : 'No workspaces match your search'
          }
          description={
            items.length === 0
              ? 'Create a workspace above or publish one from the CLI.'
              : 'Try clearing the search filter.'
          }
        />
      )}

      {items !== null && filtered.length > 0 && (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {filtered.map((w) => (
            <WorkspaceCard key={w.id} ws={w} onDelete={onDelete} />
          ))}
        </div>
      )}
    </div>
  );
}

function WorkspaceCard({
  ws,
  onDelete,
}: {
  ws: WorkspaceRecord;
  onDelete: (id: string, name: string) => void;
}) {
  return (
    <Card>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <Link
            to={`/workspaces/${ws.id}`}
            className="truncate text-base font-semibold text-white hover:text-indigo-300 transition-colors"
          >
            {ws.name}
          </Link>
          <div className="mt-0.5 truncate text-xs text-slate-400">{ws.id}</div>
        </div>
        <span
          className={`shrink-0 rounded-full border px-2 py-0.5 text-xs font-medium ${
            ws.status === 'active'
              ? 'border-emerald-500/40 bg-emerald-500/15 text-emerald-300'
              : 'border-slate-600 bg-slate-700/60 text-slate-400'
          }`}
        >
          {ws.status}
        </span>
      </div>

      <div className="mt-4 grid grid-cols-2 gap-2 text-center">
        <div className="rounded-md border border-slate-700 bg-slate-900/40 px-2 py-1.5">
          <div className="text-[10px] uppercase tracking-wide text-slate-500">Manifests</div>
          <div className="text-base font-semibold tabular-nums text-slate-200">
            {ws.manifest_count}
          </div>
        </div>
        <div className="rounded-md border border-slate-700 bg-slate-900/40 px-2 py-1.5">
          <div className="text-[10px] uppercase tracking-wide text-slate-500">Updated</div>
          <div className="text-sm font-medium text-slate-300">{fmtDate(ws.updated_at)}</div>
        </div>
      </div>

      <div className="mt-4 flex items-center justify-between gap-2">
        <Link to={`/workspaces/${ws.id}`}>
          <Button variant="secondary">
            Open
          </Button>
        </Link>
        <Button
          variant="ghost"
          onClick={() => onDelete(ws.id, ws.name)}
        >
          Delete
        </Button>
      </div>
    </Card>
  );
}

function ListSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <Card key={i}>
          <Skeleton className="h-5 w-2/3" />
          <Skeleton className="mt-2 h-3 w-1/3" />
          <div className="mt-4 grid grid-cols-2 gap-2">
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
          </div>
        </Card>
      ))}
    </div>
  );
}

