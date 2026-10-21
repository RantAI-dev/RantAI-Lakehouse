import { displayColumn } from "@/lib/chart-joins";
import type { EChartsOption } from "echarts";
import { decimalsFor, monthlyAxisLabel } from "@/lib/chart-axis";
import { resolvePalette } from "@/lib/chart-palettes";
import type { StyleFields } from "@/lib/chart-style";
import { formatAxisValue, formatValue } from "@/lib/value-format";
import { tileMeasureLabels, type MeasureEntry } from "@/lib/chart-shape";
import type { LayerAxis, LayerDraw } from "@/lib/chart-layers";
import { bucketLabel, type Grain } from "@/lib/time-grain";
import { LEGEND_TEXT, showsPointMarkers } from "./chart-option";

/**
 * A chart with more questions on it (`BI-13`): the base chart's series and the
 * series of each layer on one category axis, with a right value axis only when
 * a series uses it. The server answers each question on its own and the
 * console lines them up here, on the text of the dimension value; a value a
 * question lacks is a gap (`null`), never a zero.
 *
 * Each value axis formats its numbers from its own series (`decimalsFor`), so
 * visitors in the thousands and a rate with decimals do not share a format.
 */

type Row = Record<string, unknown>;

/** One layer's answer inside a tile, as `routes::layers` sends it. */
export type LayerCell = {
  label: string;
  draw: LayerDraw;
  axis: LayerAxis;
  /** The mart, or the SQL source's id. */
  source: string;
  /** The column the rows are matched on. */
  dimension?: string;
  columns: string[];
  rows: Row[];
  error?: string;
  errorId?: string;
  truncated?: boolean;
};

export type CombinedInput = {
  kind: string;
  /** The base chart's category column and its measure columns. */
  x: string;
  y: string | string[];
  /** Readable names for base result columns (`avg_stay` → "avg of stay"). */
  measureLabels?: Record<string, string>;
  /** BI-2: the label of each metric by name, from the tile payload. */
  metricLabels?: Record<string, string>;
  rows: Row[];
  layers: readonly LayerCell[];
  /** The layers' definitions, to name their measures. */
  layerMeasures?: readonly (readonly MeasureEntry[] | undefined)[];
  baseDraw?: LayerDraw;
  baseAxis?: LayerAxis;
  /** The grain the buckets were cut with, to label them. */
  grain: Grain | null;
  /** BI-17: the chart's formats and palette, the number locale and the company palette. */
  style?: StyleFields;
  locale?: string;
  companyPalette?: readonly string[];
};

export type Combined = {
  option: EChartsOption;
  /** The category of each position, as the server sent it (the stored value or bucket). */
  keys: string[];
  /** The label drawn for each position. */
  labels: string[];
  /** The owner of each series by index: -1 the chart's own, otherwise the layer's index. */
  owners: number[];
};

const text = (v: unknown) => String(v ?? "");
const num = (v: unknown) => Number(v ?? 0);
const ISO_DATE = /^\d{4}-\d{2}-\d{2}/;

/** A layer that answered with rows. */
const answered = (l: LayerCell) => !l.error && l.rows.length > 0;
const dimensionOf = (l: LayerCell) => l.dimension ?? l.columns[0] ?? "";

/** The layers that failed, with the sentence the tile lists for each. */
export function failedLayers(layers: readonly LayerCell[] | undefined): { label: string; error: string; errorId?: string }[] {
  return (layers ?? []).filter((l) => l.error).map((l) => ({ label: l.label, error: l.error ?? "", errorId: l.errorId }));
}

/** The labels of the layers that were cut off (BI-13 review fix (SHOULD-FIX) R1): the tile says so under the chart. */
export function cutLayers(layers: readonly LayerCell[] | undefined): string[] {
  return (layers ?? []).filter((l) => !l.error && l.truncated).map((l) => l.label);
}

/**
 * The categories of the chart: the base's, then those only a layer has. Dates
 * (buckets or days) read best in date order, so they are sorted; anything else
 * keeps the base's order and appends the new values after it.
 */
export function categoryKeys(baseRows: readonly Row[], x: string, layers: readonly LayerCell[]): string[] {
  const seen = new Set<string>();
  const keys: string[] = [];
  const add = (k: string) => { if (!seen.has(k)) { seen.add(k); keys.push(k); } };
  for (const r of baseRows) add(text(r[x]));
  for (const l of layers) if (answered(l)) for (const r of l.rows) add(text(r[dimensionOf(l)]));
  return keys.length && keys.every((k) => ISO_DATE.test(k)) ? [...keys].sort() : keys;
}

type Series = { name: string; type: LayerDraw; axis: LayerAxis; values: (number | null)[]; owner: number; area: boolean };

function valuesOf(rows: readonly Row[], x: string, column: string, keys: readonly string[]): (number | null)[] {
  const byKey = new Map<string, number>();
  for (const r of rows) byKey.set(text(r[x]), num(r[column]));
  return keys.map((k) => byKey.get(k) ?? null);
}

export function buildCombinedOption(input: CombinedInput, dark: boolean): Combined {
  const { x, rows, layers } = input;
  const keys = categoryKeys(rows, x, layers);
  const labels = keys.map((k) => (input.grain ? bucketLabel(input.grain, k) : k));
  const baseCols = Array.isArray(input.y) ? input.y : [input.y];
  const locale = input.locale ?? "en-US";
  const baseLabel = (c: string) => input.measureLabels?.[c] ?? displayColumn(c);

  const series: Series[] = baseCols.map((col, i) => ({
    name: baseLabel(col),
    // A combo keeps its first measure as bars on the left and the rest as lines on the right unless the chart says otherwise.
    type: input.baseDraw ?? (input.kind === "combo" ? (i === 0 ? "bar" : "line") : input.kind === "bar" ? "bar" : "line"),
    axis: input.baseAxis ?? (input.kind === "combo" && i > 0 ? "right" : "left"),
    values: valuesOf(rows, x, col, keys),
    owner: -1,
    area: input.kind === "area",
  }));
  layers.forEach((l, index) => {
    if (!answered(l)) return;
    const dim = dimensionOf(l);
    const cols = l.columns.filter((c) => c !== dim && c !== "__other");
    const labelsOf = tileMeasureLabels(input.layerMeasures?.[index], cols, undefined, input.metricLabels);
    for (const col of cols) {
      series.push({
        name: cols.length === 1 ? l.label : `${l.label} · ${labelsOf[col] ?? displayColumn(col)}`,
        type: l.draw, axis: l.axis, values: valuesOf(l.rows, dim, col, keys), owner: index, area: false,
      });
    }
  });

  const ink = dark ? "#a1a1aa" : "#71717a";
  const split = dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.06)";
  const sides: LayerAxis[] = [];
  if (series.some((s) => s.axis === "left") || !series.some((s) => s.axis === "right")) sides.push("left");
  if (series.some((s) => s.axis === "right")) sides.push("right");
  // Each axis formats from its own series.
  const decimalsOf = (side: LayerAxis) => decimalsFor(series.filter((s) => s.axis === side).flatMap((s) => s.values.filter((v): v is number => v !== null)));
  const decimals: Record<LayerAxis, number> = { left: decimalsOf("left"), right: decimalsOf("right") };

  const option = {
    color: [...resolvePalette(input.style?.colors, input.companyPalette)],
    textStyle: { color: ink, fontFamily: "inherit" },
    tooltip: {
      backgroundColor: dark ? "#18181b" : "#ffffff", borderWidth: 0,
      textStyle: { color: dark ? "#e4e4e7" : "#18181b", fontSize: 12 },
      trigger: "axis", axisPointer: { type: "cross" },
    },
    legend: { top: 0, type: "scroll", textStyle: { color: ink, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
    grid: { left: 8, right: 16, top: 34, bottom: 8, containLabel: true },
    xAxis: {
      type: "category", data: labels,
      axisLabel: { color: ink, fontSize: 11, interval: 0, hideOverlap: true, rotate: labels.length > 6 ? 28 : 0, ...monthlyAxisLabel(labels) },
      axisLine: { lineStyle: { color: split } }, axisTick: { show: false },
    },
    yAxis: sides.map((side) => ({
      type: "value",
      position: side,
      axisLabel: { color: ink, fontSize: 11, formatter: (v: number) => formatAxisValue(v, input.style?.format, locale, decimals[side]) },
      splitLine: { show: side === sides[0], lineStyle: { color: split } },
      axisLine: { show: false }, axisTick: { show: false },
      // The two axes have different scales (visitors against a rate).
      alignTicks: false,
    })),
    series: series.map((s) => ({
      name: s.name,
      type: s.type,
      yAxisIndex: sides.indexOf(s.axis),
      data: s.values,
      tooltip: { valueFormatter: (v: unknown) => (typeof v === "number" ? formatValue(v, input.style?.format, locale, decimals[s.axis]) : "–") },
      emphasis: { focus: "series" },
      ...(s.type === "bar"
        ? { barMaxWidth: 26, itemStyle: { borderRadius: [3, 3, 0, 0] } }
        : {
            smooth: false, showSymbol: showsPointMarkers("line", keys.length), symbolSize: 6, lineStyle: { width: 2 },
            ...(s.area ? { areaStyle: { opacity: 0.12 } } : {}),
          }),
    })),
  } as EChartsOption;
  return { option, keys, labels, owners: series.map((s) => s.owner) };
}
