/**
 * What number a chart gives an alert (`BI-19` part C, feature decision C1),
 * mirroring `lakehouse_alerts::number_shape`: a KPI or a gauge gives its value,
 * a line, area, bar or combo chart gives the latest point of one series, and any
 * other chart can only be asked "returns any rows".
 */
export type AlertNumberShape = "single" | "series" | "none"

export function alertNumberShape(kind: string): AlertNumberShape {
  if (kind === "kpi" || kind === "gauge") return "single"
  if (kind === "line" || kind === "area" || kind === "bar" || kind === "combo") return "series"
  return "none"
}

/**
 * Whether a series chart has a latest point to compare, mirroring the server's
 * save-time check (`check_chart_has_a_number` in `lakehouse-alerts`, which
 * refuses unless `ChartDef::effective_order()` is "none"): a sort by the
 * dimension is "none"; a sort by anything else is a value order; with no
 * sort, the legacy `order`, and an absent `order` is "none". Not
 * `effectiveOrder` of `chart-shape`: that one reads an absent `order` as
 * "desc" for the builder's sort control, which would refuse charts the
 * server accepts (`BI-19` owner QA 2026-10-28, QA-6). The server stays the
 * authority; this only keeps the dialog from offering what it will refuse.
 */
export function chartHasLatestPoint(
  def: { dimension?: string; order?: string; sort?: { by: string } | null } | undefined,
): boolean {
  if (!def) return true
  if (def.sort) return def.sort.by === def.dimension
  return (def.order ?? "none") === "none"
}
