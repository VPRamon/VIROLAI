/**
 * VIROLAI-specific TSI extensions.
 *
 * Historical source directory: `webapp/phd-extensions/`.
 */
import { Suspense, lazy } from "react";
import { EXTENSION_CONTRACT_VERSION, type TsiExtensions } from "@/extensions";
import { LoadingSpinner } from "@/components";

if (EXTENSION_CONTRACT_VERSION !== 1) {
  throw new Error(
    `VIROLAI extensions target contract v1 but TSI exposes v${EXTENSION_CONTRACT_VERSION}`,
  );
}

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
    {
      path: "experiments/:slug/:runId/*",
      element: lazyRoute(<ExperimentDetailPage />),
    },
  ],
  navItems: [],
  algorithms: [],
};
