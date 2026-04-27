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
import { lazy } from 'react';
import { EXTENSION_CONTRACT_VERSION, type TsiExtensions } from '@/extensions';

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

export const extensions: TsiExtensions = {
  routes: [],
  navItems: [],
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

