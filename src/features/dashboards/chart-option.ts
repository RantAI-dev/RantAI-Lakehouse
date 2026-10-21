import { displayColumn } from "@/lib/chart-joins";
import type { EChartsOption } from "echarts";
import { decimalsFor, formatNumber, monthlyAxisLabel } from "@/lib/chart-axis";
import { DEFAULT_PALETTE, resolvePalette } from "@/lib/chart-palettes";
import type { ResolvedReferenceLine, StyleFields, ValueFormat } from "@/lib/chart-style";
import { formatAxisValue, formatValue } from "@/lib/value-format";
import type { ChartSpec } from "@/lib/dashboard-specs";
import { boxplotNeedsLogAxis, toBoxplot, toCalendar, toSankey, toSunburst } from "@/lib/chart-transforms";

/**
 * Phase 2 QA R16: a long series name folds with an ellipsis in the legend and is read in full
 * from the legend's tooltip (which every legend turns on beside this), so the legend stays one line.
 */
export const LEGEND_TEXT = { width: 140, overflow: "truncate" as const };

// The map kinds (`geomap`, `pointmap`, `geoheat`) are built in geo-option.ts:
// they need the registered map's feature names and report what they leave
// out, which a plain `(spec, rows, dark) => option` cannot.

/** buildOption only needs how to render (not the SQL) — fits ChartSpec & ChartRenderSpec. */
type Renderable = Pick<ChartSpec, "kind" | "x" | "y" | "series" | "target"> & {
  /** BI-6: a readable name ("avg of stay") for a result column that holds a measure with its own aggregate. */
  measureLabels?: Record<string, string>;
};

/** The console's categorical palette (indigo-led). Used consistently across charts. */
export const PALETTE: readonly string[] = DEFAULT_PALETTE;

// English number format, matching the English UI (data values stay as-is).
const fmtInt = (v: number) => Math.round(v).toLocaleString("en-US");

type Row = Record<string, unknown>;

/**
 * BI-9 review fix (SHOULD-FIX) R5: a line with more points than this draws no
 * point markers (1000 hourly points read as a solid block). The hover marker
 * and tooltip still work: ECharts shows the symbol under the axis pointer even
 * with `showSymbol: false`.
 */
export const MARKER_LIMIT = 60;

/** Whether a chart of `kind` with `points` category points draws a marker on each. */
export function showsPointMarkers(kind: string, points: number): boolean {
  return kind === "line" && points <= MARKER_LIMIT;
}
const num = (v: unknown) => Number(v ?? 0);
const str = (v: unknown) => String(v ?? "");

export function buildOption(
  spec: Renderable,
  rows: Row[],
  dark: boolean,
  /** `firstDay`: the calendar's first weekday (0 Sunday, 1 Monday), from the Settings (BI-9). */
  opts: {
    firstDay?: 0 | 1;
    /** BI-17: the chart's formats and colours; absent draws the chart as it always was. */
    style?: StyleFields;
    /** BI-17: the deployment's number locale. */
    locale?: string;
    /** BI-17: the company palette (Admin > Settings). */
    companyPalette?: readonly string[];
  } = {},
): EChartsOption {
  // BI-8 review fix (SHOULD-FIX) R4: axis, tooltip and label numbers carry the
  // decimals the drawn values need (none for whole numbers, as before).
  const yColumns = Array.isArray(spec.y) ? spec.y : [spec.y];
  const decimals = decimalsFor(rows.flatMap((r) => yColumns.map((c) => num(r[c]))));
  // BI-17 (D3, D4): every number goes through the one formatter, with the
  // chart's format (or the one of its first value) and the deployment's locale.
  // With no format this is `formatNumber` / `formatCompactNumber`, as before.
  const locale = opts.locale ?? "en-US";
  const fmtOf = (column?: string): ValueFormat | undefined => (column ? opts.style?.valueFormats?.[column] : undefined) ?? opts.style?.format;
  const fmtInt = (v: number) => formatValue(v, fmtOf(yColumns[0]), locale, decimals);
  const fmtCompact = (v: number) => formatAxisValue(v, fmtOf(yColumns[0]), locale, decimals);
  const pal = resolvePalette(opts.style?.colors, opts.companyPalette);
  // A company palette may hold only two colours, so an index wraps.
  const pc = (i: number) => pal[i % pal.length] ?? DEFAULT_PALETTE[0];
  const axis = dark ? "#a1a1aa" : "#71717a";
  const split = dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.06)";
  const tooltipBg = dark ? "#18181b" : "#ffffff";
  const tooltipText = dark ? "#e4e4e7" : "#18181b";

  const base: EChartsOption = {
    color: [...pal],
    textStyle: { color: axis, fontFamily: "inherit" },
    tooltip: {
      backgroundColor: tooltipBg,
      borderWidth: 0,
      textStyle: { color: tooltipText, fontSize: 12 },
      valueFormatter: (v) => (typeof v === "number" ? fmtInt(v) : String(v)),
    },
    grid: { left: 8, right: 16, top: 16, bottom: 8, containLabel: true },
  };

  const cat = rows.map((r) => str(r[spec.x]));
  // BI-6: legend, axis and tooltip name a measure with its own aggregate as
  // "avg of stay", not by the `avg_stay` it is called in the result.
  const labelOf = (column: string) => spec.measureLabels?.[column] ?? displayColumn(column);

  // Pie / donut.
  if (spec.kind === "pie") {
    const yCol = Array.isArray(spec.y) ? spec.y[0] : spec.y;
    return {
      ...base,
      grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => {
        const o = p as { name: string; value: number; percent: number };
        return `${o.name}<br/><b>${fmtInt(o.value)}</b> (${o.percent}%)`;
      } },
      legend: { bottom: 0, textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      series: [
        {
          type: "pie",
          radius: ["45%", "72%"],
          center: ["50%", "44%"],
          avoidLabelOverlap: true,
          itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 2 },
          label: { show: false },
          data: rows.map((r) => ({ name: str(r[spec.x]), value: num(r[yCol]) })),
        },
      ],
    };
  }

  // ── Non-standard-axis kinds (Metabase/Tableau-parity) ─────────────────────
  const yArr = Array.isArray(spec.y) ? spec.y : [spec.y];
  const y0 = yArr[0];
  const niceMax = (v: number) => {
    if (v <= 0) return 100;
    const p = Math.pow(10, Math.floor(Math.log10(v)));
    return Math.ceil(v / p) * p;
  };
  const valAxis = (name?: string, right = false) => ({
    type: "value" as const,
    name, nameTextStyle: { color: axis, fontSize: 10 }, nameGap: 8,
    position: (right ? "right" : "left") as "right" | "left",
    axisLabel: { color: axis, fontSize: 11, formatter: (v: number) => fmtCompact(v) },
    splitLine: { show: !right, lineStyle: { color: split } },
    axisLine: { show: false }, axisTick: { show: false },
  });
  const catAxis = (data: string[]) => ({
    type: "category" as const, data,
    axisLabel: { color: axis, fontSize: 11, interval: 0, hideOverlap: true, rotate: data.length > 6 ? 28 : 0, ...monthlyAxisLabel(data) },
    axisLine: { lineStyle: { color: split } }, axisTick: { show: false },
  });
  type PieP = { name: string; value: number; percent: number };
  type ItemP = { name: string; value: number[] };
  type AxisPs = { dataIndex: number }[];

  if (spec.kind === "rose") {
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as PieP; return `${o.name}<br/><b>${fmtInt(o.value)}</b> (${o.percent}%)`; } },
      legend: { bottom: 0, type: "scroll", textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      series: [{ type: "pie", roseType: "area", radius: ["15%", "72%"], center: ["50%", "44%"],
        itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 2, borderRadius: 4 },
        label: { show: false }, data: rows.map((r) => ({ name: str(r[spec.x]), value: num(r[y0]) })) }],
    } as EChartsOption;
  }

  if (spec.kind === "funnel") {
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as PieP; return `${o.name}<br/><b>${fmtInt(o.value)}</b>`; } },
      legend: { bottom: 0, type: "scroll", textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      series: [{ type: "funnel", left: "8%", right: "8%", top: 10, bottom: 28, sort: "descending", gap: 2,
        label: { show: true, position: "inside", color: "#fff", fontSize: 11, formatter: "{b}" },
        itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 1 },
        data: rows.map((r) => ({ name: str(r[spec.x]), value: num(r[y0]) })) }],
    } as EChartsOption;
  }

  if (spec.kind === "treemap") {
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as PieP; return `${o.name}<br/><b>${fmtInt(o.value)}</b>`; } },
      series: [{ type: "treemap", roam: false, nodeClick: false, breadcrumb: { show: false },
        width: "100%", height: "100%", top: 4, left: 4, right: 4, bottom: 4,
        label: { show: true, formatter: "{b}", color: "#fff", fontSize: 11 },
        itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 2, gapWidth: 2 },
        data: rows.map((r, i) => ({ name: str(r[spec.x]), value: num(r[y0]), itemStyle: { color: pc(i) } })) }],
    } as EChartsOption;
  }

  if (spec.kind === "radar") {
    const maxVal = Math.max(1, ...rows.map((r) => num(r[y0])));
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item" },
      radar: { indicator: rows.map((r) => ({ name: str(r[spec.x]), max: maxVal * 1.1 })),
        axisName: { color: axis, fontSize: 10 }, splitLine: { lineStyle: { color: split } },
        splitArea: { show: false }, axisLine: { lineStyle: { color: split } } },
      series: [{ type: "radar", symbolSize: 4, areaStyle: { opacity: 0.12 }, lineStyle: { width: 2 },
        data: [{ value: rows.map((r) => num(r[y0])), name: String(y0) }] }],
    } as EChartsOption;
  }

  if (spec.kind === "gauge") {
    const v = num(rows[0]?.v ?? rows[0]?.[y0]);
    const max = spec.target && spec.target > 0 ? spec.target : niceMax(v);
    return { ...base, grid: undefined,
      series: [{ type: "gauge", startAngle: 210, endAngle: -30, min: 0, max, radius: "92%", center: ["50%", "56%"],
        progress: { show: true, width: 12, itemStyle: { color: pc(0) } },
        axisLine: { lineStyle: { width: 12, color: [[1, split]] } },
        axisTick: { show: false }, splitLine: { length: 8, lineStyle: { color: axis } },
        axisLabel: { color: axis, fontSize: 9, distance: 12, formatter: (x: number) => fmtCompact(x) },
        pointer: { width: 4, itemStyle: { color: pc(0) } },
        anchor: { show: true, size: 8, itemStyle: { color: pc(0) } },
        detail: { valueAnimation: true, formatter: (x: number) => fmtInt(x), color: axis, fontSize: 22, offsetCenter: [0, "70%"] },
        title: { show: false }, data: [{ value: v }] }],
    } as EChartsOption;
  }

  if (spec.kind === "scatter" || spec.kind === "bubble") {
    const xc = yArr[0], yc = yArr[1] ?? yArr[0], sc = yArr[2];
    const maxS = spec.kind === "bubble" && sc ? Math.max(1, ...rows.map((r) => num(r[sc]))) : 1;
    return { ...base,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => {
        const o = p as ItemP; const d = o.value;
        return `${o.name}<br/>${labelOf(xc)}: <b>${fmtInt(d[0])}</b><br/>${labelOf(yc)}: <b>${fmtInt(d[1])}</b>${sc ? `<br/>${labelOf(sc)}: <b>${fmtInt(d[2])}</b>` : ""}`;
      } },
      xAxis: valAxis(labelOf(xc)), yAxis: valAxis(labelOf(yc)),
      series: [{ type: "scatter",
        symbolSize: spec.kind === "bubble" && sc ? (d: unknown) => { const a = d as number[]; return 8 + 38 * Math.sqrt((a[2] ?? 0) / maxS); } : 12,
        itemStyle: { opacity: 0.75 },
        data: rows.map((r) => ({ name: str(r[spec.x]), value: [num(r[xc]), num(r[yc]), sc ? num(r[sc]) : 0] })) }],
    } as EChartsOption;
  }

  if (spec.kind === "combo") {
    const m1 = yArr[0], m2 = yArr[1] ?? yArr[0];
    return { ...base,
      tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: "cross" } },
      legend: { top: 0, textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      grid: { ...base.grid, top: 34 },
      xAxis: catAxis(cat),
      // alignTicks off — the two metrics have different scales (e.g. millions vs thousands).
      yAxis: [{ ...valAxis(labelOf(m1)), alignTicks: false }, { ...valAxis(labelOf(m2), true), alignTicks: false }],
      series: [
        { name: labelOf(m1), type: "bar", data: rows.map((r) => num(r[m1])), barMaxWidth: 34, itemStyle: { borderRadius: [4, 4, 0, 0] } },
        { name: labelOf(m2), type: "line", yAxisIndex: 1, smooth: true, symbolSize: 6, lineStyle: { width: 2 }, data: rows.map((r) => num(r[m2])) },
      ],
    } as EChartsOption;
  }

  if (spec.kind === "waterfall") {
    const vals = rows.map((r) => num(r[y0]));
    const pad: number[] = []; const up: (number | "-")[] = []; const down: (number | "-")[] = [];
    let acc = 0;
    for (const v of vals) {
      pad.push(v >= 0 ? acc : acc + v);
      up.push(v >= 0 ? v : "-");
      down.push(v < 0 ? -v : "-");
      acc += v;
    }
    return { ...base,
      tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: "shadow" },
        formatter: (ps: unknown) => { const a = ps as AxisPs; const i = a[0]?.dataIndex ?? 0; return `${cat[i]}<br/><b>${fmtInt(vals[i])}</b>`; } },
      xAxis: catAxis(cat), yAxis: valAxis(),
      series: [
        { type: "bar", stack: "wf", itemStyle: { color: "transparent" }, emphasis: { itemStyle: { color: "transparent" } }, data: pad },
        { name: "naik", type: "bar", stack: "wf", data: up, itemStyle: { color: pc(2), borderRadius: [3, 3, 0, 0] } },
        { name: "turun", type: "bar", stack: "wf", data: down, itemStyle: { color: pc(4), borderRadius: [3, 3, 0, 0] } },
      ],
    } as EChartsOption;
  }

  if (spec.kind === "heatmap" && spec.series) {
    const xCats: string[] = []; const yCats: string[] = [];
    const seriesCol = spec.series;
    const data: [number, number, number][] = [];
    for (const r of rows) {
      const xv = str(r[spec.x]); const yv = str(r[seriesCol]);
      let xi = xCats.indexOf(xv); if (xi < 0) { xi = xCats.length; xCats.push(xv); }
      let yi = yCats.indexOf(yv); if (yi < 0) { yi = yCats.length; yCats.push(yv); }
      data.push([xi, yi, num(r[y0])]);
    }
    const maxV = Math.max(1, ...data.map((d) => d[2]));
    return { ...base, grid: { ...base.grid, top: 8, bottom: 48, left: 8, right: 12 },
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as { value: number[] }; return `${xCats[o.value[0]]} · ${yCats[o.value[1]]}<br/><b>${fmtInt(o.value[2])}</b>`; } },
      xAxis: { type: "category", data: xCats, splitArea: { show: true }, axisLabel: { color: axis, fontSize: 10, interval: 0, hideOverlap: true, rotate: xCats.length > 6 ? 30 : 0 }, axisLine: { lineStyle: { color: split } }, axisTick: { show: false } },
      yAxis: { type: "category", data: yCats, splitArea: { show: true }, axisLabel: { color: axis, fontSize: 10 }, axisLine: { lineStyle: { color: split } }, axisTick: { show: false } },
      visualMap: { min: 0, max: maxV, calculable: false, orient: "horizontal", left: "center", bottom: 0, itemHeight: 60, textStyle: { color: axis, fontSize: 10 }, inRange: { color: dark ? ["#1e1b4b", "#6366f1", "#a5b4fc"] : ["#eef2ff", "#818cf8", "#4338ca"] } },
      series: [{ type: "heatmap", data, label: { show: false }, emphasis: { itemStyle: { shadowBlur: 6, shadowColor: "rgba(0,0,0,0.3)" } } }],
    } as EChartsOption;
  }

  if (spec.kind === "sankey" && spec.series) {
    const { nodes, links } = toSankey(rows, spec.x, spec.series, y0);
    const labelOf = new Map(nodes.map((n) => [n.name, n.label]));
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => {
        const o = p as { dataType?: string; name: string; value: number; data: { source?: string; target?: string } };
        if (o.dataType === "edge") return `${labelOf.get(o.data.source ?? "")} → ${labelOf.get(o.data.target ?? "")}<br/><b>${fmtInt(o.value)}</b>`;
        return `${labelOf.get(o.name) ?? o.name}<br/><b>${fmtInt(o.value)}</b>`;
      } },
      series: [{ type: "sankey", left: 8, right: 96, top: 8, bottom: 8, nodeGap: 10, draggable: false,
        emphasis: { focus: "adjacency" },
        label: { color: axis, fontSize: 11, formatter: (p: { name: string }) => labelOf.get(p.name) ?? p.name },
        lineStyle: { color: "gradient", curveness: 0.5, opacity: dark ? 0.35 : 0.3 },
        // Left side indigo, right side sky — the console palette, not
        // ECharts' default per-node colours.
        data: nodes.map((n) => ({ name: n.name, itemStyle: { color: n.name.startsWith("a:") ? pc(0) : pc(1) } })), links }],
    } as EChartsOption;
  }

  if (spec.kind === "sunburst" && spec.series) {
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as { treePathInfo: { name: string }[]; value: number }; return `${o.treePathInfo.map((t) => t.name).filter(Boolean).join(" › ")}<br/><b>${fmtInt(o.value)}</b>`; } },
      // `nodeClick: false`: by default a click zooms the ring in, and a click now
      // opens the drill menu (BI-18·B); the two cannot share the gesture.
      series: [{ type: "sunburst", radius: ["12%", "90%"], center: ["50%", "50%"], sort: undefined, nodeClick: false,
        itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 1.5 },
        label: { color: "#fff", fontSize: 10, minAngle: 10, rotate: "radial" },
        levels: [{}, { r0: "12%", r: "45%" }, { r0: "45%", r: "90%", label: { fontSize: 9 } }],
        data: toSunburst(rows, spec.x, spec.series, y0) }],
    } as EChartsOption;
  }

  if (spec.kind === "boxplot") {
    const { categories, data, counts } = toBoxplot(rows, spec.x, y0);
    const log = boxplotNeedsLogAxis(data);
    // "(n=…)" under each category: a box from one row is a flat line, and
    // the count says why rather than leaving it looking empty.
    const labels = categories.map((c, i) => (counts[i] != null ? `${c} (n=${counts[i]})` : c));
    const nText = (i: number) => (counts[i] != null ? `<br/>${counts[i]} rows` : "");
    // A category whose five numbers are equal gets a dot on top of its line.
    const flat = data.flatMap((d, i) => (d[0] === d[4] ? [[i, d[0]]] : []));
    return { ...base,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => {
        const o = p as { name: string; data: number[]; seriesType: string; dataIndex: number };
        if (o.seriesType === "scatter") {
          const i = o.data[0];
          return `${categories[i]}${nText(i)}<br/>every value <b>${fmtInt(o.data[1])}</b>`;
        }
        // `data` is the five-number array we passed (toBoxplot).
        const [min, q1, med, q3, max] = o.data;
        return `${categories[o.dataIndex]}${nText(o.dataIndex)}<br/>max <b>${fmtInt(max)}</b><br/>Q3 ${fmtInt(q3)}<br/>median <b>${fmtInt(med)}</b><br/>Q1 ${fmtInt(q1)}<br/>min <b>${fmtInt(min)}</b>`;
      } },
      xAxis: catAxis(labels),
      // Log only when the values span 1000× or more (boxplotNeedsLogAxis),
      // and the axis says so, so the spacing is not read as linear.
      yAxis: log ? { ...valAxis("log scale"), type: "log" as const, logBase: 10 } : valAxis(),
      series: [
        { type: "boxplot", data, itemStyle: { color: dark ? "rgba(99,102,241,0.25)" : "rgba(99,102,241,0.15)", borderColor: pc(0) } },
        ...(flat.length ? [{ type: "scatter" as const, data: flat, symbolSize: 9, itemStyle: { color: pc(0) } }] : []),
      ],
    } as EChartsOption;
  }

  if (spec.kind === "calendar") {
    const { data, range } = toCalendar(rows, spec.x, y0);
    const maxV = Math.max(1, ...data.map((d) => d[1]));
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as { value: [string, number] }; return `${o.value[0]}<br/><b>${fmtInt(o.value[1])}</b>`; } },
      visualMap: { min: 0, max: maxV, calculable: false, orient: "horizontal", left: "center", bottom: 0, itemHeight: 60, textStyle: { color: axis, fontSize: 10 }, inRange: { color: dark ? ["#1e1b4b", "#6366f1", "#a5b4fc"] : ["#eef2ff", "#818cf8", "#4338ca"] } },
      calendar: range ? { range, top: 24, left: 36, right: 12, bottom: 44, cellSize: ["auto", "auto"],
        itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 2, color: dark ? "#18181b" : "#fafafa" },
        splitLine: { show: false }, yearLabel: { show: false },
        dayLabel: { color: axis, fontSize: 9, firstDay: opts.firstDay ?? 1 }, monthLabel: { color: axis, fontSize: 10 } } : undefined,
      series: range ? [{ type: "heatmap", coordinateSystem: "calendar", data }] : [],
      title: range ? undefined : { text: "No dated rows to show", left: "center", top: "middle", textStyle: { color: axis, fontSize: 12, fontWeight: "normal" } },
    } as EChartsOption;
  }

  const horizontal = spec.kind === "hbar";
  const valueAxis = {
    type: "value" as const,
    axisLabel: { color: axis, fontSize: 11, formatter: (v: number) => fmtCompact(v) },
    splitLine: { lineStyle: { color: split } },
    axisLine: { show: false },
    axisTick: { show: false },
  };
  const categoryAxis = {
    type: "category" as const,
    data: horizontal ? [...cat].reverse() : cat,
    axisLabel: {
      color: axis,
      fontSize: 11,
      interval: 0,
      hideOverlap: true,
      ...(horizontal ? {} : { rotate: cat.length > 6 ? 28 : 0, ...monthlyAxisLabel(cat) }),
    },
    axisLine: { lineStyle: { color: split } },
    axisTick: { show: false },
  };

  // Breakdown (2nd dimension): long-format data (x, series, value) → multiple series.
  if (spec.series) {
    const valueCol = Array.isArray(spec.y) ? spec.y[0] : spec.y;
    const seriesCol = spec.series;
    const cats: string[] = [];
    const groups: string[] = [];
    const lookup = new Map<string, number>();
    for (const r of rows) {
      const xv = str(r[spec.x]);
      const gv = str(r[seriesCol]);
      if (!cats.includes(xv)) cats.push(xv);
      if (!groups.includes(gv)) groups.push(gv);
      lookup.set(`${xv}||${gv}`, num(r[valueCol]));
    }
    const orderedCats = horizontal ? [...cats].reverse() : cats;
    const isLine = spec.kind === "line" || spec.kind === "area";
    const stack = spec.kind === "stacked";
    const series = groups.map((g) => ({
      name: g,
      type: isLine ? "line" : "bar",
      stack: stack ? "total" : undefined,
      smooth: isLine,
      showSymbol: showsPointMarkers(spec.kind, orderedCats.length),
      symbolSize: 5,
      areaStyle: spec.kind === "area" ? { opacity: 0.15 } : undefined,
      emphasis: { focus: "series" },
      barMaxWidth: 26,
      itemStyle: !isLine ? { borderRadius: horizontal ? [0, 3, 3, 0] : [3, 3, 0, 0] } : undefined,
      data: orderedCats.map((c) => lookup.get(`${c}||${g}`) ?? 0),
    }));
    return {
      ...base,
      tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: isLine ? "line" : "shadow" } },
      legend: { top: 0, type: "scroll", textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      grid: { ...base.grid, top: 34 },
      xAxis: horizontal ? valueAxis : { ...categoryAxis, data: orderedCats },
      yAxis: horizontal ? { ...categoryAxis, data: orderedCats } : valueAxis,
      series,
    } as EChartsOption;
  }

  // Stacked bar (e.g. wisnus + wisman).
  if (spec.kind === "stacked" && Array.isArray(spec.y)) {
    return {
      ...base,
      tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: "shadow" } },
      legend: { top: 0, right: 0, textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      grid: { ...base.grid, top: 32 },
      xAxis: horizontal ? valueAxis : categoryAxis,
      yAxis: horizontal ? categoryAxis : valueAxis,
      series: spec.y.map((col) => ({
        name: labelOf(col),
        type: "bar" as const,
        stack: "total",
        emphasis: { focus: "series" as const },
        data: (horizontal ? [...rows].reverse() : rows).map((r) => num(r[col])),
      })),
    };
  }

  // Bar / hbar / line / area — a single series, or (BI-6) one series per
  // measure when the chart lists several; they share the value axis.
  const isLine = spec.kind === "line" || spec.kind === "area";
  const measureCols = Array.isArray(spec.y) ? spec.y : [spec.y];
  if (measureCols.length > 1) {
    const ordered = horizontal ? [...rows].reverse() : rows;
    return {
      ...base,
      tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: isLine ? "line" : "shadow" } },
      legend: { top: 0, type: "scroll", textStyle: { color: axis, fontSize: 11, ...LEGEND_TEXT }, tooltip: { show: true }, icon: "circle" },
      grid: { ...base.grid, top: 34 },
      xAxis: horizontal ? valueAxis : categoryAxis,
      yAxis: horizontal ? categoryAxis : valueAxis,
      series: measureCols.map((col) => ({
        name: labelOf(col),
        type: isLine ? "line" : "bar",
        data: ordered.map((r) => num(r[col])),
        barMaxWidth: 26,
        smooth: isLine,
        showSymbol: showsPointMarkers(spec.kind, ordered.length),
        symbolSize: 6,
        lineStyle: isLine ? { width: 2 } : undefined,
        areaStyle: spec.kind === "area" ? { opacity: 0.12 } : undefined,
        emphasis: { focus: "series" },
        itemStyle: !isLine ? { borderRadius: horizontal ? [0, 3, 3, 0] : [3, 3, 0, 0] } : undefined,
      })),
    } as EChartsOption;
  }
  const yCol = measureCols[0];
  const values = (horizontal ? [...rows].reverse() : rows).map((r) => num(r[yCol]));

  return {
    ...base,
    tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: isLine ? "line" : "shadow" } },
    xAxis: horizontal ? valueAxis : categoryAxis,
    yAxis: horizontal ? categoryAxis : valueAxis,
    series: [
      {
        // Named only when the measure has a readable label of its own (BI-6),
        // so a chart saved before keeps the tooltip it had.
        ...(spec.measureLabels?.[yCol] ? { name: spec.measureLabels[yCol] } : {}),
        type: isLine ? "line" : "bar",
        data: values,
        barMaxWidth: 34,
        smooth: isLine,
        showSymbol: showsPointMarkers(spec.kind, values.length),
        symbolSize: 6,
        lineStyle: isLine ? { width: 2 } : undefined,
        areaStyle:
          spec.kind === "area"
            ? {
                color: {
                  type: "linear",
                  x: 0, y: 0, x2: 0, y2: 1,
                  colorStops: [
                    { offset: 0, color: "rgba(99,102,241,0.35)" },
                    { offset: 1, color: "rgba(99,102,241,0.02)" },
                  ],
                },
              }
            : undefined,
        itemStyle: { borderRadius: horizontal ? [0, 4, 4, 0] : [4, 4, 0, 0] },
      },
    ],
  };
}

export { fmtInt };

/** A reference line as the server resolved it (`BI-11`, `BI-17`). */
export type ReferenceLine = ResolvedReferenceLine;

/**
 * What the computed reference lines need of the tile (`BI-17` part A, D2): the
 * values on screen, how to write a number, and whether the result was cut off.
 */
export type LineContext = {
  /** The values of a result column, in the order the chart shows them; `undefined` names the first value. */
  values: (measure?: string) => number[];
  /** How a number of that value is written. */
  format?: (measure: string | undefined, v: number) => string;
  /** Set when the result was cut off: computed labels then say "of the N shown" (decision 9). */
  cutOff?: boolean;
};

const LINE_NAME: Record<string, string> = { average: "Average", median: "Median", min: "Min", max: "Max", trend: "Trend" };

/** The statistic a computed line draws, or `null` when there is nothing to compute it from. */
export function computedValue(kind: string, values: readonly number[]): number | null {
  const v = values.filter(Number.isFinite);
  if (!v.length) return null;
  switch (kind) {
    case "average": return v.reduce((a, b) => a + b, 0) / v.length;
    case "median": {
      const s = [...v].sort((a, b) => a - b);
      const mid = s.length >> 1;
      return s.length % 2 ? s[mid]! : (s[mid - 1]! + s[mid]!) / 2;
    }
    case "min": return Math.min(...v);
    case "max": return Math.max(...v);
    default: return null;
  }
}

/** Least-squares line over the points `0..n-1`: its value at the first and the last point. */
export function trendEnds(values: readonly number[]): [number, number] | null {
  const n = values.length;
  if (n < 2 || !values.every(Number.isFinite)) return null;
  const mx = (n - 1) / 2;
  const my = values.reduce((a, b) => a + b, 0) / n;
  let sxy = 0;
  let sxx = 0;
  values.forEach((y, x) => { sxy += (x - mx) * (y - my); sxx += (x - mx) ** 2; });
  const slope = sxy / sxx;
  return [my - slope * mx, my + slope * (n - 1 - mx)];
}

/**
 * Draw `lines` on the value axis of a bar, hbar, line, area, stacked or combo
 * option: one dashed line each, labelled with its label and value. The value
 * axis is widened to reach the highest line, since a target far above the data
 * would otherwise be drawn outside the chart. The option is returned as it is
 * when there are no lines or no series.
 *
 * `BI-17` (D2): a line may also be the average, median, minimum or maximum of
 * the values on screen, a linear trend over them, or a band between two
 * values (a shaded area). Computed lines need `ctx`; when the result was cut
 * off their label ends with "of the N shown", because they describe the rows
 * on screen, not every row of the table.
 *
 * `PR #199 review M BLOCKER 4` (decision M2): a computed line is drawn on the
 * axis of the measure it is computed from, not on the first series: `columns`
 * says which result column each series draws, and the series' own
 * `yAxisIndex` says which axis that is. The line hangs on the first series of
 * that axis and widens that axis only. A plain line and a band are values of
 * their own and stay on the first axis.
 */
export function withReferenceLines(
  option: EChartsOption,
  lines: readonly ReferenceLine[] | undefined,
  horizontal: boolean,
  dark: boolean,
  ctx?: LineContext,
  columns?: readonly (string | undefined)[],
): EChartsOption {
  const series = Array.isArray(option.series) ? option.series : [];
  if (!lines?.length || !series.length) return option;
  const fallback = dark ? "#fbbf24" : "#d97706";
  const write = (measure: string | undefined, v: number) => ctx?.format?.(measure, v) ?? formatNumber(v, decimalsFor([v]));
  const shown = ctx ? ctx.values().length : 0;
  const suffix = ctx?.cutOff ? ` of the ${shown} shown` : "";
  // The value axis a series draws on, and the axis of the measure a computed
  // line describes (`PR #199 review M BLOCKER 4`).
  const axisOf = (i: number) => Number((series[i] as { yAxisIndex?: number })?.yAxisIndex ?? 0);
  const axisOfMeasure = (measure?: string) => {
    if (!measure || !columns) return 0;
    const i = columns.findIndex((c) => c === measure);
    return i >= 0 ? axisOf(i) : 0;
  };
  type Group = { lineData: object[]; areaData: object[]; reach: number[]; measure?: string };
  const groups = new Map<number, Group>();
  const groupOf = (axis: number): Group => {
    const g = groups.get(axis) ?? { lineData: [], areaData: [], reach: [] };
    groups.set(axis, g);
    return g;
  };
  for (const l of lines) {
    const color = l.color ?? fallback;
    // Only a line with a colour of its own overrides the shared style, so a plain line stays {yAxis, name}.
    const paint = l.color ? { lineStyle: { color }, label: { color } } : {};
    const kind = l.kind ?? "value";
    const name = l.label ?? "";
    const at = (v: number) => (horizontal ? { xAxis: v } : { yAxis: v });
    // A computed line belongs to the axis of its measure; a plain line and a
    // band stay on the first axis.
    const computed = kind !== "value" && kind !== "band";
    const g = groupOf(computed ? axisOfMeasure(l.measure) : 0);
    if (g.measure === undefined) g.measure = l.measure;
    if (kind === "value" && l.value !== undefined && Number.isFinite(l.value)) {
      g.lineData.push({ ...at(l.value), name, ...paint });
      g.reach.push(l.value);
    } else if (kind === "band" && l.value !== undefined && l.to !== undefined && Number.isFinite(l.value) && Number.isFinite(l.to)) {
      g.areaData.push([{ ...at(Math.min(l.value, l.to)), name, itemStyle: { color, opacity: 0.14 }, ...(l.color ? { label: { color } } : {}) }, at(Math.max(l.value, l.to))]);
      g.reach.push(l.value, l.to);
    } else if (kind === "trend" && ctx) {
      const ends = trendEnds(ctx.values(l.measure));
      if (!ends) continue;
      const last = ctx.values(l.measure).length - 1;
      // The result of a horizontal bar chart is drawn bottom-up, so point i sits at index last - i.
      const point = (i: number, v: number) => ({ coord: horizontal ? [v, last - i] : [i, v] });
      g.lineData.push([
        { ...point(0, ends[0]), name: `${name || LINE_NAME.trend}${suffix}`, ...paint },
        point(last, ends[1]),
      ]);
      g.reach.push(ends[0], ends[1]);
    } else if (ctx && LINE_NAME[kind]) {
      const v = computedValue(kind, ctx.values(l.measure));
      if (v === null) continue;
      g.lineData.push({ ...at(v), name: `${name || LINE_NAME[kind]}${suffix}`, ...paint });
      g.reach.push(v);
    }
  }
  if (![...groups.values()].some((g) => g.lineData.length || g.areaData.length)) return option;
  const markOf = (g: Group) => ({
    silent: true,
    symbol: "none",
    lineStyle: { type: "dashed" as const, width: 1.5, color: fallback },
    label: {
      color: fallback, fontSize: 11, position: "insideEndTop" as const,
      formatter: (p: { name?: string; value?: unknown }) => {
        // A trend line has two ends and no single value to write.
        const v = typeof p.value === "number" ? write(g.measure, p.value) : "";
        return `${p.name ? `${p.name} ` : ""}${v}`.trim();
      },
    },
    data: g.lineData,
  });
  // The first series of an axis carries that axis's lines.
  const firstOf = (axis: number) => {
    const i = series.findIndex((_, k) => axisOf(k) === axis);
    return i < 0 ? 0 : i;
  };
  const topOf = (axis: number) => {
    let top = Number.NEGATIVE_INFINITY;
    for (const [a, g] of groups) if (a === axis && g.reach.length) top = Math.max(top, ...g.reach);
    return top;
  };
  const widen = (axis: unknown, top: number) => {
    if (!axis || typeof axis !== "object" || (axis as { type?: string }).type !== "value") return axis;
    if (!Number.isFinite(top)) return axis;
    return { ...(axis as object), max: (range: { max: number }) => Math.max(range.max, top) };
  };
  const valueAxes = horizontal ? option.xAxis : option.yAxis;
  const widened = Array.isArray(valueAxes)
    ? valueAxes.map((a, i) => widen(a, topOf(i)))
    : widen(valueAxes, topOf(0));
  const outSeries = series.map((s, i) => {
    const entry = [...groups.entries()].find(([axis, g]) => firstOf(axis) === i && (g.lineData.length || g.areaData.length));
    if (!entry) return s;
    const g = entry[1];
    return {
      ...s,
      ...(g.lineData.length ? { markLine: markOf(g) } : {}),
      ...(g.areaData.length ? { markArea: { silent: true, label: { position: "insideTopLeft" as const, fontSize: 11 }, data: g.areaData } } : {}),
    };
  });
  return {
    ...option,
    series: outSeries,
    ...(horizontal ? { xAxis: widened } : { yAxis: widened }),
  } as EChartsOption;
}
