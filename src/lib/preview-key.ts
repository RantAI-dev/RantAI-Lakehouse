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
}

export function previewKey(inputs: PreviewInputs): string {
  return JSON.stringify(inputs)
}
