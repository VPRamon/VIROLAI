/**
 * Re-exports the shared dimension primitives from the TSI features layer.
 *
 * Historically this file held an EST-specific copy.  The implementation has
 * been promoted to `@/features/schedules/analytics/dimensions` so that the
 * generic comparison views and the algorithm-analysis panels stay in lock
 * step.  This thin shim is kept so existing imports inside the EST package
 * continue to resolve without a wide rename.
 */
export {
  extractDimensions,
  readDimension,
  type Dimension,
  type DimensionKind,
  type DimensionSet,
} from '@/features/schedules/analytics/dimensions';
