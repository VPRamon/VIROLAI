/**
 * Per-algorithm tab — sensitivity to per-algorithm configuration
 * sweeps. Stubbed in v1 with a "coming soon" empty state per the
 * scope-cut guidance: shipping five inconsistent analytics tabs is
 * worse than four polished ones plus a clear placeholder for the
 * fifth.
 */
import { useParams } from 'react-router-dom';
import { Button, Card, EmptyState } from '../_ui';

export default function PerAlgorithmTab() {
  const { slug = '', runId = '' } = useParams();
  const matrix = `/experiments/${encodeURIComponent(slug)}/${encodeURIComponent(runId)}/matrix`;
  return (
    <Card>
      <EmptyState
        title="Per-algorithm sensitivity — coming soon"
        description={
          <>
            v1 ships per-dataset rollups, the matrix, and the Pareto explorer. The
            per-algorithm sensitivity surface (parallel-coordinates over each
            algorithm's sweep axes) is in flight for the next iteration. In the
            meantime, you can pivot the matrix by metric to spot algorithm-level
            effects.
          </>
        }
        action={
          <a href={matrix}>
            <Button variant="primary">Open matrix</Button>
          </a>
        }
      />
    </Card>
  );
}
