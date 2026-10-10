/**
 * BI-16A review fix (BLOCKER) R2: the render spec the preview route returns
 * carries no `def`, but a raw table's column settings, a pivot's fields and a
 * KPI's comparison are read from the definition (a saved tile gets it from the
 * dashboard payload). The builder attaches the payload it just sent, so the
 * preview renders through the same component and settings as the saved tile.
 */
export function withPreviewDef<S extends object>(spec: S, payload: Record<string, unknown>): S & { def: Record<string, unknown> } {
  return { ...spec, def: payload }
}
