import type { EChartsOption } from "echarts";
import { binLabel } from "@/lib/chart-bins";
import { DEFAULT_PALETTE } from "@/lib/chart-palettes";
import { fillRanges, MAX_BULLETS, MAX_DRAWN_RANGES } from "@/lib/type-b";

/**
 * The ECharts options of the histogram and the bullet graph (`BI-16` part B,
 * decision 5). Pure functions of the rows a tile got, so a test can pin every
 * mark; the tile body decides which colours and formats to pass.
 */

type Row = Record<string, unknown>;

const num = (v: unknown): number | null => {
  if (v === undefined || v === null || v === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
};

const theme = (dark: boolean) => ({
  axis: dark ? "#a1a1aa" : "#71717a",
  split: dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.06)",
  tooltipBg: dark ? "#18181b" : "#ffffff",
  tooltipText: dark ? "#e4e4e7" : "#18181b",
  separator: dark ? "#09090b" : "#ffffff",
});

// ── histogram ────────────────────────────────────────────────────────────

export type HistogramInput = {
  rows: readonly Row[];
  /** The binned column (each row holds a range's lower edge) and the count column. */
  column: string;
  countColumn: string;
  /** The ranges' width when the server read it; without it the edges are shown as they are and nothing is filled in. */
  width?: number;
  dark: boolean;
  /** Formats a count in the tooltip, the axis and the data labels. */
  format: (v: number) => string;
  /** Formats a lower edge when the width is unknown. */
  formatEdge: (v: number) => string;
  color?: string;
  showLabels?: boolean;
};

export type HistogramDrawn = { option: EChartsOption; ranges: number } | { message: string };

export function histogramOption(i: HistogramInput): HistogramDrawn {
  const counted = i.rows
    .map((r) => ({ lower: num(r[i.column]), count: num(r[i.countColumn]) ?? 0 }))
    .filter((r): r is { lower: number; count: number } => r.lower !== null);
  if (!counted.length) return { message: "No data" };
  const filled = i.width ? fillRanges(counted, i.width) : [...counted].sort((a, b) => a.lower - b.lower);
  if (filled === null) {
    return { message: `Too many ranges to draw at this width (more than ${MAX_DRAWN_RANGES}); choose a wider width` };
  }
  const t = theme(i.dark);
  const labels = filled.map((r) => (i.width ? binLabel(r.lower, i.width) : i.formatEdge(r.lower)));
  const color = i.color ?? DEFAULT_PALETTE[0];
  const option: EChartsOption = {
    color: [color],
    textStyle: { color: t.axis, fontFamily: "inherit" },
    grid: { left: 8, right: 16, top: 16, bottom: 8, containLabel: true },
    tooltip: {
      trigger: "axis", axisPointer: { type: "shadow" },
      backgroundColor: t.tooltipBg, borderWidth: 0, textStyle: { color: t.tooltipText, fontSize: 12 },
      valueFormatter: (v) => (typeof v === "number" ? i.format(v) : String(v)),
    },
    xAxis: {
      type: "category", data: labels,
      axisLabel: { color: t.axis, fontSize: 11, hideOverlap: true, rotate: labels.length > 8 ? 28 : 0 },
      axisLine: { lineStyle: { color: t.split } }, axisTick: { show: false },
    },
    yAxis: {
      type: "value", minInterval: 1,
      axisLabel: { color: t.axis, fontSize: 11, formatter: (v: number) => i.format(v) },
      splitLine: { lineStyle: { color: t.split } }, axisLine: { show: false }, axisTick: { show: false },
    },
    series: [{
      type: "bar", name: "Rows", data: filled.map((r) => r.count),
      // Touching bars: a range is the width of its own bar, so the shape of the spread is the shape drawn.
      barCategoryGap: 0,
      itemStyle: { color, borderColor: t.separator, borderWidth: 1 },
      label: { show: !!i.showLabels, position: "top", fontSize: 10, color: t.axis, formatter: (p: { value?: unknown }) => i.format(Number(p.value ?? 0)) },
    }],
  };
  return { option, ranges: filled.length };
}

// ── bullet ───────────────────────────────────────────────────────────────

/** Three greys, light to dark, for the bands; lighter in the dark theme so the bar stays the strongest mark. */
export const BAND_GREYS = {
  light: ["#f4f4f5", "#e4e4e7", "#d4d4d8"],
  dark: ["#3f3f46", "#52525b", "#71717a"],
} as const;

export type BulletInput = {
  rows: readonly Row[];
  /** The category column; absent for one bullet (the rows' only row is the value). */
  category?: string;
  /** The result columns of the value and of a goal that is a value. */
  valueColumn: string;
  goalColumn?: string;
  /** A fixed goal. */
  goal?: number;
  /** Ascending upper limits of the bands. */
  bands: readonly number[];
  /** What the single bullet is called. */
  singleLabel: string;
  dark: boolean;
  format: (v: number) => string;
  axisFormat: (v: number) => string;
  color?: string;
};

export type BulletDrawn = { option: EChartsOption; shown: number; more: number } | { message: string };

export function bulletOption(i: BulletInput): BulletDrawn {
  const all = i.rows.map((r) => ({
    name: i.category ? String(r[i.category] ?? "") : i.singleLabel,
    value: num(r[i.valueColumn]),
    goal: i.goalColumn ? num(r[i.goalColumn]) : null,
  }));
  if (!all.length) return { message: "No data" };
  const shownRows = all.slice(0, MAX_BULLETS);
  const more = Math.max(0, all.length - MAX_BULLETS);
  const t = theme(i.dark);
  const greys = i.dark ? BAND_GREYS.dark : BAND_GREYS.light;
  const color = i.color ?? DEFAULT_PALETTE[0];
  const values = shownRows.map((r) => r.value ?? 0);
  const goals = [i.goal, ...shownRows.map((r) => r.goal)].filter((g): g is number => typeof g === "number");
  const top = Math.max(0, ...values, ...goals, ...i.bands);
  const bottom = Math.min(0, ...values, ...goals, ...i.bands);
  // Bands run from the axis' start to each limit; a band is only drawn when its limit is above the one before.
  const edges = [bottom, ...i.bands];
  const markAreas = i.bands.map((limit, k) => [
    { xAxis: edges[k], itemStyle: { color: greys[k] ?? greys[greys.length - 1] } },
    { xAxis: limit },
  ]);
  const target = "#18181b";
  const targetColor = i.dark ? "#fafafa" : target;
  const series: NonNullable<EChartsOption["series"]> = [{
    type: "bar", name: "Value", data: shownRows.map((r) => r.value), z: 3,
    barWidth: "34%", itemStyle: { color },
    // The bands behind every bar, and a fixed target as one line across them.
    ...(markAreas.length ? { markArea: { silent: true, z: 1, data: markAreas as never } } : {}),
    ...(typeof i.goal === "number"
      ? { markLine: { silent: true, symbol: "none", z: 5, label: { show: false }, lineStyle: { color: targetColor, width: 3, type: "solid" as const }, data: [{ xAxis: i.goal }] } }
      : {}),
  }];
  // A goal that is a value differs per row, which a mark line cannot show: one tick per row.
  if (i.goalColumn) {
    series.push({
      type: "scatter", name: "Target", z: 5, symbol: "rect", symbolSize: [3, 26], itemStyle: { color: targetColor },
      data: shownRows.flatMap((r, k) => (r.goal === null ? [] : [[r.goal, k]])),
    });
  }
  const option: EChartsOption = {
    textStyle: { color: t.axis, fontFamily: "inherit" },
    grid: { left: 8, right: 24, top: 8, bottom: 8, containLabel: true },
    tooltip: {
      trigger: "axis", axisPointer: { type: "shadow" },
      backgroundColor: t.tooltipBg, borderWidth: 0, textStyle: { color: t.tooltipText, fontSize: 12 },
      valueFormatter: (v) => (typeof v === "number" ? i.format(v) : String(v)),
    },
    xAxis: {
      type: "value", min: bottom, max: top > 0 ? top * 1.05 : undefined,
      axisLabel: { color: t.axis, fontSize: 11, formatter: (v: number) => i.axisFormat(v) },
      splitLine: { show: false }, axisLine: { show: false }, axisTick: { show: false },
    },
    yAxis: {
      type: "category", inverse: true, data: shownRows.map((r) => r.name),
      axisLabel: { color: t.axis, fontSize: 11, width: 110, overflow: "truncate" },
      axisLine: { lineStyle: { color: t.split } }, axisTick: { show: false },
    },
    series,
  };
  return { option, shown: shownRows.length, more };
}
