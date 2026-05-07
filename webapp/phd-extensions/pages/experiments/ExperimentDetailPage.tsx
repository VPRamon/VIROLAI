/**
 * `/experiments/:slug/:runId` — detail shell with header, live progress
 * bar (driven by SSE) and a tab bar for the analytics surfaces. Each
 * tab consumes `useExperimentRunContext` so the SSE stream is opened
 * exactly once per (slug, runId) pair.
 */
import { Suspense, createContext, lazy, useContext } from 'react';
import { Link, NavLink, Route, Routes, useParams } from 'react-router-dom';
import { cancelRun, resumeRun, summaryCsvUrl } from '../../lib/experiments/api';
import {
  type UseExperimentRunResult,
  useExperimentRun,
} from '../../lib/experiments/useExperimentRun';
import {
  Button,
  Card,
  ErrorState,
  ProgressBar,
  Skeleton,
  SectionHeader,
  StatusPill,
  fmtDate,
} from './_ui';

const OverviewTab = lazy(() => import('./tabs/OverviewTab'));
const MatrixTab = lazy(() => import('./tabs/MatrixTab'));
const ParetoTab = lazy(() => import('./tabs/ParetoTab'));
const PerDatasetTab = lazy(() => import('./tabs/PerDatasetTab'));
const PerAlgorithmTab = lazy(() => import('./tabs/PerAlgorithmTab'));
const CellDetailPage = lazy(() => import('./CellDetailPage'));

const RunCtx = createContext<UseExperimentRunResult | null>(null);

export function useExperimentRunContext(): UseExperimentRunResult {
  const ctx = useContext(RunCtx);
  if (!ctx) {
    throw new Error('useExperimentRunContext must be used inside ExperimentDetailPage');
  }
  return ctx;
}

const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'matrix', label: 'Matrix' },
  { id: 'pareto', label: 'Pareto' },
  { id: 'per-dataset', label: 'Per dataset' },
  { id: 'per-algorithm', label: 'Per algorithm' },
] as const;

export default function ExperimentDetailPage() {
  const { slug = '', runId = '' } = useParams();
  const run = useExperimentRun(slug, runId);

  return (
    <RunCtx.Provider value={run}>
      <div className="mx-auto max-w-7xl px-6 py-8">
        <Header slug={slug} runId={runId} />
        <Routes>
          <Route path="cells/:cellId" element={<TabFrame><CellDetailPage /></TabFrame>} />
          <Route path="matrix" element={<TabFrame><MatrixTab /></TabFrame>} />
          <Route path="pareto" element={<TabFrame><ParetoTab /></TabFrame>} />
          <Route path="per-dataset" element={<TabFrame><PerDatasetTab /></TabFrame>} />
          <Route path="per-algorithm" element={<TabFrame><PerAlgorithmTab /></TabFrame>} />
          <Route path="overview" element={<TabFrame><OverviewTab /></TabFrame>} />
          <Route path="*" element={<TabFrame><OverviewTab /></TabFrame>} />
        </Routes>
      </div>
    </RunCtx.Provider>
  );
}

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

function Header({ slug, runId }: { slug: string; runId: string }) {
  const { data, error, loading, reload, counters, connected } = useExperimentRunContext();

  return (
    <div className="mb-6 space-y-4">
      <div className="text-xs text-slate-500">
        <Link to="/experiments" className="hover:text-indigo-300">
          Experiments
        </Link>
        <span className="mx-1.5">/</span>
        <span className="text-slate-300">{slug}</span>
        <span className="mx-1.5">/</span>
        <span className="text-slate-400">{runId}</span>
      </div>

      <SectionHeader
        title={
          <span className="flex items-center gap-3">
            {data?.experiment_slug ?? slug}
            <StatusPill kind={data?.status} />
            <span
              className={`size-2 rounded-full ${connected ? 'bg-emerald-400' : 'bg-slate-600'}`}
              title={connected ? 'Live' : 'Disconnected'}
            />
          </span>
        }
        subtitle={
          data ? (
            <>
              Run <code className="text-slate-300">{data.run_id}</code> · created {fmtDate(data.created_at)} · updated{' '}
              {fmtDate(data.updated_at)}
            </>
          ) : (
            <Skeleton className="h-4 w-64" />
          )
        }
        actions={
          data && (
            <>
              {data.status === 'running' && (
                <Button
                  variant="secondary"
                  onClick={async () => {
                    await cancelRun(slug, runId).catch(() => undefined);
                    reload();
                  }}
                >
                  Cancel run
                </Button>
              )}
              {(data.status === 'failed' || data.status === 'pending') && (
                <Button
                  variant="secondary"
                  onClick={async () => {
                    await resumeRun(slug, runId).catch(() => undefined);
                    reload();
                  }}
                >
                  Resume
                </Button>
              )}
              <a href={summaryCsvUrl(slug, runId)} target="_blank" rel="noreferrer">
                <Button variant="ghost">Download CSV</Button>
              </a>
            </>
          )
        }
      />

      {error && <ErrorState error={error} onRetry={reload} />}

      {!error && (
        <Card>
          <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
            <Counter label="Total cells" value={counters.total} loading={loading} />
            <Counter label="Completed" value={counters.completed} tone="positive" loading={loading} />
            <Counter label="Running" value={counters.started} tone="warning" loading={loading} />
            <Counter label="Failed" value={counters.failed} tone={counters.failed > 0 ? 'negative' : 'default'} loading={loading} />
          </div>
          <div className="mt-5">
            <ProgressBar value={counters.progress} label="Run progress" />
          </div>
        </Card>
      )}

      <TabBar />
    </div>
  );
}

function Counter({
  label,
  value,
  tone = 'default',
  loading,
}: {
  label: string;
  value: number;
  tone?: 'default' | 'positive' | 'warning' | 'negative';
  loading?: boolean;
}) {
  const toneClass = {
    default: 'text-white',
    positive: 'text-emerald-300',
    warning: 'text-amber-300',
    negative: 'text-rose-300',
  }[tone];
  return (
    <div>
      <div className="text-xs uppercase tracking-wide text-slate-400">{label}</div>
      {loading ? (
        <Skeleton className="mt-1 h-7 w-16" />
      ) : (
        <div className={`mt-1 text-2xl font-semibold tabular-nums ${toneClass}`}>{value}</div>
      )}
    </div>
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
