/**
 * Thin tab adapters: read the shell-provided selection via `useAlgorithm`,
 * fan out to per-schedule queries through `useRunMatrix`, and forward the
 * resulting `RunRow[]` to each EST panel.  Each adapter is registered as a
 * `TsiAlgorithmTab.component` in the extensions registry.
 */
import { LoadingSpinner } from '@/components';
import { useAlgorithm } from '@/pages/AlgorithmAnalysis';
import { useRunMatrix } from './useRunMatrix';
import OverviewPanel from './panels/OverviewPanel';
import SensitivityPanel from './panels/SensitivityPanel';
import ParetoPanel from './panels/ParetoPanel';
import InternalsPanel from './panels/InternalsPanel';
import StatisticsPanel from './panels/StatisticsPanel';

function withRuns(Panel: (props: { runs: ReturnType<typeof useRunMatrix>['runs'] }) => JSX.Element) {
  return function Tab() {
    const { selectedSchedules } = useAlgorithm();
    const { runs, loading } = useRunMatrix(selectedSchedules);
    if (loading && runs.every((r) => !r.insights && !r.iterations)) {
      return <LoadingSpinner />;
    }
    return <Panel runs={runs} />;
  };
}

export const OverviewTab = withRuns(OverviewPanel);
export const SensitivityTab = withRuns(SensitivityPanel);
export const ParetoTab = withRuns(ParetoPanel);
export const InternalsTab = withRuns(InternalsPanel);
export const StatisticsTab = withRuns(StatisticsPanel);
