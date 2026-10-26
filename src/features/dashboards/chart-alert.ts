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
