import type { EChartsOption } from "echarts";
import type { ChartSpec } from "@/lib/dashboard-specs";
import { JAKARTA_MAP, normalizeJakartaArea } from "./echarts-maps";

/** buildOption hanya butuh cara render (bukan SQL) — muat ChartSpec & ChartRenderSpec. */
type Renderable = Pick<ChartSpec, "kind" | "x" | "y" | "series" | "target">;

/** Palet kategorikal konsol (indigo-led). Dipakai konsisten lintas chart. */
const PALETTE = [
  "#6366f1", "#0ea5e9", "#10b981", "#f59e0b",
  "#ef4444", "#8b5cf6", "#14b8a6", "#ec4899",
  "#84cc16", "#f97316",
];

const fmtInt = (v: number) => Math.round(v).toLocaleString("id-ID");
const fmtCompact = (v: number) => {
  const a = Math.abs(v);
  if (a >= 1_000_000) return `${(v / 1_000_000).toLocaleString("id-ID", { maximumFractionDigits: 1 })} jt`;
  if (a >= 1_000) return `${(v / 1_000).toLocaleString("id-ID", { maximumFractionDigits: 1 })} rb`;
  return fmtInt(v);
};

type Row = Record<string, unknown>;
const num = (v: unknown) => Number(v ?? 0);
const str = (v: unknown) => String(v ?? "");

export function buildOption(
  spec: Renderable,
  rows: Row[],
  dark: boolean,
): EChartsOption {
  const axis = dark ? "#a1a1aa" : "#71717a";
  const split = dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.06)";
  const tooltipBg = dark ? "#18181b" : "#ffffff";
  const tooltipText = dark ? "#e4e4e7" : "#18181b";

  const base: EChartsOption = {
    color: PALETTE,
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
      legend: { bottom: 0, textStyle: { color: axis, fontSize: 11 }, icon: "circle" },
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

  // ── Kind non-sumbu-standar (Metabase/Tableau-parity) ─────────────────────
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
    axisLabel: { color: axis, fontSize: 11, interval: 0, hideOverlap: true, rotate: data.length > 6 ? 28 : 0 },
    axisLine: { lineStyle: { color: split } }, axisTick: { show: false },
  });
  type PieP = { name: string; value: number; percent: number };
  type ItemP = { name: string; value: number[] };
  type AxisPs = { dataIndex: number }[];

  if (spec.kind === "rose") {
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as PieP; return `${o.name}<br/><b>${fmtInt(o.value)}</b> (${o.percent}%)`; } },
      legend: { bottom: 0, type: "scroll", textStyle: { color: axis, fontSize: 11 }, icon: "circle" },
      series: [{ type: "pie", roseType: "area", radius: ["15%", "72%"], center: ["50%", "44%"],
        itemStyle: { borderColor: dark ? "#09090b" : "#fff", borderWidth: 2, borderRadius: 4 },
        label: { show: false }, data: rows.map((r) => ({ name: str(r[spec.x]), value: num(r[y0]) })) }],
    } as EChartsOption;
  }

  if (spec.kind === "funnel") {
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as PieP; return `${o.name}<br/><b>${fmtInt(o.value)}</b>`; } },
      legend: { bottom: 0, type: "scroll", textStyle: { color: axis, fontSize: 11 }, icon: "circle" },
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
        data: rows.map((r, i) => ({ name: str(r[spec.x]), value: num(r[y0]), itemStyle: { color: PALETTE[i % PALETTE.length] } })) }],
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
        progress: { show: true, width: 12, itemStyle: { color: PALETTE[0] } },
        axisLine: { lineStyle: { width: 12, color: [[1, split]] } },
        axisTick: { show: false }, splitLine: { length: 8, lineStyle: { color: axis } },
        axisLabel: { color: axis, fontSize: 9, distance: 12, formatter: (x: number) => fmtCompact(x) },
        pointer: { width: 4, itemStyle: { color: PALETTE[0] } },
        anchor: { show: true, size: 8, itemStyle: { color: PALETTE[0] } },
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
        return `${o.name}<br/>${xc}: <b>${fmtInt(d[0])}</b><br/>${yc}: <b>${fmtInt(d[1])}</b>${sc ? `<br/>${sc}: <b>${fmtInt(d[2])}</b>` : ""}`;
      } },
      xAxis: valAxis(xc), yAxis: valAxis(yc),
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
      legend: { top: 0, textStyle: { color: axis, fontSize: 11 }, icon: "circle" },
      grid: { ...base.grid, top: 34 },
      xAxis: catAxis(cat),
      // alignTicks off — dua metrik beda skala (mis. jutaan vs ribuan).
      yAxis: [{ ...valAxis(m1), alignTicks: false }, { ...valAxis(m2, true), alignTicks: false }],
      series: [
        { name: m1, type: "bar", data: rows.map((r) => num(r[m1])), barMaxWidth: 34, itemStyle: { borderRadius: [4, 4, 0, 0] } },
        { name: m2, type: "line", yAxisIndex: 1, smooth: true, symbolSize: 6, lineStyle: { width: 2 }, data: rows.map((r) => num(r[m2])) },
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
        { name: "naik", type: "bar", stack: "wf", data: up, itemStyle: { color: PALETTE[2], borderRadius: [3, 3, 0, 0] } },
        { name: "turun", type: "bar", stack: "wf", data: down, itemStyle: { color: PALETTE[4], borderRadius: [3, 3, 0, 0] } },
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

  if (spec.kind === "geomap") {
    const data = rows.map((r) => ({ name: normalizeJakartaArea(str(r[spec.x])), value: num(r[y0]) }));
    const maxV = Math.max(1, ...data.map((d) => d.value));
    return { ...base, grid: undefined,
      tooltip: { ...base.tooltip, trigger: "item", formatter: (p: unknown) => { const o = p as { name: string; value?: number }; return `${o.name}<br/><b>${o.value != null && !Number.isNaN(o.value) ? fmtInt(o.value) : "—"}</b>`; } },
      visualMap: { min: 0, max: maxV, left: "left", bottom: 6, calculable: true, itemHeight: 70,
        textStyle: { color: axis, fontSize: 10 },
        inRange: { color: dark ? ["#1e1b4b", "#4f46e5", "#a5b4fc"] : ["#eef2ff", "#818cf8", "#3730a3"] } },
      series: [{ type: "map", map: JAKARTA_MAP, roam: false, aspectScale: 1,
        itemStyle: { borderColor: dark ? "#09090b" : "#ffffff", borderWidth: 1, areaColor: dark ? "#27272a" : "#f4f4f5" },
        emphasis: { label: { show: true, color: dark ? "#fff" : "#111", fontSize: 10 }, itemStyle: { areaColor: PALETTE[3] } },
        select: { disabled: true }, label: { show: false }, data }],
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
      ...(horizontal ? {} : { rotate: cat.length > 6 ? 28 : 0 }),
    },
    axisLine: { lineStyle: { color: split } },
    axisTick: { show: false },
  };

  // Breakdown (dimensi ke-2): data long-format (x, series, nilai) → banyak seri.
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
      showSymbol: spec.kind === "line",
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
      legend: { top: 0, type: "scroll", textStyle: { color: axis, fontSize: 11 }, icon: "circle" },
      grid: { ...base.grid, top: 34 },
      xAxis: horizontal ? valueAxis : { ...categoryAxis, data: orderedCats },
      yAxis: horizontal ? { ...categoryAxis, data: orderedCats } : valueAxis,
      series,
    } as EChartsOption;
  }

  // Stacked bar (mis. wisnus + wisman).
  if (spec.kind === "stacked" && Array.isArray(spec.y)) {
    return {
      ...base,
      tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: "shadow" } },
      legend: { top: 0, right: 0, textStyle: { color: axis, fontSize: 11 }, icon: "circle" },
      grid: { ...base.grid, top: 32 },
      xAxis: horizontal ? valueAxis : categoryAxis,
      yAxis: horizontal ? categoryAxis : valueAxis,
      series: spec.y.map((col) => ({
        name: col,
        type: "bar" as const,
        stack: "total",
        emphasis: { focus: "series" as const },
        data: (horizontal ? [...rows].reverse() : rows).map((r) => num(r[col])),
      })),
    };
  }

  // Bar / hbar / line / area — satu seri.
  const yCol = Array.isArray(spec.y) ? spec.y[0] : spec.y;
  const values = (horizontal ? [...rows].reverse() : rows).map((r) => num(r[yCol]));
  const isLine = spec.kind === "line" || spec.kind === "area";

  return {
    ...base,
    tooltip: { ...base.tooltip, trigger: "axis", axisPointer: { type: isLine ? "line" : "shadow" } },
    xAxis: horizontal ? valueAxis : categoryAxis,
    yAxis: horizontal ? categoryAxis : valueAxis,
    series: [
      {
        type: isLine ? "line" : "bar",
        data: values,
        barMaxWidth: 34,
        smooth: isLine,
        showSymbol: spec.kind === "line",
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
