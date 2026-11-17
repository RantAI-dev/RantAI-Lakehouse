import type { EChartsOption } from "echarts";
import { interpolate } from "@/lib/chart-palettes";
import { escapeHtml } from "@/lib/geo-points";
import type { AxisStyle, ResolvedReferenceLine, StyleFields, ValueFormat } from "@/lib/chart-style";
import { displayColumn } from "@/lib/chart-joins";
import { formatAxisValue, formatValue } from "@/lib/value-format";

/**
 * The part of a chart's style that is applied to an option after `buildOption`
 * has drawn it (`BI-17` part A, T5): axes, colours by series and by value, data
 * labels, notes and the tooltip's fields. The number formats and the palette
 * are applied inside `buildOption`, because its closures write every number.
 *
 * Why a pass and not more branches in `buildOption`: the same options apply to
 * a dozen chart kinds, and a pass reads the option the kind already built
 * (its axes, its series) instead of repeating each kind's layout. The cost is
 * that it must know which result column a series draws; `seriesColumns` is
 * that one rule.
 */

export type StyleInput = {
  kind: string;
  y: string | string[];
  /** Set when the chart has a breakdown: the series are the groups, all of one value. */
  breakdown?: boolean;
  style: StyleFields | undefined;
  locale: string;
  /** Decimals the `auto` style needs, read from the data (`decimalsFor`). */
  decimals: number;
  /**
   * The name each series is known by, when it is not a result column: a chart
   * with more questions on it (`BI-13`) draws series called after their question,
   * and a per-value format or a second-axis choice names them that way.
   */
  columns?: (string | undefined)[];
  /**
   * The reference lines the tile draws, so a line at or below zero on a
   * logarithmic axis is refused instead of vanishing
   * (`PR #199 review M SHOULD-FIX BI-17 1`).
   */
  referenceLines?: readonly ResolvedReferenceLine[];
  /**
   * The readable label of each value, by result name: the same labels the
   * legend and the tooltip use. The second axis is named after the values on
   * it when it has no title of its own (`PR #199 review M SHOULD-FIX BI-17 2`).
   */
  measureLabels?: Record<string, string>;
};

export type Styled = {
  option: EChartsOption;
  /** Set instead of a chart that would be drawn wrong (decision 8: a logarithmic axis with zero or negative values). */
  message?: string;
  /** Notes whose point is not in the result (decision 11): "1 note not shown: its point is not in the result". */
  hiddenNotes: number;
};

type Obj = Record<string, unknown>;

/** The result column each series of the option draws, in series order; `undefined` for a helper series. */
export function seriesColumns(input: Pick<StyleInput, "kind" | "y" | "breakdown">, count: number): (string | undefined)[] {
  const y = Array.isArray(input.y) ? input.y : [input.y];
  if (input.breakdown) return Array.from({ length: count }, () => y[0]);
  if (input.kind === "waterfall") return [undefined, y[0], y[0]].slice(0, count);
  if (input.kind === "combo") return [y[0], y[1] ?? y[0]].slice(0, count);
  return Array.from({ length: count }, (_, i) => (y.length > 1 ? y[i] : y[0]));
}

/** Whether anything here needs the post-pass. */
export function usesStylePass(style: StyleFields | undefined): boolean {
  if (!style) return false;
  const c = style.colors;
  return !!(style.axes || style.labels?.show || style.annotations?.length || style.tooltip?.fields.length || c?.byValue || (c?.series && Object.keys(c.series).length) || (style.valueFormats && Object.keys(style.valueFormats).length));
}

const asObj = (v: unknown): Obj => (v && typeof v === "object" ? (v as Obj) : {});

/** The number a data entry stands for, on the axis `index` of a scatter (0 x, 1 y) or its value otherwise. */
function entryNumber(d: unknown, scatter: boolean, index: number): number | null {
  const v = Array.isArray(d) ? d[scatter ? index : d.length - 1] : typeof d === "object" && d ? (d as Obj).value : d;
  const inner = Array.isArray(v) ? v[scatter ? index : v.length - 1] : v;
  return typeof inner === "number" && Number.isFinite(inner) ? inner : null;
}

/**
 * The numbers of one data entry that a logarithmic axis must not see at or
 * below zero.
 *
 * A box plot's entry is its five numbers (min, Q1, median, Q3, max): one of
 * them at or below zero is enough, and the check used to read only the last
 * one, so a box whose minimum was negative got a logarithmic axis
 * (`PR #199 review M SHOULD-FIX BI-17 1`). Any other entry is the value it
 * draws.
 */
function axisValues(series: Obj, d: unknown, scatter: boolean): number[] {
  if (series.type === "boxplot" && Array.isArray(d)) {
    return d.map((n) => Number(n)).filter((n) => Number.isFinite(n));
  }
  const v = entryNumber(d, scatter, 1);
  return v === null ? [] : [v];
}

/**
 * Whether a reference line sits at or below zero: a plain line at a value, or
 * a band with such an edge. A computed line is not: it is computed from the
 * values on screen, which the axis check has already cleared.
 */
function lineAtOrBelowZero(line: ResolvedReferenceLine): boolean {
  if (line.kind === "band") {
    return (
      (line.value !== undefined && line.value <= 0) || (line.to !== undefined && line.to <= 0)
    );
  }
  if (line.kind && line.kind !== "value") return false;
  return line.value !== undefined && line.value <= 0;
}

function restyleAxis(axis: unknown, s: AxisStyle, category: boolean): unknown {
  const a = { ...asObj(axis) };
  if (s.title !== undefined) {
    a.name = s.title;
    a.nameLocation = "middle";
    a.nameGap = category ? 28 : 40;
    a.nameTextStyle = { ...asObj(a.nameTextStyle), fontSize: 11 };
  }
  if (s.show === false) {
    a.show = false;
  }
  if (!category) {
    if (s.min !== undefined) a.min = s.min;
    if (s.max !== undefined) a.max = s.max;
    if (s.log) {
      a.type = "log";
      a.logBase = 10;
    }
  }
  if (s.rotate !== undefined) a.axisLabel = { ...asObj(a.axisLabel), rotate: s.rotate, interval: 0 };
  return a;
}

const LOG_MESSAGE = "A logarithmic axis cannot show zero or negative values";

/** Apply the axes of the style to `option`; returns the message when a logarithmic axis would be drawn wrong. */
function applyAxes(option: Obj, series: Obj[], cols: (string | undefined)[], input: StyleInput): string | undefined {
  const axes = input.style?.axes;
  if (!axes) return undefined;
  const scatter = input.kind === "scatter" || input.kind === "bubble";
  const horizontal = input.kind === "hbar";
  const catKey = horizontal ? "yAxis" : "xAxis";
  const valKey = horizontal ? "xAxis" : "yAxis";
  const fmt = (col?: string) => input.style?.valueFormats?.[col ?? ""] ?? input.style?.format;
  // Second axis: the right values go to a copy of the left one.
  const right = new Set(axes.rightValues ?? []);
  if (right.size && !horizontal) {
    const left = Array.isArray(option[valKey]) ? (option[valKey] as unknown[])[0] : option[valKey];
    const existing = Array.isArray(option[valKey]) ? (option[valKey] as unknown[])[1] : undefined;
    const rightAxis: Obj =
      existing === undefined
        ? { ...asObj(left), position: "right", splitLine: { show: false }, alignTicks: false }
        : asObj(existing);
    // The right axis's name: its own title, or the labels of the values on
    // it, so an axis without a title is not a nameless one
    // (`PR #199 review M SHOULD-FIX BI-17 2`).
    if (rightAxis.name === undefined && !axes.y2?.title) {
      const labels = [...right]
        .map((col) => input.measureLabels?.[col] ?? displayColumn(col))
        .filter((label) => label.length > 0);
      if (labels.length) rightAxis.name = labels.join(", ");
    }
    option[valKey] = [left, rightAxis];
    series.forEach((s, i) => {
      s.yAxisIndex = right.has(cols[i] ?? "") ? 1 : 0;
    });
  }
  const valueAxes = (Array.isArray(option[valKey]) ? (option[valKey] as unknown[]) : [option[valKey]]).map((a) => asObj(a));
  const styles = [axes.y, axes.y2];
  const firstOnAxis = (i: number) => cols.find((_, k) => (series[k]?.yAxisIndex ?? 0) === i) ?? cols[0];
  for (let i = 0; i < valueAxes.length; i++) {
    const st = styles[i];
    if (!st || (i > 0 && !right.size && !axes.y2)) continue;
    if (st.log) {
      const hit = series.some((s) => ((s.yAxisIndex ?? 0) === i || valueAxes.length === 1) && (s.data as unknown[] | undefined)?.some((d) => {
        return axisValues(s, d, scatter).some((n) => n <= 0);
      }));
      if (hit) return LOG_MESSAGE;
      // A line or a band at or below zero would vanish on a logarithmic
      // axis, so it is refused with the same sentence.
      if (i === 0 && (input.referenceLines ?? []).some(lineAtOrBelowZero)) return LOG_MESSAGE;
    }
    let axis = restyleAxis(valueAxes[i], st, false) as Obj;
    // The labels of each axis are written in the format of the value it carries.
    const col = firstOnAxis(i);
    axis = { ...axis, axisLabel: { ...asObj(axis.axisLabel), formatter: (v: number) => formatAxisValue(v, fmt(col), input.locale, input.decimals) } };
    valueAxes[i] = axis;
  }
  option[valKey] = Array.isArray(option[valKey]) ? valueAxes : valueAxes[0];
  if (scatter && axes.x) {
    if (axes.x.log && series.some((s) => (s.data as unknown[] | undefined)?.some((d) => { const n = entryNumber(d, true, 0); return n !== null && n <= 0; }))) return LOG_MESSAGE;
    option.xAxis = restyleAxis(option.xAxis, axes.x, false);
  } else if (axes.x && option[catKey]) {
    option[catKey] = Array.isArray(option[catKey])
      ? (option[catKey] as unknown[]).map((a) => restyleAxis(a, axes.x!, true))
      : restyleAxis(option[catKey], axes.x, true);
  }
  return undefined;
}

/** `[min, max]` of the numbers in the first series, or `null` when there are none. */
function shown(series: Obj | undefined): [number, number] | null {
  const nums = ((series?.data as unknown[] | undefined) ?? []).map((d) => entryNumber(d, false, 0)).filter((n): n is number => n !== null);
  return nums.length ? [Math.min(...nums), Math.max(...nums)] : null;
}

function applyByValue(series: Obj[], stops: string[]): void {
  const first = series[0];
  const range = shown(first);
  if (!first || !range) return;
  const [lo, hi] = range;
  const colour = (n: number) => interpolate(stops, hi === lo ? 0.5 : (n - lo) / (hi - lo));
  first.data = (first.data as unknown[]).map((d) => {
    const n = entryNumber(d, false, 0);
    if (n === null) return d;
    const item = typeof d === "object" && d && !Array.isArray(d) ? (d as Obj) : { value: d };
    return { ...item, itemStyle: { ...asObj(item.itemStyle), color: colour(n) } };
  });
}

function applySeriesColours(series: Obj[], cols: (string | undefined)[], byName: Record<string, string>): void {
  series.forEach((s, i) => {
    // By the series' name, else by the result column it draws (the builder lists result names).
    const own = (typeof s.name === "string" ? byName[s.name] : undefined) ?? (cols[i] ? byName[cols[i]!] : undefined);
    if (own) {
      s.itemStyle = { ...asObj(s.itemStyle), color: own };
      if (s.type === "line") s.lineStyle = { ...asObj(s.lineStyle), color: own };
      if (s.areaStyle) s.areaStyle = { opacity: 0.18, color: own };
    }
    // Pie, rose, funnel and treemap name their marks, not their series.
    if (Array.isArray(s.data)) {
      s.data = s.data.map((d) => {
        const item = asObj(d);
        const colour = typeof item.name === "string" ? byName[item.name] : undefined;
        return !Array.isArray(d) && colour ? { ...item, itemStyle: { ...asObj(item.itemStyle), color: colour } } : d;
      });
    }
  });
}

const POINT_LABEL_KINDS = new Set(["bar", "line", "scatter", "heatmap"]);

function applyLabels(series: Obj[], cols: (string | undefined)[], input: StyleInput): void {
  const fmtFor = (col?: string): ValueFormat | undefined => input.style?.valueFormats?.[col ?? ""] ?? input.style?.format;
  series.forEach((s, i) => {
    // The transparent base of a waterfall has nothing to write.
    if (asObj(s.itemStyle).color === "transparent") return;
    const f = fmtFor(cols[i]);
    const write = (v: unknown) => (typeof v === "number" ? formatValue(v, f, input.locale, input.decimals) : "");
    if (POINT_LABEL_KINDS.has(String(s.type))) {
      s.label = {
        ...asObj(s.label),
        show: true,
        position: input.kind === "hbar" ? "right" : input.kind === "stacked" ? "inside" : "top",
        fontSize: 10,
        formatter: (p: { value?: unknown }) => write(Array.isArray(p.value) ? p.value[p.value.length - 1] : p.value),
      };
    } else if (["pie", "funnel", "treemap"].includes(String(s.type))) {
      s.label = {
        ...asObj(s.label),
        show: true,
        formatter: (p: { name?: string; value?: unknown }) => `${p.name ?? ""} ${write(p.value)}`.trim(),
      };
    } else {
      return;
    }
    // Labels that would overlap are dropped, not drawn on top of each other (feature item 5).
    s.labelLayout = { hideOverlap: true };
  });
}

function applyNotes(series: Obj[], option: Obj, input: StyleInput): number {
  const notes = input.style?.annotations ?? [];
  if (!notes.length) return 0;
  const horizontal = input.kind === "hbar";
  const catAxis = asObj(Array.isArray(option[horizontal ? "yAxis" : "xAxis"]) ? (option[horizontal ? "yAxis" : "xAxis"] as unknown[])[0] : option[horizontal ? "yAxis" : "xAxis"]);
  const categories = Array.isArray(catAxis.data) ? (catAxis.data as unknown[]).map(String) : [];
  let hidden = 0;
  for (const note of notes) {
    const target = note.series ? series.find((s) => s.name === note.series) : series.find((s) => s.type === "bar" || s.type === "line");
    const at = categories.indexOf(note.x);
    const value = target && at >= 0 ? entryNumber((target.data as unknown[] | undefined)?.[at], false, 0) : null;
    if (!target || at < 0 || value === null) {
      hidden += 1;
      continue;
    }
    const mark = asObj(target.markPoint);
    target.markPoint = {
      ...mark,
      symbol: "pin",
      symbolSize: 26,
      // The note's text is the user's, and the server's check for it allows
      // `<` and `>`; ECharts sets a tooltip formatter's result as HTML, so
      // the text is escaped before it goes in (`PR #199 review M BLOCKER 1`).
      tooltip: { formatter: (p: { name?: string }) => escapeHtml(p.name ?? "") },
      data: [...((mark.data as unknown[] | undefined) ?? []), { name: note.text, coord: horizontal ? [value, at] : [at, value], value }],
    };
  }
  return hidden;
}

/** Only the fields chosen are written in an axis tooltip. */
function applyTooltipFields(option: Obj, series: Obj[], cols: (string | undefined)[], input: StyleInput): void {
  const fields = input.style?.tooltip?.fields;
  const tip = asObj(option.tooltip);
  if (!fields?.length || tip.trigger !== "axis") return;
  const keep = new Set(fields);
  option.tooltip = {
    ...tip,
    formatter: (params: unknown) => {
      const list = (Array.isArray(params) ? params : [params]) as { axisValueLabel?: string; seriesIndex: number; marker?: string; seriesName?: string; value?: unknown }[];
      const f = (col?: string) => input.style?.valueFormats?.[col ?? ""] ?? input.style?.format;
      // The series name and the axis label are warehouse data and ECharts
      // sets a tooltip formatter's result as HTML, so every text but the
      // marker's own markup is escaped (`PR #199 review M BLOCKER 1`).
      const lines = list
        .filter((p) => keep.has(cols[p.seriesIndex] ?? ""))
        .map((p) => `${p.marker ?? ""} ${escapeHtml(p.seriesName || cols[p.seriesIndex] || "")}: <b>${typeof p.value === "number" ? formatValue(p.value, f(cols[p.seriesIndex]), input.locale, input.decimals) : escapeHtml(String(p.value ?? ""))}</b>`);
      return [escapeHtml(list[0]?.axisValueLabel ?? ""), ...lines].join("<br/>");
    },
  };
  void series;
}

/** Apply the post-pass. The option passed in is not changed. */
export function styleOption(base: EChartsOption, input: StyleInput): Styled {
  const style = input.style;
  if (!usesStylePass(style) || !style) return { option: base, hiddenNotes: 0 };
  const option: Obj = { ...(base as Obj) };
  const series: Obj[] = (Array.isArray(option.series) ? (option.series as Obj[]) : []).map((s) => ({ ...s }));
  const cols = input.columns ?? seriesColumns(input, series.length);
  const message = applyAxes(option, series, cols, input);
  if (message) return { option: base, message, hiddenNotes: 0 };
  if (style.colors?.byValue) applyByValue(series, style.colors.byValue);
  if (style.colors?.series) applySeriesColours(series, cols, style.colors.series);
  // A per-value format reaches the tooltip of its own series.
  series.forEach((s, i) => {
    const f = style.valueFormats?.[cols[i] ?? ""];
    if (f) s.tooltip = { ...asObj(s.tooltip), valueFormatter: (v: unknown) => (typeof v === "number" ? formatValue(v, f, input.locale, input.decimals) : String(v)) };
  });
  if (style.labels?.show) applyLabels(series, cols, input);
  const hiddenNotes = applyNotes(series, option, input);
  option.series = series;
  applyTooltipFields(option, series, cols, input);
  return { option: option as EChartsOption, hiddenNotes };
}
