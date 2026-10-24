import type { EChartsOption } from "echarts";
import { forecast, isRefusal, type Forecast } from "@/lib/forecast";
import { bucketEnd, futureLabels, layPeriods, parseBucket, wallClockMs, type Periods } from "@/lib/forecast-periods";
import { isTruncation, type Grain } from "@/lib/time-grain";

/**
 * Drawing a `forecast` reference line (BI-17 part B, plan
 * `2026-10-24-bi-17b-forecast-clustering.md`, D4): future axis labels from the
 * last bucket and the grain, a dashed line for the point forecast, a shaded
 * 95 % band (two stacked line series, the first invisible), and the sentence
 * for the note under the chart. Q5: when the forecast refuses, the sentence
 * is shown and no line is drawn; nothing here fills in for a refusal.
 */

/** What the chart knows about its rows, for a forecast; set by the tile (`LineContext.buckets`). */
export type ForecastBuckets = {
  grain: Grain | null;
  /** The raw bucket of each row (`YYYY-MM-DD[ HH:MM:SS]`), in the order the chart shows them. */
  raw: readonly unknown[];
  /** The report time zone: "now" there decides whether the last period is finished. */
  timeZone: string;
  /** The clock, epoch ms; tests pass it. */
  now?: number;
  /** A chart with a breakdown has one series per group, which a single forecast cannot describe. */
  breakdown?: boolean;
};

export type ForecastPlan = {
  /** The forecast, or the sentence for the note when there is none. */
  result: Forecast | { refused: string };
  /** The period grid the forecast was fitted on; null when the rows were not a run of dates. */
  periods: Periods | null;
  /** Labels of the periods ahead, `result.point.length` of them. */
  labels: string[];
  /** How many of the chart's rows are the history the forecast continues. */
  rowsUsed?: number;
  /** The line under the chart. */
  note: string;
};

const cache = new Map<string, ForecastPlan>();

/** Work out a forecast once per set of inputs (the tile asks for the line and for the note). */
export function planForecast(horizon: number, values: readonly number[], src: ForecastBuckets, horizontal: boolean): ForecastPlan {
  const refuse = (sentence: string): ForecastPlan => ({ result: { refused: sentence }, periods: null, labels: [], note: sentence });
  if (horizontal) return refuse("A forecast is not drawn on horizontal bars.");
  if (src.breakdown) return refuse("A forecast describes one series; remove the breakdown to see it.");
  const grain = src.grain;
  if (!grain || !isTruncation(grain)) {
    return refuse("A forecast needs the dates grouped by minute, hour, day, week, month, quarter or year.");
  }
  const laid = layPeriods(grain, src.raw, values);
  if (!laid) return refuse("The chart's dates could not be read as a run of periods, so there is nothing to forecast from.");
  const wall = wallClockMs(src.now ?? Date.now(), src.timeZone);
  const end = bucketEnd(grain, laid.last);
  // Whether the clock has passed the end of the last period is part of the answer.
  const key = [horizon, grain, laid.starts[0], laid.values.join(","), wall < end ? "open" : "closed"].join("|");
  const hit = cache.get(key);
  if (hit) return hit;
  const result = forecast({ values: laid.values, grain, horizon, lastPeriodEnd: end, now: wall });
  let plan: ForecastPlan;
  if (isRefusal(result)) {
    plan = { result, periods: laid, labels: [], note: result.refused };
  } else {
    // The first forecast period follows the last period the fit used (`lastIndex`): not "now", and not the end of the range.
    const lastUsed = parseBucket(laid.starts[result.lastIndex]) ?? laid.last;
    // The rows up to it are the actuals the forecast continues; later rows (an unfinished or empty end) stay as drawn.
    const rowsUsed = laid.values.slice(0, result.lastIndex + 1).filter((v) => v !== null).length;
    plan = { result, periods: laid, labels: futureLabels(grain, lastUsed, result.point.length), rowsUsed, note: [`Forecast: ${result.label}, 95 % band.`, ...result.notes].join(" ") };
  }
  if (cache.size > 24) cache.clear();
  cache.set(key, plan);
  return plan;
}

type Obj = Record<string, unknown>;

/**
 * Add the forecast to `option`: its labels on the category axis, the point
 * line and the band. `measureAxis` is the value axis of the measure, `rowCount`
 * the number of categories already on the axis (one per row).
 */
export function withForecastSeries(
  option: EChartsOption,
  plan: ForecastPlan,
  opts: { name: string; color: string; seriesIndex: number; actual: readonly number[]; write: (v: number) => string },
): EChartsOption {
  const result = plan.result;
  if ("refused" in result) return option;
  const series = Array.isArray(option.series) ? (option.series as Obj[]) : [];
  const source = series[opts.seriesIndex];
  if (!source) return option;
  const yAxisIndex = Number((source as { yAxisIndex?: number }).yAxisIndex ?? 0);
  const rows = (Array.isArray(option.xAxis) ? (option.xAxis[0] as Obj) : (option.xAxis as Obj))?.data as unknown[] | undefined;
  if (!rows) return option;
  const h = result.point.length;
  // Position of the first forecast period: after the last row the fit used. When rows follow it
  // (an unfinished period left out) the forecast sits on the category of the next period if the
  // axis already has it, else after all the rows.
  const used = plan.rowsUsed ?? rows.length;
  const nextLabel = plan.labels[0];
  const onAxis = used < rows.length && String(rows[used]) === nextLabel;
  const start = used < rows.length && !onAxis ? rows.length : used;
  const lastActual = start === used ? opts.actual[start - 1] : undefined;
  // Categories beyond the axis's end are the periods ahead; one the axis already holds is not repeated.
  const extra = plan.labels.slice(Math.max(0, rows.length - start));
  const nulls = (n: number) => Array<number | null>(n).fill(null);
  const pad = (head: (number | null)[]) => [...nulls(start - 1), ...head];
  const lowerData = pad([lastActual ?? null, ...result.lower]);
  const widthData = pad([0, ...result.upper.map((u, i) => u - result.lower[i])]);
  const pointData = pad([lastActual ?? null, ...result.point]);
  const common = { type: "line" as const, yAxisIndex, symbol: "none", silent: true, smooth: false };
  const bandName = `${opts.name} band`;
  const added: Obj[] = [
    { ...common, name: `${bandName} (lower)`, stack: "forecast-band", data: lowerData, lineStyle: { opacity: 0 }, tooltip: { show: false }, z: 1 },
    { ...common, name: bandName, stack: "forecast-band", data: widthData, lineStyle: { opacity: 0 }, areaStyle: { color: opts.color, opacity: 0.15 }, tooltip: { show: false }, z: 1 },
    {
      ...common, name: opts.name, data: pointData, z: 3,
      lineStyle: { type: "dashed", width: 2, color: opts.color }, itemStyle: { color: opts.color },
      tooltip: {
        valueFormatter: (v: unknown, i?: number) => {
          const k = (i ?? 0) - start;
          if (typeof v !== "number" || k < 0 || k >= h) return typeof v === "number" ? opts.write(v) : String(v);
          return `${opts.write(v)} (95 % band ${opts.write(result.lower[k])} to ${opts.write(result.upper[k])})`;
        },
      },
    },
  ];
  const widenAxis = (axis: unknown) => (axis && typeof axis === "object" ? { ...(axis as Obj), data: [...rows, ...extra] } : axis);
  const legend = option.legend && !Array.isArray(option.legend) && typeof option.legend === "object" ? (option.legend as Obj) : undefined;
  const named = series.map((s) => s.name).filter((n): n is string => typeof n === "string" && n !== "");
  return {
    ...option,
    xAxis: Array.isArray(option.xAxis) ? option.xAxis.map((a, i) => (i === 0 ? widenAxis(a) : a)) : widenAxis(option.xAxis),
    // Keep the helper series out of the legend; only the line is named there.
    ...(legend && !legend.data ? { legend: { ...legend, data: [...named, opts.name] } } : {}),
    series: [...series, ...added],
  } as EChartsOption;
}
