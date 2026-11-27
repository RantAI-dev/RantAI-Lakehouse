"use client";

import * as React from "react";
import * as echarts from "echarts";
import type { ChartHit } from "@/lib/chart-click";
import { readView, withView } from "@/lib/geo-view";
import { fitCategoryLabels, fitSignature } from "./category-labels";
import { useRenderMode } from "./render-mode";

/**
 * BI-32: a stable fingerprint of an option, for "is this the same chart?".
 * The tile rebuilds its option on every render, so its identity says nothing;
 * a function (a formatter) is compared by its source, since a fresh closure
 * of the same code draws the same text.
 */
export function optionKey(option: unknown): string {
  return JSON.stringify(option, (_k, v) => (typeof v === "function" ? `fn:${Function.prototype.toString.call(v)}` : v));
}

/**
 * A thin Apache ECharts wrapper for React 19 — no echarts-for-react, for
 * full control (React 19) and a single dependency. Takes a finished
 * `option` (built theme-aware in chart-option.ts) and handles
 * init / setOption / resize / dispose.
 */
export function EChart({
  option,
  height = 280,
  onDataClick,
  keepView,
  onChart,
  live,
  optionDeps,
}: {
  option: echarts.EChartsOption;
  height?: number | string;
  /**
   * Click a data point (bar/slice/region/day/node…) → drill/cross-filter.
   * `pos` = screen coordinates. The whole hit is passed on, not just its
   * name: a calendar day has none, and a box's name is a label (BI-18·B,
   * lib/chart-click.ts maps each kind).
   */
  onDataClick?: (hit: ChartHit, pos: { x: number; y: number }) => void;
  /**
   * Maps only: when `option` changes (a data refresh, a theme switch) carry
   * the map's current pan and zoom over instead of letting `setOption` reset
   * it to the initial fit. A different map must remount (`key`), since the
   * old centre means nothing on it.
   */
  keepView?: boolean;
  /** Receives the ECharts instance once created (and `null` on dispose), for zoom controls. */
  onChart?: (chart: echarts.ECharts | null) => void;
  /**
   * A chart that stays on screen while its data changes (the sign-in page's
   * sample dashboard): after the first draw, a changed `option` is merged
   * into the chart instead of replacing it, so bars and points morph from
   * their current values in 500 ms (`cubicOut`) and nothing regrows from
   * zero. Absent, every change is a clean replace, as for the rest of the console.
   */
  live?: boolean;
  /**
   * BI-32 review: what the option's formatter functions close over (number
   * format, locale, decimals, theme…). Functions are compared by source, so
   * the same code over other values would look unchanged; pass those values
   * here and a change of them applies the option again.
   */
  optionDeps?: unknown;
}) {
  const elRef = React.useRef<HTMLDivElement>(null);
  const chartRef = React.useRef<echarts.ECharts | null>(null);
  // Keep the callback in a ref so the click handler always uses the latest
  // version without re-initializing the chart.
  const clickRef = React.useRef(onDataClick);
  React.useEffect(() => { clickRef.current = onDataClick; }, [onDataClick]);
  const chartCbRef = React.useRef(onChart);
  React.useEffect(() => { chartCbRef.current = onChart; }, [onChart]);
  const hasOption = React.useRef(false);
  // BI-73: the option as built (before it was fitted to the tile's width) and what the fit decided.
  const rawRef = React.useRef<echarts.EChartsOption | null>(null);
  const fitRef = React.useRef<string | null>(null);
  // BI-32: the fingerprint of the option the chart now shows.
  const appliedKey = React.useRef<string | null>(null);
  // BI-1: under the PDF print view the chart draws without animation and
  // tells the view when it has drawn (see `render-mode.tsx`).
  const renderMode = useRenderMode();

  React.useEffect(() => {
    if (!elRef.current) return;
    const chart = echarts.init(elRef.current, undefined, { renderer: "canvas" });
    chartRef.current = chart;
    let reported = false;
    if (renderMode) {
      renderMode.chartMounted();
      chart.on("finished", () => {
        if (reported) return;
        reported = true;
        renderMode.chartFinished();
      });
    }
    chartCbRef.current?.(chart);
    chart.on("click", (p: unknown) => {
      const o = p as {
        name?: string; value?: unknown; dataIndex?: number; seriesType?: string; dataType?: string; seriesIndex?: number;
        treePathInfo?: { name?: string }[]; event?: { event?: MouseEvent };
      };
      if (!clickRef.current) return;
      const ev = o.event?.event;
      clickRef.current(
        {
          name: o.name ?? "", value: o.value, dataIndex: o.dataIndex, seriesType: o.seriesType, dataType: o.dataType, seriesIndex: o.seriesIndex,
          treePathInfo: o.treePathInfo?.map((t) => ({ name: t.name ?? "" })),
        },
        { x: ev?.clientX ?? 0, y: ev?.clientY ?? 0 },
      );
    });
    const ro = new ResizeObserver(() => {
      chart.resize();
      // BI-73 (a, b): the label rule follows the tile's width (a drag, a side panel).
      const el = elRef.current;
      const raw = rawRef.current;
      if (!el || !raw) return;
      const fitted = fitCategoryLabels(raw, el.clientWidth);
      const sig = fitSignature(fitted);
      if (sig === fitRef.current) return;
      fitRef.current = sig;
      // A clean replace (a merge would keep the old label width), without replaying the entrance.
      chart.setOption({ ...fitted, animation: false }, true);
    });
    ro.observe(elRef.current);
    return () => {
      ro.disconnect();
      // A chart removed before it finished must not hold the print view back.
      if (renderMode && !reported) renderMode.chartFinished();
      chartCbRef.current?.(null);
      chart.dispose();
      chartRef.current = null;
      hasOption.current = false;
      appliedKey.current = null;
    };
  }, [renderMode]);

  React.useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    // BI-32: the dashboard page re-renders with every streamed assistant
    // message and rebuilds each tile's option, equal in content but new in
    // identity. `setOption(…, true)` replays the entrance animation (a
    // reference line redrawn from the left), so an option that says the same
    // as the one on screen is not applied again.
    const key = `${keepView ? "v" : ""}${renderMode ? "r" : ""}${live ? "l" : ""}${optionKey(optionDeps)}|${optionKey(option)}`;
    if (appliedKey.current === key) return;
    appliedKey.current = key;
    // The first option has no view to keep (getOption would be empty).
    rawRef.current = option;
    const sized = fitCategoryLabels(option, elRef.current?.clientWidth ?? 0);
    fitRef.current = fitSignature(sized);
    const base = keepView && hasOption.current ? withView(sized, readView(chart.getOption())) : sized;
    const timed = live ? { ...base, animationDurationUpdate: 500, animationEasingUpdate: "cubicOut" as const } : base;
    const next = renderMode ? { ...timed, animation: false } : timed;
    // notMerge=true so theme/option changes are clean (no stacking of old series);
    // a `live` chart merges after its first draw so its data morphs in place.
    chart.setOption(next, !(live && hasOption.current));
    hasOption.current = true;
  }, [option, optionDeps, keepView, renderMode, live]);

  return <div ref={elRef} style={{ width: "100%", height, cursor: onDataClick ? "pointer" : "default" }} />;
}
