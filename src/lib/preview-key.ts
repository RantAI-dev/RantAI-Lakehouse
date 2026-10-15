/**
 * BI-9 review fix (BLOCKER) R3: the chart builder's preview is requested again
 * whenever this key changes. It lists every field that reaches the generated
 * SQL or render spec; "Group by" was missing from the hand-written dependency
 * list, so changing it requested no new preview. Kept as one pure function so
 * a test can pin that each field, the grain included, changes the key.
 */
export type PreviewInputs = {
  title: string; source: string; kind: string; dimension: string
  measure: string; measure2: string; measure3: string; breakdown: string
  mapId: string; lat: string; lon: string; aggregate: string; span: number
  caption: string; target: string; text: string; order: string; limit: number
  targetBoard: string; grain: string
  /** The fields of raw tables, pivots and KPI comparisons as they would be sent (BI-16 part A). */
  tables: string
  /** The source's calculated fields as (name, formula), so editing a formula refreshes the preview (BI-8). */
  calc: string
  /**
   * BI-6: the measures with their own aggregate, the chart's own filters, the sort, "Other" and the
   * groupings as they would be sent. Each reaches the SQL, so each must request a new preview.
   */
  shape: string
  /** BI-11: the parameter values the preview is drawn with, the limit's parameter and the reference lines. */
  params: string
  /** BI-13: the chart's other questions and how its own series is drawn, as they would be sent. */
  layers: string
  /** BI-17 part A: the style as it would be sent (formats, colours, axes, labels, notes, tooltip, conditional colours). */
  style: string
}

export function previewKey(inputs: PreviewInputs): string {
  return JSON.stringify(inputs)
}
