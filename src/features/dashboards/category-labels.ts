import type { EChartsOption } from "echarts";

/**
 * BI-73 (a, b): every category label of a vertical bar, line, combo or box
 * chart stays readable or reachable on a narrow tile. The builders draw the
 * axis unrotated (`categoryAxisLabel`); the chart wrapper, which knows the
 * tile's width, calls `fitCategoryLabels`: labels that would collide are
 * rotated (about 30°) before any is hidden, and a label too long even for
 * that is cut with an ellipsis at a fixed width (the tooltip names it in
 * full). `hideOverlap` stays as the last resort for very many categories on
 * a very narrow tile. A rotated label reaches past the axis to the left, so
 * the grid keeps room for it.
 */

/** The label settings every category axis starts from (the wrapper decides the rotation). */
export function categoryAxisLabel(color: string, fontSize = 11) {
  return { color, fontSize, interval: 0, hideOverlap: true, rotate: 0 };
}

/** Rotation applied to colliding labels, in degrees. */
export const LABEL_ROTATION = 30;
/** The widest a rotated label is drawn before it ends in an ellipsis, in px. */
export const LABEL_MAX_WIDTH = 96;
/** What the value axis, its labels and the grid padding take from the tile's width, in px. */
const SIDE_ROOM = 56;
/** A rough width of one character at the axis font size, in px. */
const CHAR_WIDTH = 6.4;

type Loose = Record<string, unknown>;
const isObj = (v: unknown): v is Loose => typeof v === "object" && v !== null && !Array.isArray(v);

/**
 * `option` with its category x axis fitted to `width` (the tile's width in
 * px). The same object comes back when nothing needs to change: another kind
 * of axis, a label rule of its own (monthly years, a rotation the author
 * chose), or labels that already fit.
 */
export function fitCategoryLabels(option: EChartsOption, width: number): EChartsOption {
  const xAxis = (option as Loose).xAxis;
  if (!(width > 0) || !isObj(xAxis) || xAxis.type !== "category" || !Array.isArray(xAxis.data)) return option;
  const label = xAxis.axisLabel;
  // Only an axis the builder drew with the plain rule: every label, no rotation chosen.
  if (!isObj(label) || label.interval !== 0 || (label.rotate ?? 0) !== 0 || label.width !== undefined) return option;
  const names = (xAxis.data as unknown[]).map((d) => String(isObj(d) ? d.value ?? "" : d));
  if (names.length < 2) return option;
  const slot = Math.max(1, width - SIDE_ROOM) / names.length;
  const widest = Math.max(...names.map((n) => n.length * CHAR_WIDTH));
  if (widest <= slot - 6) return option;
  const drawn = Math.min(widest, LABEL_MAX_WIDTH);
  const cut = widest > LABEL_MAX_WIDTH;
  // The first rotated label ends at its category and runs left, past the axis.
  const reach = Math.max(0, Math.round(drawn * Math.cos((LABEL_ROTATION * Math.PI) / 180) - slot / 2));
  const grid = isObj((option as Loose).grid) ? ((option as Loose).grid as Loose) : {};
  const left = typeof grid.left === "number" ? grid.left : 8;
  const next = {
    ...(option as Loose),
    xAxis: { ...xAxis, axisLabel: { ...label, rotate: LABEL_ROTATION, ...(cut ? { width: LABEL_MAX_WIDTH, overflow: "truncate" as const } : {}) } },
    grid: { ...grid, containLabel: true, left: left + reach },
  };
  return next as EChartsOption;
}

/** What `fitCategoryLabels` decided, to compare two fits without comparing options. */
export function fitSignature(option: EChartsOption): string {
  const o = option as Loose;
  const label = isObj(o.xAxis) && isObj(o.xAxis.axisLabel) ? o.xAxis.axisLabel : {};
  const grid = isObj(o.grid) ? o.grid : {};
  return `${String(label.rotate ?? "")}|${String(label.width ?? "")}|${String(grid.left ?? "")}`;
}
