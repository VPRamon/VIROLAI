/**
 * Tiny design system shared by every experiments page.
 *
 * Palette (matches TSI's slate/indigo dark chrome):
 *   - chrome:   slate-900 / slate-800 / slate-700
 *   - accent:   indigo-500 / indigo-400
 *   - status:   emerald (good), amber (in progress), rose (bad), slate (idle)
 *
 * Stick to these primitives and the experiments section will look
 * coherent without one-off Tailwind classes per page.
 */
import type { ReactNode } from 'react';

// ── Card ───────────────────────────────────────────────────────────────────

export function Card({
  children,
  className = '',
  padded = true,
  interactive = false,
}: {
  children: ReactNode;
  className?: string;
  padded?: boolean;
  interactive?: boolean;
}) {
  const base =
    'rounded-xl border border-slate-700 bg-slate-800/80 shadow-sm transition-colors';
  const hover = interactive
    ? 'hover:border-indigo-500 hover:bg-slate-800 cursor-pointer'
    : '';
  return (
    <div className={`${base} ${hover} ${padded ? 'p-5' : ''} ${className}`}>
      {children}
    </div>
  );
}

// ── Section header ─────────────────────────────────────────────────────────

export function SectionHeader({
  title,
  subtitle,
  actions,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-6 flex flex-wrap items-end justify-between gap-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight text-white">{title}</h1>
        {subtitle && (
          <p className="mt-1 text-sm text-slate-400">{subtitle}</p>
        )}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </div>
  );
}

// ── Status pill ────────────────────────────────────────────────────────────

export type StatusKind =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'started'
  | 'unknown';

const STATUS_STYLES: Record<StatusKind, string> = {
  pending: 'bg-slate-700/60 text-slate-300 border-slate-600',
  running: 'bg-amber-500/15 text-amber-300 border-amber-500/40',
  started: 'bg-amber-500/15 text-amber-300 border-amber-500/40',
  completed: 'bg-emerald-500/15 text-emerald-300 border-emerald-500/40',
  failed: 'bg-rose-500/15 text-rose-300 border-rose-500/40',
  unknown: 'bg-slate-700/60 text-slate-400 border-slate-600',
};

export function StatusPill({
  kind,
  children,
}: {
  kind: StatusKind | string | undefined | null;
  children?: ReactNode;
}) {
  const k = (kind ?? 'unknown') as StatusKind;
  const style = STATUS_STYLES[k] ?? STATUS_STYLES.unknown;
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium uppercase tracking-wide ${style}`}
    >
      <span className="size-1.5 rounded-full bg-current opacity-80" />
      {children ?? k}
    </span>
  );
}

// ── Metric badge ───────────────────────────────────────────────────────────

export function MetricBadge({
  label,
  value,
  hint,
  tone = 'default',
}: {
  label: ReactNode;
  value: ReactNode;
  hint?: ReactNode;
  tone?: 'default' | 'positive' | 'warning' | 'negative';
}) {
  const toneClass = {
    default: 'text-white',
    positive: 'text-emerald-300',
    warning: 'text-amber-300',
    negative: 'text-rose-300',
  }[tone];
  return (
    <div className="rounded-lg border border-slate-700 bg-slate-800/60 p-4">
      <div className="text-xs uppercase tracking-wide text-slate-400">{label}</div>
      <div className={`mt-1 text-2xl font-semibold tabular-nums ${toneClass}`}>{value}</div>
      {hint && <div className="mt-1 text-xs text-slate-500">{hint}</div>}
    </div>
  );
}

// ── Empty / error / skeleton ──────────────────────────────────────────────

export function EmptyState({
  title,
  description,
  action,
  icon,
}: {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-slate-700 bg-slate-800/40 px-6 py-12 text-center">
      {icon && <div className="mb-4 text-slate-500">{icon}</div>}
      <div className="text-base font-medium text-white">{title}</div>
      {description && (
        <div className="mt-1 max-w-md text-sm text-slate-400">{description}</div>
      )}
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

export function ErrorState({
  title = 'Something went wrong',
  error,
  onRetry,
}: {
  title?: ReactNode;
  error: Error | string | undefined | null;
  onRetry?: () => void;
}) {
  const message =
    error instanceof Error ? error.message : typeof error === 'string' ? error : 'Unknown error';
  return (
    <div className="rounded-xl border border-rose-500/40 bg-rose-500/10 p-5 text-rose-100">
      <div className="text-sm font-semibold">{title}</div>
      <div className="mt-1 text-sm text-rose-200/90">{message}</div>
      {onRetry && (
        <Button variant="secondary" className="mt-4" onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  );
}

export function Skeleton({ className = '' }: { className?: string }) {
  return (
    <div
      className={`animate-pulse rounded-md bg-slate-700/60 ${className}`}
      aria-hidden="true"
    />
  );
}

// ── Buttons ────────────────────────────────────────────────────────────────

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary:
    'bg-indigo-500 hover:bg-indigo-400 text-white border-indigo-400 shadow-sm',
  secondary:
    'bg-slate-700 hover:bg-slate-600 text-slate-100 border-slate-600',
  ghost:
    'bg-transparent hover:bg-slate-700/60 text-slate-200 border-transparent',
  danger:
    'bg-rose-500 hover:bg-rose-400 text-white border-rose-400',
};

export function Button({
  variant = 'primary',
  className = '',
  children,
  type = 'button',
  ...rest
}: {
  variant?: ButtonVariant;
  className?: string;
  children: ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type={type}
      className={`inline-flex items-center justify-center gap-2 rounded-lg border px-3.5 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${BUTTON_VARIANTS[variant]} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

// ── Input / select ─────────────────────────────────────────────────────────

export function TextField({
  label,
  hint,
  error,
  ...rest
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
} & React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <label className="block">
      <div className="mb-1.5 text-sm font-medium text-slate-200">{label}</div>
      <input
        {...rest}
        className={`block w-full rounded-lg border bg-slate-900/70 px-3 py-2 text-sm text-slate-100 placeholder:text-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-400 ${
          error ? 'border-rose-500/60' : 'border-slate-700'
        } ${rest.className ?? ''}`}
      />
      {hint && !error && <div className="mt-1 text-xs text-slate-500">{hint}</div>}
      {error && <div className="mt-1 text-xs text-rose-400">{error}</div>}
    </label>
  );
}

export function TextArea({
  label,
  hint,
  error,
  ...rest
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
} & React.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <label className="block">
      <div className="mb-1.5 text-sm font-medium text-slate-200">{label}</div>
      <textarea
        {...rest}
        className={`block w-full rounded-lg border bg-slate-900/70 px-3 py-2 font-mono text-xs text-slate-100 placeholder:text-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-400 ${
          error ? 'border-rose-500/60' : 'border-slate-700'
        } ${rest.className ?? ''}`}
      />
      {hint && !error && <div className="mt-1 text-xs text-slate-500">{hint}</div>}
      {error && <div className="mt-1 text-xs text-rose-400">{error}</div>}
    </label>
  );
}

export function Select<T extends string>({
  label,
  options,
  value,
  onChange,
  hint,
}: {
  label: ReactNode;
  options: ReadonlyArray<{ value: T; label: string }>;
  value: T;
  onChange: (v: T) => void;
  hint?: ReactNode;
}) {
  return (
    <label className="block">
      <div className="mb-1.5 text-sm font-medium text-slate-200">{label}</div>
      <select
        className="block w-full rounded-lg border border-slate-700 bg-slate-900/70 px-3 py-2 text-sm text-slate-100 focus:outline-none focus:ring-2 focus:ring-indigo-400"
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      {hint && <div className="mt-1 text-xs text-slate-500">{hint}</div>}
    </label>
  );
}

// ── Progress bar ───────────────────────────────────────────────────────────

export function ProgressBar({
  value,
  label,
}: {
  /** 0..1 */ value: number;
  label?: ReactNode;
}) {
  const pct = Math.min(100, Math.max(0, value * 100));
  return (
    <div>
      {label && (
        <div className="mb-1 flex justify-between text-xs text-slate-400">
          <span>{label}</span>
          <span className="tabular-nums">{pct.toFixed(0)}%</span>
        </div>
      )}
      <div
        className="h-2 w-full overflow-hidden rounded-full bg-slate-700"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="h-full bg-gradient-to-r from-indigo-500 to-emerald-400 transition-all"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

// ── Number formatting helpers ──────────────────────────────────────────────

export function fmtNumber(value: number | undefined | null, digits = 3): string {
  if (value == null || !Number.isFinite(value)) return '—';
  if (value === 0) return '0';
  const abs = Math.abs(value);
  if (abs >= 1000) return value.toLocaleString(undefined, { maximumFractionDigits: 0 });
  if (abs < 0.01) return value.toExponential(2);
  return value.toFixed(digits).replace(/\.?0+$/, '');
}

export function fmtPercent(value: number | undefined | null, digits = 1): string {
  if (value == null || !Number.isFinite(value)) return '—';
  return `${(value * 100).toFixed(digits)}%`;
}

export function fmtDuration(seconds: number | undefined | null): string {
  if (seconds == null || !Number.isFinite(seconds)) return '—';
  if (seconds < 60) return `${seconds.toFixed(0)}s`;
  if (seconds < 3600) return `${(seconds / 60).toFixed(1)}m`;
  return `${(seconds / 3600).toFixed(2)}h`;
}

export function fmtDate(iso: string | undefined | null): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

/** Reusable chart layout defaults — consistent dark theme + muted grid. */
export const PLOTLY_DARK_LAYOUT = {
  paper_bgcolor: 'rgba(0,0,0,0)',
  plot_bgcolor: 'rgba(0,0,0,0)',
  font: { color: '#cbd5e1', family: 'inherit', size: 12 },
  margin: { l: 56, r: 24, t: 32, b: 48 },
  xaxis: { gridcolor: 'rgba(148,163,184,0.15)', zerolinecolor: 'rgba(148,163,184,0.2)' },
  yaxis: { gridcolor: 'rgba(148,163,184,0.15)', zerolinecolor: 'rgba(148,163,184,0.2)' },
  hoverlabel: { bgcolor: '#0f172a', bordercolor: '#334155', font: { color: '#e2e8f0' } },
  colorway: ['#818cf8', '#34d399', '#fbbf24', '#f87171', '#22d3ee', '#a78bfa', '#f472b6'],
};

export const PLOTLY_DEFAULT_CONFIG = {
  displayModeBar: false,
  responsive: true,
};
