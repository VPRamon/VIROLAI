/**
 * PhD-specific TSI extensions.
 *
 * - routes:     React-Router RouteObject[] placed under the Layout route.
 * - navItems:   appear in the top navigation bar.
 *
 * Heavy panels are loaded with `React.lazy` so they are code-split out of
 * the main TSI bundle. The TSI shell wraps every route element in a
 * `<Suspense>` boundary.
 *
 * The @ext alias in vite.config.ts points to this directory.
 */
import { Suspense, lazy } from "react";
import { EXTENSION_CONTRACT_VERSION, type TsiExtensions } from "@/extensions";
import { LoadingSpinner } from "@/components";

if (EXTENSION_CONTRACT_VERSION !== 1) {
  // Bumping the contract is a breaking change — fail loudly so the
  // integrator notices at startup rather than at runtime.
  throw new Error(
    `phd-extensions targets contract v1 but TSI exposes v${EXTENSION_CONTRACT_VERSION}`,
  );
}

// ── Experiments section (top-level nav) ────────────────────────────────────
//
// The Experiments surface is mounted as a top-level destination (not a
// per-algorithm tab) because it spans many algorithms. It has its own
// data layer (`./lib/experiments`) and design system (`./pages/experiments/_ui`)
// that don't reach into TSI internals — only `@/extensions` and
// `@/components` are touched, in line with the v1 extension contract.

const ExperimentsListPage = lazy(
  () => import("./pages/experiments/ExperimentsListPage"),
);
const NewExperimentPage = lazy(
  () => import("./pages/experiments/NewExperimentPage"),
);
const ExperimentDetailPage = lazy(
  () => import("./pages/experiments/ExperimentDetailPage"),
);

function lazyRoute(node: React.ReactNode): React.ReactNode {
  return <Suspense fallback={<LoadingSpinner />}>{node}</Suspense>;
}

export const extensions: TsiExtensions = {
  routes: [
    { path: "experiments", element: lazyRoute(<ExperimentsListPage />) },
    { path: "experiments/new", element: lazyRoute(<NewExperimentPage />) },
    // Trailing `/*` lets the detail page mount its own nested
    // <Routes> for tabs and the cell-detail subroute.
    {
      path: "experiments/:slug/:runId/*",
      element: lazyRoute(<ExperimentDetailPage />),
    },
  ],
  navItems: [],
  algorithms: [],
};
