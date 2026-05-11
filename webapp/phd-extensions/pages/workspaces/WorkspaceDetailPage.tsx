/**
 * `/workspaces/:id` — workspace detail shell with tab bar.
 *
 * Loads workspace record + comparison summaries once and exposes them
 * via `WorkspaceCtx` to every child tab. Each tab is code-split with
 * React.lazy so heavy Plotly panels don't block the initial render.
 *
 * Tabs: Overview · Comparison · Pareto · Per-dataset · Per-algorithm · Stairs
 */
import { Suspense, createContext, lazy, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { Link, NavLink, Route, Routes, useParams } from 'react-router-dom';
import {
  WorkspacesApiError,
  getComparison,
  getScheduleForManifest,
  getWorkspace,
} from '../../lib/workspaces/api';
import {
  collectFiles,
  uploadFiles,
  type QueueItem,
  type QueueItemStatus,
} from '../../lib/workspaces/uploader';
import type {
  ComparisonResponse,
  ManifestSummary,
  WorkspaceRecord,
} from '../../lib/workspaces/types';
import {
  Button,
  Card,
  ErrorState,
  SectionHeader,
  Skeleton,
} from '../experiments/_ui';

// ── Context ───────────────────────────────────────────────────────────────

export interface WorkspaceContextValue {
  workspace: WorkspaceRecord;
  summaries: ManifestSummary[];
  reload: () => void;
  loading: boolean;
}

const WorkspaceCtx = createContext<WorkspaceContextValue | null>(null);

export function useWorkspaceContext(): WorkspaceContextValue {
  const ctx = useContext(WorkspaceCtx);
  if (!ctx) throw new Error('useWorkspaceContext must be inside WorkspaceDetailPage');
  return ctx;
}

// ── Lazy tabs ─────────────────────────────────────────────────────────────

const OverviewTab = lazy(() => import('./tabs/OverviewTab'));
const ComparisonTab = lazy(() => import('./tabs/ComparisonTab'));
const ParetoTab = lazy(() => import('./tabs/ParetoTab'));
const PerDatasetTab = lazy(() => import('./tabs/PerDatasetTab'));
const PerAlgorithmTab = lazy(() => import('./tabs/PerAlgorithmTab'));
const StairsTab = lazy(() => import('./tabs/StairsTab'));

const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'comparison', label: 'Comparison' },
  { id: 'pareto', label: 'Pareto' },
  { id: 'per-dataset', label: 'Per dataset' },
  { id: 'per-algorithm', label: 'Per algorithm' },
  { id: 'stairs', label: 'Priority stairs' },
] as const;

// ── Main page ─────────────────────────────────────────────────────────────

export default function WorkspaceDetailPage() {
  const { id = '' } = useParams();
  const [workspace, setWorkspace] = useState<WorkspaceRecord | null>(null);
  const [summaries, setSummaries] = useState<ManifestSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = useCallback(async () => {
    setError(null);
    setLoading(true);
    try {
      const [detail, cmp] = await Promise.all([getWorkspace(id), getComparison(id)]);
      setWorkspace(detail.workspace);
      setSummaries((cmp as ComparisonResponse).summaries);
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    void reload();
  }, [reload]);

  if (error && !workspace) {
    return (
      <div className="mx-auto max-w-7xl px-6 py-8">
        <div className="mb-4 text-xs text-slate-500">
          <Link to="/workspaces" className="hover:text-indigo-300">
            Workspaces
          </Link>
        </div>
        <ErrorState error={error} onRetry={reload} />
      </div>
    );
  }

  if (!workspace) {
    return (
      <div className="mx-auto max-w-7xl px-6 py-8">
        <Skeleton className="h-8 w-1/3" />
        <Skeleton className="mt-2 h-4 w-1/4" />
        <Skeleton className="mt-6 h-12 w-full" />
        <Skeleton className="mt-4 h-64 w-full" />
      </div>
    );
  }

  return (
    <WorkspaceCtx.Provider value={{ workspace, summaries, reload, loading }}>
      <div className="mx-auto max-w-7xl px-6 py-8">
        <div className="mb-4 text-xs text-slate-500">
          <Link to="/workspaces" className="hover:text-indigo-300">
            Workspaces
          </Link>
          <span className="mx-1.5">/</span>
          <span className="text-slate-300">{workspace.name}</span>
        </div>

        <SectionHeader
          title={workspace.name}
          subtitle={
            <>
              <code className="text-slate-300">{workspace.id}</code>
              {' · '}
              <span
                className={
                  workspace.status === 'active' ? 'text-emerald-400' : 'text-slate-500'
                }
              >
                {workspace.status}
              </span>
              {' · '}
              {summaries.length} manifest{summaries.length !== 1 ? 's' : ''}
            </>
          }
          actions={
            <Button variant="secondary" onClick={reload}>
              Refresh
            </Button>
          }
        />

        {error && <ErrorState error={error} onRetry={reload} />}

        {/* Stats strip */}
        <Card className="mb-4">
          <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
            <StatCell label="Manifests" value={summaries.length} />
            <StatCell
              label="Avg completion"
              value={fmtPct(avg(summaries, (s) => s.completion_ratio))}
            />
            <StatCell
              label="Avg utilization"
              value={fmtPct(avg(summaries, (s) => s.utilization))}
            />
            <StatCell
              label="Avg composite"
              value={fmtNum(avg(summaries, (s) => s.composite_rank_score))}
            />
          </div>
        </Card>

        {/* Upload panel — always accessible, expanded by default when empty */}
        <UploadPanel
          workspaceId={id}
          isEmpty={summaries.length === 0}
          onUploaded={reload}
        />

        {/* Manifests table — primary entry point for drill-down */}
        {summaries.length > 0 && (
          <ManifestsTable workspaceId={id} summaries={summaries} />
        )}

        <TabBar />

        <div className="mt-4">
          <Routes>
            <Route path="comparison" element={<TabFrame><ComparisonTab /></TabFrame>} />
            <Route path="pareto" element={<TabFrame><ParetoTab /></TabFrame>} />
            <Route path="per-dataset" element={<TabFrame><PerDatasetTab /></TabFrame>} />
            <Route path="per-algorithm" element={<TabFrame><PerAlgorithmTab /></TabFrame>} />
            <Route path="stairs" element={<TabFrame><StairsTab /></TabFrame>} />
            <Route path="overview" element={<TabFrame><OverviewTab /></TabFrame>} />
            <Route path="*" element={<TabFrame><OverviewTab /></TabFrame>} />
          </Routes>
        </div>
      </div>
    </WorkspaceCtx.Provider>
  );
}

// ── Upload panel ──────────────────────────────────────────────────────────

function UploadPanel({
  workspaceId,
  isEmpty,
  onUploaded,
}: {
  workspaceId: string;
  isEmpty: boolean;
  onUploaded: () => void;
}) {
  const [open, setOpen] = useState(isEmpty);
  const [busy, setBusy] = useState(false);
  const [includeSchedules, setIncludeSchedules] = useState(true);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [dragOver, setDragOver] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const folderRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (isEmpty) setOpen(true);
  }, [isEmpty]);

  const totals = useMemo(() => summarizeQueue(queue), [queue]);

  const run = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      setBusy(true);
      try {
        await uploadFiles(files, {
          workspaceId,
          includeSchedules,
          onUpdate: (next) => setQueue([...next]),
        });
      } finally {
        setBusy(false);
        onUploaded();
      }
    },
    [workspaceId, includeSchedules, onUploaded],
  );

  const onPick = useCallback(
    async (input: FileList | null) => {
      if (!input) return;
      const files = await collectFiles(input);
      if (fileRef.current) fileRef.current.value = '';
      if (folderRef.current) folderRef.current.value = '';
      void run(files);
    },
    [run],
  );

  const onDrop = useCallback(
    async (ev: React.DragEvent<HTMLDivElement>) => {
      ev.preventDefault();
      setDragOver(false);
      if (!ev.dataTransfer) return;
      const files = await collectFiles(ev.dataTransfer.items);
      void run(files);
    },
    [run],
  );

  return (
    <div className="mb-4">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center justify-between rounded-lg border border-slate-700 bg-slate-800/60 px-4 py-2.5 text-left text-sm font-medium text-slate-300 hover:bg-slate-700/60 transition-colors"
      >
        <span className="flex items-center gap-2">
          <svg xmlns="http://www.w3.org/2000/svg" className="h-4 w-4 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v1a2 2 0 002 2h12a2 2 0 002-2v-1M12 12V4m0 0l-3 3m3-3l3 3" />
          </svg>
          Upload manifests &amp; schedules
          {queue.length > 0 && (
            <span className="ml-2 rounded-full bg-slate-700 px-2 py-0.5 text-xs">
              {totals.done}/{queue.length}
            </span>
          )}
        </span>
        <svg
          xmlns="http://www.w3.org/2000/svg"
          className={`h-4 w-4 transition-transform ${open ? 'rotate-180' : ''}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {open && (
        <Card className="rounded-t-none border-t-0 !mt-0">
          <div
            onDragOver={(e) => {
              e.preventDefault();
              setDragOver(true);
            }}
            onDragLeave={() => setDragOver(false)}
            onDrop={onDrop}
            className={`rounded-lg border-2 border-dashed px-4 py-8 text-center transition-colors ${
              dragOver
                ? 'border-indigo-400 bg-indigo-500/5'
                : 'border-slate-600 bg-slate-900/40'
            }`}
          >
            <div className="text-sm font-medium text-slate-200">
              Drop a folder or files here
            </div>
            <p className="mt-1 text-xs text-slate-500">
              Manifests (<code>*.manifest.json</code>) and self-contained schedules
              (<code>*.json</code>) are auto-classified by content.
            </p>
            <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
              <input
                ref={fileRef}
                type="file"
                multiple
                accept="application/json,.json"
                disabled={busy}
                onChange={(e) => void onPick(e.target.files)}
                className="hidden"
              />
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() => fileRef.current?.click()}
              >
                Browse files…
              </Button>
              <input
                ref={folderRef}
                type="file"
                /* @ts-expect-error -- non-standard but widely supported */
                webkitdirectory=""
                directory=""
                multiple
                disabled={busy}
                onChange={(e) => void onPick(e.target.files)}
                className="hidden"
              />
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() => folderRef.current?.click()}
              >
                Browse folder…
              </Button>
              <label className="ml-2 inline-flex select-none items-center gap-1.5 text-xs text-slate-400">
                <input
                  type="checkbox"
                  checked={includeSchedules}
                  onChange={(e) => setIncludeSchedules(e.target.checked)}
                  disabled={busy}
                  className="h-3.5 w-3.5 rounded border-slate-600 bg-slate-900 text-indigo-400 focus:ring-indigo-400"
                />
                Persist full schedules
              </label>
            </div>
          </div>

          {queue.length > 0 && (
            <div className="mt-4">
              <div className="mb-2 flex items-center justify-between">
                <div className="text-xs text-slate-400">
                  {totals.created} created · {totals.deduped} duplicate ·{' '}
                  {totals.error} errored · {totals.skipped} skipped
                </div>
                <Button variant="ghost" onClick={() => setQueue([])} disabled={busy}>
                  Clear
                </Button>
              </div>
              <UploadQueueTable items={queue} />
            </div>
          )}

          <p className="mt-4 text-xs text-slate-500">
            Equivalent terminal command:{' '}
            <code className="text-slate-300">
              phd publish --workspace {workspaceId} --dir out/&lt;run&gt; --include-schedules
            </code>
          </p>
        </Card>
      )}
    </div>
  );
}

function UploadQueueTable({ items }: { items: QueueItem[] }) {
  return (
    <div className="max-h-80 overflow-auto rounded-md border border-slate-700">
      <table className="w-full text-xs">
        <thead className="sticky top-0 bg-slate-800/80 text-left text-slate-400">
          <tr>
            <th className="px-2 py-1.5 font-medium">File</th>
            <th className="px-2 py-1.5 font-medium">Kind</th>
            <th className="px-2 py-1.5 font-medium">Status</th>
            <th className="px-2 py-1.5 font-medium">Detail</th>
          </tr>
        </thead>
        <tbody>
          {items.map((it) => (
            <tr key={it.id} className="border-t border-slate-700/60">
              <td className="px-2 py-1 font-mono text-slate-300">{it.path}</td>
              <td className="px-2 py-1 text-slate-400">{it.kind}</td>
              <td className="px-2 py-1"><StatusBadge status={it.status} /></td>
              <td className="px-2 py-1 text-slate-500">
                {it.message ?? (it.manifestId ? `mid: ${it.manifestId.slice(0, 8)}…` : '')}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function StatusBadge({ status }: { status: QueueItemStatus }) {
  const map: Record<QueueItemStatus, [string, string]> = {
    pending: ['bg-slate-700 text-slate-300', '⏳ pending'],
    classifying: ['bg-slate-700 text-slate-300', '🔍 classifying'],
    uploading: ['bg-amber-500/20 text-amber-300', '⏫ uploading'],
    created: ['bg-emerald-500/20 text-emerald-300', '✅ created'],
    deduped: ['bg-sky-500/20 text-sky-300', '♻️ duplicate'],
    error: ['bg-rose-500/20 text-rose-300', '❌ error'],
    skipped: ['bg-slate-700/60 text-slate-400', '⏭ skipped'],
  };
  const [cls, label] = map[status];
  return (
    <span className={`inline-block whitespace-nowrap rounded px-1.5 py-0.5 ${cls}`}>{label}</span>
  );
}

function summarizeQueue(items: QueueItem[]) {
  const acc = { created: 0, deduped: 0, error: 0, skipped: 0, done: 0 };
  for (const it of items) {
    if (it.status === 'created') acc.created++;
    else if (it.status === 'deduped') acc.deduped++;
    else if (it.status === 'error') acc.error++;
    else if (it.status === 'skipped') acc.skipped++;
    if (
      it.status === 'created' ||
      it.status === 'deduped' ||
      it.status === 'error' ||
      it.status === 'skipped'
    ) {
      acc.done++;
    }
  }
  return acc;
}

// ── Manifests table + schedule drill-down ─────────────────────────────────

const ROWS_PER_PAGE = 50;

function ManifestsTable({
  workspaceId,
  summaries,
}: {
  workspaceId: string;
  summaries: ManifestSummary[];
}) {
  const [filter, setFilter] = useState('');
  const [page, setPage] = useState(0);
  const [drill, setDrill] = useState<{ mid: string; name: string } | null>(null);

  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return summaries;
    return summaries.filter(
      (s) =>
        s.display_name.toLowerCase().includes(needle) ||
        s.dataset_id.toLowerCase().includes(needle) ||
        s.algorithm_id.toLowerCase().includes(needle),
    );
  }, [summaries, filter]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / ROWS_PER_PAGE));
  const safePage = Math.min(page, pageCount - 1);
  const slice = filtered.slice(
    safePage * ROWS_PER_PAGE,
    (safePage + 1) * ROWS_PER_PAGE,
  );

  return (
    <Card className="mb-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="text-sm font-medium text-slate-200">
          Manifests <span className="text-slate-500">({filtered.length})</span>
        </div>
        <input
          type="search"
          placeholder="Filter by dataset, algorithm, name…"
          value={filter}
          onChange={(e) => {
            setFilter(e.target.value);
            setPage(0);
          }}
          className="w-72 rounded-md border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-slate-200 placeholder:text-slate-500 focus:border-indigo-400 focus:outline-none"
        />
      </div>
      <div className="overflow-auto rounded-md border border-slate-700">
        <table className="w-full text-xs">
          <thead className="bg-slate-800/80 text-left text-slate-400">
            <tr>
              <th className="px-2 py-1.5 font-medium">Name</th>
              <th className="px-2 py-1.5 font-medium">Dataset</th>
              <th className="px-2 py-1.5 font-medium">Algorithm</th>
              <th className="px-2 py-1.5 text-right font-medium">Completion</th>
              <th className="px-2 py-1.5 text-right font-medium">Utilization</th>
              <th className="px-2 py-1.5 text-right font-medium">Composite</th>
              <th className="px-2 py-1.5 font-medium">Schedule</th>
              <th className="px-2 py-1.5"></th>
            </tr>
          </thead>
          <tbody>
            {slice.map((s) => (
              <tr key={s.manifest_id} className="border-t border-slate-700/60">
                <td className="px-2 py-1 font-mono text-slate-300" title={s.manifest_id}>
                  {s.display_name}
                </td>
                <td className="px-2 py-1 text-slate-400">{s.dataset_id}</td>
                <td className="px-2 py-1 text-slate-400">{s.algorithm_id}</td>
                <td className="px-2 py-1 text-right tabular-nums text-slate-200">
                  {fmtPct(s.completion_ratio)}
                </td>
                <td className="px-2 py-1 text-right tabular-nums text-slate-200">
                  {fmtPct(s.utilization)}
                </td>
                <td className="px-2 py-1 text-right tabular-nums text-slate-200">
                  {fmtNum(s.composite_rank_score)}
                </td>
                <td className="px-2 py-1">
                  {s.has_full_schedule ? (
                    <span className="rounded bg-emerald-500/20 px-1.5 py-0.5 text-emerald-300">
                      stored
                    </span>
                  ) : (
                    <span className="text-slate-500">—</span>
                  )}
                </td>
                <td className="px-2 py-1 text-right">
                  {s.has_full_schedule && (
                    <button
                      onClick={() =>
                        setDrill({ mid: s.manifest_id, name: s.display_name })
                      }
                      className="rounded border border-slate-600 px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-700"
                    >
                      Open
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {pageCount > 1 && (
        <div className="mt-2 flex items-center justify-end gap-2 text-xs text-slate-400">
          <Button
            variant="ghost"
            disabled={safePage === 0}
            onClick={() => setPage((p) => Math.max(0, p - 1))}
          >
            ← Prev
          </Button>
          <span>
            Page {safePage + 1} / {pageCount}
          </span>
          <Button
            variant="ghost"
            disabled={safePage >= pageCount - 1}
            onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
          >
            Next →
          </Button>
        </div>
      )}
      {drill && (
        <ScheduleDrawer
          workspaceId={workspaceId}
          manifestId={drill.mid}
          title={drill.name}
          onClose={() => setDrill(null)}
        />
      )}
    </Card>
  );
}

function ScheduleDrawer({
  workspaceId,
  manifestId,
  title,
  onClose,
}: {
  workspaceId: string;
  manifestId: string;
  title: string;
  onClose: () => void;
}) {
  const [data, setData] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getScheduleForManifest(workspaceId, manifestId)
      .then((r) => {
        if (alive) setData(r.schedule);
      })
      .catch((e: unknown) => {
        if (alive)
          setError(e instanceof WorkspacesApiError ? e.message : String(e));
      });
    return () => {
      alive = false;
    };
  }, [workspaceId, manifestId]);

  return (
    <div className="fixed inset-0 z-50 flex justify-end bg-black/60" onClick={onClose}>
      <div
        className="flex h-full w-full max-w-3xl flex-col bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center justify-between border-b border-slate-700 px-4 py-3">
          <div>
            <div className="text-sm font-medium text-slate-200">Schedule</div>
            <div className="font-mono text-xs text-slate-500">{title}</div>
          </div>
          <Button variant="ghost" onClick={onClose}>Close</Button>
        </header>
        <div className="flex-1 overflow-auto p-4">
          {error && <ErrorState error={error} />}
          {!error && !data && <Skeleton className="h-64 w-full" />}
          {!error && data !== null && (
            <pre className="whitespace-pre-wrap break-all rounded-md bg-slate-950 p-3 text-[11px] leading-snug text-slate-300">
              {JSON.stringify(data, null, 2)}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────

function TabFrame({ children }: { children: React.ReactNode }) {
  return (
    <Suspense
      fallback={
        <Card>
          <Skeleton className="h-6 w-1/3" />
          <Skeleton className="mt-3 h-4 w-2/3" />
          <Skeleton className="mt-6 h-64 w-full" />
        </Card>
      }
    >
      {children}
    </Suspense>
  );
}

function TabBar() {
  return (
    <nav className="flex flex-wrap gap-1 border-b border-slate-700">
      {TABS.map((t) => (
        <NavLink
          key={t.id}
          to={t.id}
          end={false}
          className={({ isActive }) =>
            `relative -mb-px border-b-2 px-3.5 py-2 text-sm font-medium transition-colors ${
              isActive
                ? 'border-indigo-400 text-white'
                : 'border-transparent text-slate-400 hover:text-slate-200'
            }`
          }
        >
          {t.label}
        </NavLink>
      ))}
    </nav>
  );
}

function StatCell({ label, value }: { label: string; value: string | number }) {
  return (
    <div>
      <div className="text-xs uppercase tracking-wide text-slate-400">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-white">{value}</div>
    </div>
  );
}

function avg(
  rows: ManifestSummary[],
  fn: (s: ManifestSummary) => number | null,
): number | null {
  const vals = rows.map(fn).filter((v): v is number => v !== null);
  if (vals.length === 0) return null;
  return vals.reduce((a, b) => a + b, 0) / vals.length;
}

function fmtPct(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '—';
  return `${(v * 100).toFixed(1)}%`;
}

function fmtNum(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '—';
  return v.toFixed(3);
}

