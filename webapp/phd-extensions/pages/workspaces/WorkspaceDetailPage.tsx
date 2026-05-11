/**
 * `/workspaces/:id` — workspace detail shell with tab bar.
 *
 * Loads workspace record + comparison summaries once and exposes them
 * via `WorkspaceCtx` to every child tab. Each tab is code-split with
 * React.lazy so heavy Plotly panels don't block the initial render.
 *
 * Tabs: Overview · Comparison · Pareto · Per-dataset · Per-algorithm · Stairs
 */
import { Suspense, createContext, lazy, useCallback, useContext, useEffect, useState } from 'react';
import { Link, NavLink, Route, Routes, useParams } from 'react-router-dom';
import {
  WorkspacesApiError,
  getComparison,
  getWorkspace,
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
        <Card className="mb-6">
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

