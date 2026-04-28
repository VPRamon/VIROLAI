/**
 * FocusBadge — shared, accessible "focused runs" indicator for the EST
 * algorithm-analysis tabs.
 *
 * The chip is rendered only when {@link useRunFocus} reports an active
 * focus.  It is itself a `<button>` (so screen readers and keyboard
 * users can activate it directly) that clears the focus on click; an
 * `Esc` key handler does the same.  An adjacent `aria-live="polite"`
 * region re-announces the count whenever it changes, so assistive
 * technology users learn about cross-panel focus updates without
 * having to leave the current chart.
 */
import { useEffect } from 'react';
import { useRunFocus } from './useRunFocus';

export interface FocusBadgeProps {
  /** Optional Tailwind palette override (defaults to sky). */
  tone?: 'sky' | 'emerald' | 'primary';
}

const TONE_CLASSES: Record<NonNullable<FocusBadgeProps['tone']>, string> = {
  sky: 'border-sky-700/40 bg-sky-950/30 text-sky-200',
  emerald: 'border-emerald-700/40 bg-emerald-950/30 text-emerald-200',
  primary: 'border-primary-700 bg-primary-900/40 text-primary-300',
};

export function FocusBadge({ tone = 'sky' }: FocusBadgeProps) {
  const focus = useRunFocus();
  const count = focus.focused.size;
  const active = focus.active;

  // Esc clears focus regardless of which panel mounted the badge.
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') focus.clear();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [active, focus]);

  return (
    <>
      {/* Live region is always rendered so SR users hear the transition
          to/from "no focus"; it carries the same text either way. */}
      <div className="sr-only" aria-live="polite" role="status">
        {active ? `Focused: ${count} run${count === 1 ? '' : 's'}` : 'Focus cleared'}
      </div>

      {active && (
        <button
          type="button"
          onClick={() => focus.clear()}
          aria-label={`Clear focus on ${count} run${count === 1 ? '' : 's'}`}
          title="Press Esc to clear focus"
          className={`inline-flex items-center gap-2 rounded border px-3 py-1.5 text-xs font-medium hover:brightness-110 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 ${TONE_CLASSES[tone]}`}
        >
          <span>
            Focused: <span className="font-semibold">{count}</span> run
            {count === 1 ? '' : 's'}
          </span>
          <span aria-hidden="true">·</span>
          <span aria-hidden="true">Clear</span>
        </button>
      )}
    </>
  );
}

export default FocusBadge;
