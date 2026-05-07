/**
 * `/experiments/new` — submit an `ExperimentSpec` to the backend.
 *
 * The spec mirror lives in `scripts/experiment_matrix/spec.rs`; for v1
 * the form gives users a structured surface for slug + datasets +
 * algorithms and a JSON escape hatch for sweep axes (everything else
 * an advanced user might want to control).
 */
import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { submitExperiment } from '../../lib/experiments/api';
import {
  Button,
  Card,
  ErrorState,
  SectionHeader,
  TextArea,
  TextField,
} from './_ui';

const KNOWN_ALGORITHMS = [
  { id: 'est', label: 'EST — Earliest Start Time' },
  { id: 'hap', label: 'HAP — Heuristic Allocation Planner' },
  { id: 'random', label: 'Random (baseline)' },
];

interface DatasetEntry {
  id: string;
  path: string;
}

export default function NewExperimentPage() {
  const navigate = useNavigate();
  const [slug, setSlug] = useState('');
  const [description, setDescription] = useState('');
  const [datasets, setDatasets] = useState<DatasetEntry[]>([{ id: '', path: '' }]);
  const [algorithms, setAlgorithms] = useState<string[]>(['est']);
  const [sweepsText, setSweepsText] = useState('{}');

  const [submitting, setSubmitting] = useState(false);
  const [serverError, setServerError] = useState<Error | undefined>(undefined);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const sweepsParseError = useMemo(() => {
    if (!sweepsText.trim()) return undefined;
    try {
      JSON.parse(sweepsText);
      return undefined;
    } catch (e) {
      return (e as Error).message;
    }
  }, [sweepsText]);

  function validate(): boolean {
    const errs: Record<string, string> = {};
    if (!slug.trim()) errs.slug = 'Slug is required';
    else if (!/^[a-z0-9][a-z0-9_-]*$/.test(slug.trim()))
      errs.slug = 'Use lowercase letters, digits, dashes, or underscores';
    if (algorithms.length === 0) errs.algorithms = 'Select at least one algorithm';
    const cleanDatasets = datasets.filter((d) => d.path.trim());
    if (cleanDatasets.length === 0) errs.datasets = 'Add at least one dataset path';
    if (sweepsParseError) errs.sweeps = `Invalid JSON: ${sweepsParseError}`;
    setFieldErrors(errs);
    return Object.keys(errs).length === 0;
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setServerError(undefined);
    if (!validate()) return;
    setSubmitting(true);
    try {
      const spec: Record<string, unknown> = {
        slug: slug.trim(),
        description: description.trim() || undefined,
        datasets: datasets
          .filter((d) => d.path.trim())
          .map((d) => ({ id: d.id.trim() || undefined, path: d.path.trim() })),
        algorithms,
      };
      if (sweepsText.trim()) {
        const parsed = JSON.parse(sweepsText);
        if (parsed && typeof parsed === 'object' && Object.keys(parsed).length > 0) {
          spec.sweeps = parsed;
        }
      }
      const result = await submitExperiment(spec);
      const targetSlug = (result.slug as string | undefined) ?? slug.trim();
      const targetRun = result.run_id as string | undefined;
      if (targetRun) {
        navigate(`/experiments/${encodeURIComponent(targetSlug)}/${encodeURIComponent(targetRun)}`);
      } else {
        navigate('/experiments');
      }
    } catch (err) {
      setServerError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setSubmitting(false);
    }
  }

  function toggleAlgorithm(id: string) {
    setAlgorithms((prev) => (prev.includes(id) ? prev.filter((a) => a !== id) : [...prev, id]));
  }

  return (
    <form onSubmit={onSubmit} className="mx-auto max-w-3xl px-6 py-8">
      <SectionHeader
        title="New experiment"
        subtitle="Submit a matrix of (dataset × algorithm × config) cells to the experiment runner."
        actions={
          <Button variant="ghost" onClick={() => navigate('/experiments')}>
            Cancel
          </Button>
        }
      />

      {serverError && (
        <div className="mb-6">
          <ErrorState title="Submission failed" error={serverError} />
        </div>
      )}

      <div className="space-y-6">
        <Card>
          <h2 className="mb-4 text-base font-semibold text-white">Identity</h2>
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <TextField
              label="Slug"
              placeholder="e.g. baseline-2024"
              value={slug}
              onChange={(e) => setSlug(e.target.value)}
              error={fieldErrors.slug}
              hint="Lowercase, dash- or underscore-separated identifier."
            />
            <TextField
              label="Description (optional)"
              placeholder="One-line summary"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>
        </Card>

        <Card>
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-base font-semibold text-white">Datasets</h2>
            <Button
              variant="secondary"
              onClick={() => setDatasets((prev) => [...prev, { id: '', path: '' }])}
            >
              + Add dataset
            </Button>
          </div>
          {fieldErrors.datasets && (
            <p className="mb-3 text-xs text-rose-400">{fieldErrors.datasets}</p>
          )}
          <div className="space-y-3">
            {datasets.map((d, i) => (
              <div key={i} className="grid grid-cols-1 gap-3 md:grid-cols-[160px_1fr_auto]">
                <TextField
                  label={i === 0 ? 'ID (optional)' : ''}
                  placeholder="ctao-n"
                  value={d.id}
                  onChange={(e) =>
                    setDatasets((prev) =>
                      prev.map((row, idx) => (idx === i ? { ...row, id: e.target.value } : row)),
                    )
                  }
                />
                <TextField
                  label={i === 0 ? 'Path' : ''}
                  placeholder="data/ctao_n.json"
                  value={d.path}
                  onChange={(e) =>
                    setDatasets((prev) =>
                      prev.map((row, idx) => (idx === i ? { ...row, path: e.target.value } : row)),
                    )
                  }
                />
                <div className={i === 0 ? 'pt-7' : ''}>
                  <Button
                    variant="ghost"
                    onClick={() => setDatasets((prev) => prev.filter((_, idx) => idx !== i))}
                    aria-label="Remove dataset"
                  >
                    ✕
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </Card>

        <Card>
          <h2 className="mb-4 text-base font-semibold text-white">Algorithms</h2>
          {fieldErrors.algorithms && (
            <p className="mb-3 text-xs text-rose-400">{fieldErrors.algorithms}</p>
          )}
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
            {KNOWN_ALGORITHMS.map((a) => {
              const checked = algorithms.includes(a.id);
              return (
                <label
                  key={a.id}
                  className={`flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 transition-colors ${
                    checked
                      ? 'border-indigo-500 bg-indigo-500/10'
                      : 'border-slate-700 bg-slate-900/40 hover:border-slate-500'
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => toggleAlgorithm(a.id)}
                    className="size-4 accent-indigo-500"
                  />
                  <div>
                    <div className="text-sm font-medium text-white">{a.id}</div>
                    <div className="text-xs text-slate-400">{a.label}</div>
                  </div>
                </label>
              );
            })}
          </div>
          <p className="mt-3 text-xs text-slate-500">
            Custom algorithms registered in the runner can be added via the JSON sweeps field below.
          </p>
        </Card>

        <Card>
          <h2 className="mb-1 text-base font-semibold text-white">Sweep axes (advanced)</h2>
          <p className="mb-3 text-xs text-slate-500">
            Per-algorithm parameter sweeps. Mirrors the
            <code className="mx-1 rounded bg-slate-900 px-1 py-0.5 text-[11px]">sweeps</code>
            field of <code className="rounded bg-slate-900 px-1 py-0.5 text-[11px]">ExperimentSpec</code>.
            Leave as <code className="rounded bg-slate-900 px-1 py-0.5 text-[11px]">{'{}'}</code> for
            the default config.
          </p>
          <TextArea
            label=""
            rows={8}
            spellCheck={false}
            value={sweepsText}
            onChange={(e) => setSweepsText(e.target.value)}
            error={fieldErrors.sweeps}
            placeholder='{\n  "est": { "alpha": [0.1, 0.5, 1.0] }\n}'
          />
        </Card>

        <div className="flex items-center justify-end gap-3">
          <Button variant="ghost" onClick={() => navigate('/experiments')}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={submitting}>
            {submitting ? 'Submitting…' : 'Submit experiment'}
          </Button>
        </div>
      </div>
    </form>
  );
}
