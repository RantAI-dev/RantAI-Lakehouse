/**
 * Suggested title for a chart in the builder, from the columns it uses.
 *
 * The builder shows it as the Title placeholder and saves it when the Title
 * is left empty, so a chart never has to be named before it is understood.
 * It only names columns the user picked; nothing is invented.
 */
export type TitleParts = {
  kind: string;
  dimension?: string;
  measures?: string[];
  breakdown?: string;
};

export function suggestChartTitle({ kind, dimension, measures = [], breakdown }: TitleParts): string {
  const [m1, m2] = measures.filter(Boolean);
  if (kind === "text" || !m1) return "";
  if (kind === "kpi" || kind === "gauge") return m1;
  const metric = (kind === "scatter" || kind === "bubble") && m2 ? `${m1} vs ${m2}` : kind === "combo" && m2 ? `${m1} and ${m2}` : m1;
  // A point map has no category: its dimension is only a tooltip label.
  if (kind === "geoheat") return `${metric} density`;
  if (kind === "pointmap") return dimension ? `${metric} by ${dimension}` : `${metric} by location`;
  if (!dimension) return metric;
  if (kind === "sankey" && breakdown) return `${metric}: ${dimension} → ${breakdown}`;
  if (kind === "calendar") return `${metric} per day`;
  return breakdown ? `${metric} by ${dimension} and ${breakdown}` : `${metric} by ${dimension}`;
}
