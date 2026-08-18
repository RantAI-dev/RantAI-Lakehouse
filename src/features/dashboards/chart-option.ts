import type { EChartsOption } from "echarts";
import type { ChartSpec } from "@/lib/dashboard-specs";

/** buildOption hanya butuh cara render (bukan SQL) — muat ChartSpec & ChartRenderSpec. */
type Renderable = Pick<ChartSpec, "kind" | "x" | "y">;

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
