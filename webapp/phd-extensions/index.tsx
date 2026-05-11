/**
 * PhD-specific TSI extensions.
 *
 * - routes:     React-Router RouteObject[] placed under the Layout route
 *               (reserved for future PhD-specific routes if needed).
 * - navItems:   appear in the top navigation bar.
 * - algorithms: per-algorithm dashboard tabs surfaced under
 *               `/algorithm/{algoId}/{tabId}` by the TSI core
 *               `AlgorithmAnalysisPage`.
 *
 * Tabs are loaded with `React.lazy` so the heavy EST-specific panels
 * (Plotly charts, sweep matrix, etc.) are code-split out of the main
 * TSI bundle. The TSI shell wraps every tab in a `<Suspense>` boundary.
 *
 * The @ext alias in vite.config.ts points to this directory.
 */
import { Suspense, lazy } from 'react';
import { EXTENSION_CONTRACT_VERSION, type TsiExtensions } from '@/extensions';
import { LoadingSpinner } from '@/components';

if (EXTENSION_CONTRACT_VERSION !== 1) {
  // Bumping the contract is a breaking change — fail loudly so the
  // integrator notices at startup rather than at runtime.
  throw new Error(
    `phd-extensions targets contract v1 but TSI exposes v${EXTENSION_CONTRACT_VERSION}`,
  );
}

const OverviewTab = lazy(() =>
  import('./pages/algorithms/est/tabs').then((m) => ({ default: m.OverviewTab })),
);
const SensitivityTab = lazy(() =>
  import('./pages/algorithms/est/tabs').then((m) => ({ default: m.SensitivityTab })),
);
const ParetoTab = lazy(() =>
  import('./pages/algorithms/est/tabs').then((m) => ({ default: m.ParetoTab })),
);
const InternalsTab = lazy(() =>
  import('./pages/algorithms/est/tabs').then((m) => ({ default: m.InternalsTab })),
);
const StatisticsTab = lazy(() =>
  import('./pages/algorithms/est/tabs').then((m) => ({ default: m.StatisticsTab })),
);
const SweepPanel = lazy(() => import('./pages/algorithms/est/panels/SweepPanel'));

// ── Experiments section (top-level nav) ────────────────────────────────────
//
// The Experiments surface is mounted as a top-level destination (not a
// per-algorithm tab) because it spans many algorithms. It has its own
// data layer (`./lib/experiments`) and design system (`./pages/experiments/_ui`)
// that don't reach into TSI internals — only `@/extensions` and
// `@/components` are touched, in line with the v1 extension contract.

const ExperimentsListPage = lazy(() => import('./pages/experiments/ExperimentsListPage'));
const NewExperimentPage = lazy(() => import('./pages/experiments/NewExperimentPage'));
const ExperimentDetailPage = lazy(() => import('./pages/experiments/ExperimentDetailPage'));

// ── Workspaces section (manifest-first comparison) ─────────────────────────
const WorkspacesListPage = lazy(() => import('./pages/workspaces/WorkspacesListPage'));
const WorkspaceDetailPage = lazy(() => import('./pages/workspaces/WorkspaceDetailPage'));

function lazyRoute(node: React.ReactNode): React.ReactNode {
  return <Suspense fallback={<LoadingSpinner />}>{node}</Suspense>;
}

export const extensions: TsiExtensions = {
  routes: [
    { path: 'experiments', element: lazyRoute(<ExperimentsListPage />) },
    { path: 'experiments/new', element: lazyRoute(<NewExperimentPage />) },
    // Trailing `/*` lets the detail page mount its own nested
    // <Routes> for tabs and the cell-detail subroute.
    { path: 'experiments/:slug/:runId/*', element: lazyRoute(<ExperimentDetailPage />) },
    { path: 'workspaces', element: lazyRoute(<WorkspacesListPage />) },
    // Trailing `/*` lets the detail page mount its own nested <Routes> for tabs.
    { path: 'workspaces/:id/*', element: lazyRoute(<WorkspaceDetailPage />) },
  ],
  navItems: [
    { path: '/workspaces', label: 'Workspaces', scope: 'global' },
  ],
  algorithms: [
    {
      id: 'est',
      label: 'EST',
      tabs: [
        { id: 'overview', label: 'Overview', component: OverviewTab },
        { id: 'sensitivity', label: 'Sensitivity', component: SensitivityTab },
        { id: 'pareto', label: 'Pareto', component: ParetoTab },
        { id: 'internals', label: 'Internals', component: InternalsTab },
        { id: 'statistics', label: 'Statistics', component: StatisticsTab },
        { id: 'sweep', label: 'Sweep', component: SweepPanel },
      ],
    },
  ],
};

