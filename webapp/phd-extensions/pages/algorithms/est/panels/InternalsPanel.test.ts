import { describe, expect, it } from 'vitest';
import {
  computeDiversityTrajectory,
  computeFinalGapToBest,
  computeImprovementRate,
  computeNormalizedTrajectory,
  computeRoundsToBest,
} from './InternalsPanel.helpers';
import type { EstTraceIteration } from '../useRunMatrix';

function it_(round: number, best: number | null, beams: number[] = []): EstTraceIteration {
  return { round, beam_scores: beams, best_score: best };
}

describe('computeRoundsToBest', () => {
  it('returns the round at which the best score first appears', () => {
    const iters = [it_(0, 1), it_(1, 5), it_(2, 5), it_(3, 4)];
    expect(computeRoundsToBest(iters)).toBe(1);
  });

  it('returns null when no scores are finite', () => {
    expect(computeRoundsToBest([it_(0, null), it_(1, null)])).toBeNull();
  });
});

describe('computeFinalGapToBest', () => {
  it('computes (best - last) / |best|', () => {
    const iters = [it_(0, 1), it_(1, 10), it_(2, 7)];
    expect(computeFinalGapToBest(iters)).toBeCloseTo(0.3, 6);
  });

  it('returns 0 when the run ends at the best score', () => {
    expect(computeFinalGapToBest([it_(0, 2), it_(1, 5)])).toBe(0);
  });

  it('returns null when best is zero or no data', () => {
    expect(computeFinalGapToBest([])).toBeNull();
    expect(computeFinalGapToBest([it_(0, 0), it_(1, 0)])).toBeNull();
  });
});

describe('computeImprovementRate', () => {
  it('returns slope of best-so-far vs round', () => {
    // best-so-far = [1, 2, 3, 4] vs rounds [0,1,2,3] → slope 1
    const iters = [it_(0, 1), it_(1, 2), it_(2, 3), it_(3, 4)];
    expect(computeImprovementRate(iters)).toBeCloseTo(1, 6);
  });

  it('is flat (slope 0) when scores never improve', () => {
    const iters = [it_(0, 5), it_(1, 4), it_(2, 3)];
    expect(computeImprovementRate(iters)).toBeCloseTo(0, 6);
  });

  it('returns null with fewer than 2 valid points', () => {
    expect(computeImprovementRate([it_(0, 1)])).toBeNull();
  });
});

describe('computeNormalizedTrajectory', () => {
  it('scales best-so-far so the max is 1', () => {
    const out = computeNormalizedTrajectory([it_(0, 2), it_(1, 4), it_(2, 8)]);
    expect(out.rounds).toEqual([0, 1, 2]);
    expect(out.normalized).toEqual([0.25, 0.5, 1]);
  });

  it('returns empty arrays when no scores are present', () => {
    expect(computeNormalizedTrajectory([])).toEqual({ rounds: [], normalized: [] });
  });
});

describe('computeDiversityTrajectory', () => {
  it('emits population std per round', () => {
    const out = computeDiversityTrajectory([it_(0, 1, [1, 1, 1]), it_(1, 2, [0, 2, 4])]);
    expect(out.rounds).toEqual([0, 1]);
    expect(out.std[0]).toBeCloseTo(0, 6);
    // mean=2, variance = ((4)+(0)+(4))/3 = 8/3 → std = sqrt(8/3)
    expect(out.std[1]).toBeCloseTo(Math.sqrt(8 / 3), 6);
  });

  it('emits null for rounds with fewer than 2 finite beams', () => {
    const out = computeDiversityTrajectory([it_(0, 1, []), it_(1, 1, [3])]);
    expect(out.std).toEqual([null, null]);
  });
});
