/**
 * Shared EST help-popover content for the algorithm-analysis tabs.
 */
import type { HelpContent } from '@/components/charts';

export const EST_FILTER_HELP: HelpContent = {
  title: 'Configuration filters',
  summary:
    'Restrict which EST runs feed the charts and tables on this tab by ranging over the algorithm configuration knobs.',
  bullets: [
    'Each slider corresponds to a numeric configuration field (e.g. e, k, b).',
    'Runs whose value falls outside the selected range disappear from every panel below.',
    'Sliders auto-hide when a knob has only a single value across the loaded runs.',
    'Lasso/box-select on a chart (or row checkbox) further narrows the view to a "focus set"; clear it from the badge above the toolbar.',
    'Selections persist across tabs and reloads via the URL — share the link to share your view.',
    'Charts can be downloaded as PNG or SVG from the chart header; tables export to CSV via the same header bar.',
  ],
};

export const SWEEP_LINE_HELP: HelpContent = {
  title: 'Metric vs swept axis',
  summary:
    'Line chart showing how the chosen outcome metric varies with one of the EST knobs while grouping runs by the remaining knobs.',
  bullets: [
    'Use the X-axis radios to pick which knob varies along the horizontal axis.',
    'Each line is one combination of the other two knobs; the legend lists them.',
    'Disconnected gaps mean a run is missing for that combination.',
  ],
};

export const SWEEP_3D_HELP: HelpContent = {
  title: '3D parameter space',
  summary:
    'Spatial view of every loaded run in (e, k, b) space, coloured by the active metric.',
  bullets: [
    'Drag to rotate; scroll to zoom. The colorbar maps the metric to colour.',
    'Labels show the schedule name; sparse regions indicate missing runs.',
    'Use the filter sliders to focus on a slice of the cube.',
  ],
};

export const SWEEP_TABLE_HELP: HelpContent = {
  title: 'Sweep summary',
  summary: 'Tabular view of every loaded run with its config and key outcome metrics.',
  bullets: [
    'Sortable columns make it easy to find the best/worst run for any metric.',
    'A row reads "…" when its insights are still loading.',
  ],
};

export const SENSITIVITY_3D_HELP: HelpContent = {
  title: 'Configuration cube',
  summary:
    'Maps three numeric configuration dimensions to the X/Y/Z axes; colour encodes the chosen metric.',
  bullets: [
    'Pick which dimensions land on each axis using the X/Y/Z selectors above.',
    'Choose a categorical "Facet by" dimension to render one cube per category value.',
    'Click a marker to toggle it in/out of the focus set (lasso doesn\u2019t work in 3D).',
    'Use the metric radios to switch the colour scale.',
  ],
};

export const SENSITIVITY_2D_HELP: HelpContent = {
  title: '2D scatter',
  summary:
    'Single-axis view of the metric versus the chosen X dimension.',
  bullets: [
    'Quick check of the marginal trend; complements the 3D cube above.',
    'Lasso or box-select markers to define a focus set shared with every chart and table on this tab.',
    'Hover a marker to read its schedule name and exact metric value.',
  ],
};

export const SENSITIVITY_PARCOORDS_HELP: HelpContent = {
  title: 'Parallel coordinates',
  summary:
    'Each polyline is a run; axes are the numeric configuration knobs followed by the chosen metric.',
  bullets: [
    'Drag the small handle on any axis to brush a value range.',
    'Lines change colour with the metric value; bright = high.',
    'Useful for narrowing down the configuration patterns that lead to top scores.',
  ],
};

export const PARETO_HELP: HelpContent = {
  title: 'Pareto front',
  summary:
    'Configurable multi-objective scatter with non-dominated runs highlighted. Pick the axes (and optionally a third dimension) to explore trade-offs between any pair/triple of metrics.',
  bullets: [
    'Switch between 2D and 3D using the toolbar, then choose any metric on each axis.',
    'Green markers form the Pareto front: no other run beats them on every chosen metric, respecting each metric\u2019s direction (max/min).',
    'Grey markers are dominated by at least one green run.',
    'Use "Color by" to colour the front by a configuration knob (e, k, b, \u2026); the trend that lights the front reveals which configuration drives the win.',
    'Lasso (2D) or click-toggle (3D) sets a focus set that filters the table and equivalent-run views below.',
    'Toggle "Scalarize" to weigh each active metric and recolour points by Σ wᵢ·norm(metricᵢ); the underlying CSV gains a "scalar" column.',
    'When "Collapse equivalents" is on, runs producing identical scheduled-task sets are folded into one representative annotated with a count badge.',
  ],
};

export const INTERNALS_FOM_HELP: HelpContent = {
  title: 'Score trajectory',
  summary:
    'Per-iteration best, median and worst beam score for every traced run.',
  bullets: [
    'Solid line = best beam; dotted = median; dashed = worst.',
    'Convergence shows up as the three lines collapsing onto each other.',
    'Use the legend to focus on a single run when many overlap.',
    'See also the Convergence summary, Normalized trajectory, and Diversity charts below for at-a-glance comparisons across runs.',
    'Selections persist across tabs and reloads via the URL — share the link to share your view.',
  ],
};

export const INTERNALS_CONVERGENCE_HELP: HelpContent = {
  title: 'Convergence summary',
  summary:
    'Per-run scalar summary of how quickly the trajectory settled: rounds-to-best, plateau length, and final improvement margin.',
  bullets: [
    'Bars/columns are one per traced run; shorter bars indicate faster convergence.',
    'Pair with the Score trajectory above to confirm whether early convergence was a true optimum or a premature plateau.',
    'Selections persist across tabs and reloads via the URL — share the link to share your view.',
  ],
};

export const INTERNALS_NORMALIZED_HELP: HelpContent = {
  title: 'Normalized trajectory',
  summary:
    'Same per-iteration best score as the Score trajectory, but each run is rescaled to [0, 1] over its own (min, max).',
  bullets: [
    'Removes absolute-magnitude differences so curve shape is directly comparable across runs.',
    'A run that climbs to 1.0 early indicates fast relative convergence; a long flat tail means stagnation.',
    'Selections persist across tabs and reloads via the URL — share the link to share your view.',
  ],
};

export const INTERNALS_DIVERSITY_HELP: HelpContent = {
  title: 'Diversity',
  summary:
    'Per-iteration spread of the beam pool — how different the candidate solutions are from each other within each round.',
  bullets: [
    'Higher values mean the search is still exploring; collapsing diversity signals exploitation/convergence.',
    'A premature drop alongside a flat best-score curve is a classic premature-convergence signature.',
    'Selections persist across tabs and reloads via the URL — share the link to share your view.',
  ],
};

export const INTERNALS_HEATMAP_HELP: HelpContent = {
  title: 'Beam-score distribution',
  summary:
    'Heatmap of the full beam pool per round, sorted from best (bottom) to worst (top), for the first traced run.',
  bullets: [
    'Brighter colours indicate higher scores; horizontal bands signal stagnation.',
    'A widening colour spread per round means EST is still exploring.',
  ],
};

export const INTERNALS_WALL_HELP: HelpContent = {
  title: 'Wall time per round',
  summary: 'How long each EST round took, useful for spotting slow phases.',
  bullets: [
    'Spikes typically correlate with rounds that explore many candidates.',
    'A steady downward trend indicates the beam is shrinking effectively.',
  ],
};

export const STATISTICS_HELP: HelpContent = {
  title: 'Statistics report',
  summary:
    'Per-metric summary statistics (mean / std / min / max / best run) plus Pearson correlations against each numeric configuration knob.',
  bullets: [
    'Strong positive correlation (|r| ≥ 0.6) means turning that knob up reliably moves the metric.',
    'Negative correlation tells you the opposite direction helps.',
    'Use the filter sliders to compute statistics over a sub-range of configurations.',
  ],
};

export const OVERVIEW_HELP: HelpContent = {
  title: 'Run inventory',
  summary: 'One row per loaded EST run with its algorithm config and outcome metrics.',
  bullets: [
    'The metric cards above summarise the best run for each headline KPI.',
    'Rows show "…" while their insights are still loading.',
    'Use the filter sliders to narrow down which runs are summarised.',
    'Tick a row\u2019s checkbox to add/remove it from the focus set; the metric cards update accordingly.',
    'Use "Run inventory" in the panel header to download the visible table as CSV.',
  ],
};
