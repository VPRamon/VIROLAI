/**
 * Pure helpers backing the EST Internals panel — extracted so they can be
 * unit-tested without dragging in the Plotly bundle (which side-effects on
 * canvas APIs that jsdom does not implement).
 */
import type { EstTraceIteration } from '../useRunMatrix';

const num = (v: unknown): number | null => {
  if (typeof v === 'number' && Number.isFinite(v)) return v;
  return null;
};

/** Build the running max ("best-so-far") of `best_score` aligned with each round. */
export function bestSoFar(iters: EstTraceIteration[]): Array<{ round: number; value: number }> {
  const out: Array<{ round: number; value: number }> = [];
  let running = -Infinity;
  iters.forEach((it, i) => {
    const v = num(it.best_score);
    const r = num(it.round) ?? i;
    if (v !== null && v > running) running = v;
    if (running !== -Infinity) out.push({ round: r, value: running });
  });
  return out;
}

/** Round index at which the best `best_score` was first reached. */
export function computeRoundsToBest(iters: EstTraceIteration[]): number | null {
  let best = -Infinity;
  let bestRound: number | null = null;
  iters.forEach((it, i) => {
    const v = num(it.best_score);
    if (v === null) return;
    if (v > best) {
      best = v;
      bestRound = num(it.round) ?? i;
    }
  });
  return bestRound;
}

/**
 * Relative gap between the best score ever observed and the score at the last
 * recorded round: `(best - last) / |best|`. Returns `null` if either value is
 * missing or if `best === 0` (gap undefined).
 */
export function computeFinalGapToBest(iters: EstTraceIteration[]): number | null {
  let best = -Infinity;
  let last: number | null = null;
  for (const it of iters) {
    const v = num(it.best_score);
    if (v === null) continue;
    if (v > best) best = v;
    last = v;
  }
  if (last === null || !Number.isFinite(best) || best === 0) return null;
  return (best - last) / Math.abs(best);
}

/** Slope of best-so-far vs round (simple linear regression). `null` if <2 points. */
export function computeImprovementRate(iters: EstTraceIteration[]): number | null {
  const pts = bestSoFar(iters);
  if (pts.length < 2) return null;
  const n = pts.length;
  let sx = 0;
  let sy = 0;
  let sxy = 0;
  let sxx = 0;
  for (const { round, value } of pts) {
    sx += round;
    sy += value;
    sxy += round * value;
    sxx += round * round;
  }
  const denom = n * sxx - sx * sx;
  if (denom === 0) return null;
  return (n * sxy - sx * sy) / denom;
}

/** `best_so_far / max(best_so_far)` per round. Empty when no valid scores. */
export function computeNormalizedTrajectory(
  iters: EstTraceIteration[],
): { rounds: number[]; normalized: number[] } {
  const pts = bestSoFar(iters);
  if (pts.length === 0) return { rounds: [], normalized: [] };
  const max = pts[pts.length - 1].value;
  if (!Number.isFinite(max) || max === 0) return { rounds: [], normalized: [] };
  return {
    rounds: pts.map((p) => p.round),
    normalized: pts.map((p) => p.value / max),
  };
}

/** `std(beam_scores)` per round. Rounds with <2 finite beams emit `null`. */
export function computeDiversityTrajectory(
  iters: EstTraceIteration[],
): { rounds: number[]; std: Array<number | null> } {
  const rounds: number[] = [];
  const std: Array<number | null> = [];
  iters.forEach((it, i) => {
    rounds.push(num(it.round) ?? i);
    const beams = (it.beam_scores ?? []).filter((b): b is number => Number.isFinite(b));
    if (beams.length < 2) {
      std.push(null);
      return;
    }
    const mean = beams.reduce((a, b) => a + b, 0) / beams.length;
    const variance = beams.reduce((a, b) => a + (b - mean) ** 2, 0) / beams.length;
    std.push(Math.sqrt(variance));
  });
  return { rounds, std };
}
