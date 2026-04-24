/**
 * PhD-specific TSI extensions.
 *
 * Add routes and navigation items here to inject them into the TSI app shell
 * without modifying TSI core source files.
 *
 * - routes:   React-Router RouteObject[]  placed under the Layout route
 * - navItems: appear in the top navigation bar
 *
 * The @ext alias in vite.config.ts points to this directory.
 */
import { lazy, Suspense } from 'react';
import type { TsiExtensions } from '@/extensions';

const EstSweep = lazy(() => import('./pages/EstSweep'));

export const extensions: TsiExtensions = {
  routes: [
    {
      path: 'est-sweep',
      element: (
        <Suspense fallback={null}>
          <EstSweep />
        </Suspense>
      ),
    },
  ],
  navItems: [
    {
      path: '/est-sweep',
      label: 'EST Sweep',
      scope: 'global',
      icon: (
        <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
          />
        </svg>
      ),
    },
  ],
};
