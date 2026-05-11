/**
 * `/workspaces/:id` — workspace detail shell with tab bar.
 *
 * Loads workspace record + comparison summaries once and exposes them
 * via `WorkspaceCtx` to every child tab. Each tab is code-split with
 * React.lazy so heavy Plotly panels don't block the initial render.
 *
 * Tabs: Overview · Comparison · Pareto · Per-dataset · Per-algorithm · Stairs
 */
import { Suspense, createContext, lazy, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { Link, NavLink, Route, Routes, useParams } from 'react-router-dom';
import {
  WorkspacesApiError,
  addManifest,
  addManifestBatch,
  getComparison,
  getWorkspace,
  ingestSchedule,
} from '../../lib/workspaces/api';
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
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const manifestRef = useRef<HTMLInputElement>(null);
  const scheduleRef = useRef<HTMLInputElement>(null);

  // Auto-expand when workspace becomes empty (e.g. after removing last manifest)
  useEffect(() => {
    if (isEmpty) setOpen(true);
  }, [isEmpty]);

  async function onManifestUpload(files: FileList | null) {
    if (!files || files.length === 0) return;
    setError(null);
    setUploading(true);
    try {
      if (files.length === 1) {
        const text = await files[0].text();
        await addManifest(workspaceId, JSON.parse(text));
      } else {
        const items: { manifest: unknown }[] = [];
        for (const f of Array.from(files)) {
          items.push({ manifest: JSON.parse(await f.text()) });
        }
        await addManifestBatch(workspaceId, items);
      }
      if (manifestRef.current) manifestRef.current.value = '';
      onUploaded();
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    } finally {
      setUploading(false);
    }
  }

  async function onScheduleUpload(files: FileList | null) {
    if (!files || files.length === 0) return;
    setError(null);
    setUploading(true);
    try {
      for (const f of Array.from(files)) {
        await ingestSchedule(workspaceId, JSON.parse(await f.text()));
      }
      if (scheduleRef.current) scheduleRef.current.value = '';
      onUploaded();
    } catch (e) {
      setError(e instanceof WorkspacesApiError ? e.message : String(e));
    } finally {
      setUploading(false);
    }
  }

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
          Add data to this workspace
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
          <div className="grid grid-cols-1 gap-6 md:grid-cols-2">
            {/* Manifest upload */}
            <div>
              <div className="mb-1 text-sm font-medium text-slate-200">Upload manifests</div>
              <p className="mb-3 text-xs text-slate-500">
                JSON files produced by{' '}
                <code className="text-slate-300">phd manifest create</code>. Select one or
                multiple files at once.
              </p>
              <input
                ref={manifestRef}
                type="file"
                multiple
                accept="application/json,.json"
                disabled={uploading}
                onChange={(e) => onManifestUpload(e.target.files)}
                className="block w-full text-sm text-slate-300 file:mr-3 file:cursor-pointer file:rounded-lg file:border file:border-slate-600 file:bg-slate-700 file:px-3 file:py-1.5 file:text-xs file:font-medium file:text-slate-200 hover:file:bg-slate-600 disabled:opacity-50"
              />
            </div>

            {/* Schedule upload */}
            <div>
              <div className="mb-1 text-sm font-medium text-slate-200">Upload schedules</div>
              <p className="mb-3 text-xs text-slate-500">
                Full schedule JSON files (with embedded{' '}
                <code className="text-slate-300">schedule_metrics</code>). A manifest is
                auto-derived and only metrics are stored.
              </p>
              <input
                ref={scheduleRef}
                type="file"
                multiple
                accept="application/json,.json"
                disabled={uploading}
                onChange={(e) => onScheduleUpload(e.target.files)}
                className="block w-full text-sm text-slate-300 file:mr-3 file:cursor-pointer file:rounded-lg file:border file:border-slate-600 file:bg-slate-700 file:px-3 file:py-1.5 file:text-xs file:font-medium file:text-slate-200 hover:file:bg-slate-600 disabled:opacity-50"
              />
            </div>
          </div>

          {uploading && <p className="mt-3 text-xs text-amber-300">Uploading…</p>}
          {error && <p className="mt-3 text-xs text-rose-400">{error}</p>}

          <p className="mt-4 text-xs text-slate-500">
            Or publish from the terminal:{' '}
            <code className="text-slate-300">
              phd publish --workspace {workspaceId} --manifest-dir out/&lt;run&gt;
            </code>
          </p>
        </Card>
      )}
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

