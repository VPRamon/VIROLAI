/**
 * `/experiments` — landing surface listing every experiment run.
 */
import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { listExperiments } from '../../lib/experiments/api';
import { useAsync } from '../../lib/experiments/useAsync';
import type { ExperimentSummary, RunStatus } from '../../lib/experiments/types';
import {
  Button,
  Card,
  EmptyState,
  ErrorState,
  Skeleton,
  SectionHeader,
  Select,
  StatusPill,
  TextField,
  fmtDate,
} from './_ui';

type SortKey = 'updated_at' | 'created_at' | 'slug';

export default function ExperimentsListPage() {
  const { data, error, loading, reload } = useAsync(() => listExperiments(), []);
  const [query, setQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState<'all' | RunStatus>('all');
  const [sort, setSort] = useState<SortKey>('updated_at');

  const filtered = useMemo<ExperimentSummary[]>(() => {
    const all = data?.experiments ?? [];
    const needle = query.trim().toLowerCase();
    let rows = all.filter((e) => {
      if (statusFilter !== 'all' && e.status !== statusFilter) return false;
      if (!needle) return true;
      return (
        e.experiment_slug.toLowerCase().includes(needle) ||
        e.experiment_name.toLowerCase().includes(needle) ||
        e.run_id.toLowerCase().includes(needle)
      );
    });
    rows = [...rows].sort((a, b) => {
      if (sort === 'slug') return a.experiment_slug.localeCompare(b.experiment_slug);
      const av = sort === 'updated_at' ? a.updated_at : a.created_at;
      const bv = sort === 'updated_at' ? b.updated_at : b.created_at;
      return bv.localeCompare(av);
    });
    return rows;
  }, [data, query, statusFilter, sort]);

  return (
    <div className="mx-auto max-w-7xl px-6 py-8">
      <SectionHeader
        title="Experiments"
        subtitle="Algorithm-evaluation runs produced by the experiment matrix."
        actions={
          <Link to="/experiments/new">
            <Button variant="primary">+ New experiment</Button>
          </Link>
        }
      />

      <Card className="mb-6" padded>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
          <TextField
            label="Search"
            placeholder="Filter by slug, name, or run id…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <Select
            label="Status"
            value={statusFilter}
            onChange={(v) => setStatusFilter(v)}
            options={[
              { value: 'all', label: 'Any status' },
              { value: 'running', label: 'Running' },
              { value: 'completed', label: 'Completed' },
              { value: 'failed', label: 'Failed' },
              { value: 'pending', label: 'Pending' },
            ]}
          />
          <Select
            label="Sort by"
            value={sort}
            onChange={(v) => setSort(v)}
            options={[
              { value: 'updated_at', label: 'Recently updated' },
              { value: 'created_at', label: 'Recently created' },
              { value: 'slug', label: 'Slug (A→Z)' },
            ]}
          />
        </div>
      </Card>

      {loading && <ListSkeleton />}
      {!loading && error && <ErrorState error={error} onRetry={reload} />}
      {!loading && !error && filtered.length === 0 && (
        <EmptyState
          title={data?.experiments.length ? 'No experiments match your filters' : 'No experiments yet'}
          description={
            data?.experiments.length
              ? 'Try clearing the search or status filter.'
              : 'Submit your first matrix run to populate this dashboard.'
          }
          action={
            !data?.experiments.length && (
              <Link to="/experiments/new">
                <Button variant="primary">+ New experiment</Button>
              </Link>
            )
          }
        />
      )}

      {!loading && !error && filtered.length > 0 && (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {filtered.map((exp) => (
            <ExperimentCard key={`${exp.experiment_slug}/${exp.run_id}`} exp={exp} />
          ))}
        </div>
      )}
    </div>
  );
}

function ExperimentCard({ exp }: { exp: ExperimentSummary }) {
  const target = `/experiments/${encodeURIComponent(exp.experiment_slug)}/${encodeURIComponent(exp.run_id)}`;
  return (
    <Link to={target} className="block">
      <Card interactive>
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="truncate text-base font-semibold text-white">
              {exp.experiment_slug}
            </div>
            <div className="mt-0.5 truncate text-xs text-slate-400">{exp.run_id}</div>
          </div>
          <StatusPill kind={exp.status} />
        </div>
        <div className="mt-4 grid grid-cols-3 gap-2 text-center">
          <Stat label="Total" value={exp.total_cells} />
          <Stat label="Done" value={exp.completed_cells} tone="positive" />
          <Stat label="Failed" value={exp.failed_cells} tone={exp.failed_cells > 0 ? 'negative' : 'default'} />
        </div>
        <div className="mt-4 flex items-center justify-between text-xs text-slate-500">
          <span>Updated {fmtDate(exp.updated_at)}</span>
          {exp.running_cells > 0 && (
            <span className="text-amber-300">{exp.running_cells} running</span>
          )}
        </div>
      </Card>
    </Link>
  );
}

function Stat({
  label,
  value,
  tone = 'default',
}: {
  label: string;
  value: number;
  tone?: 'default' | 'positive' | 'negative';
}) {
  const toneClass = {
    default: 'text-slate-200',
    positive: 'text-emerald-300',
    negative: 'text-rose-300',
  }[tone];
  return (
    <div className="rounded-md border border-slate-700 bg-slate-900/40 px-2 py-1.5">
      <div className="text-[10px] uppercase tracking-wide text-slate-500">{label}</div>
      <div className={`text-base font-semibold tabular-nums ${toneClass}`}>{value}</div>
    </div>
  );
}

function ListSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <Card key={i}>
          <Skeleton className="h-5 w-2/3" />
          <Skeleton className="mt-2 h-3 w-1/3" />
          <div className="mt-4 grid grid-cols-3 gap-2">
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
          </div>
        </Card>
      ))}
    </div>
  );
}
