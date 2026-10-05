"use client";

import * as React from "react";
import * as echarts from "echarts";
import { readView, withView } from "@/lib/geo-view";

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
}: {
  option: echarts.EChartsOption;
  height?: number | string;
  /** Click a data point (bar/slice/etc.) → drill/cross-filter. `pos` = screen coordinates. */
  onDataClick?: (name: string, pos: { x: number; y: number }) => void;
  /**
   * Maps only: when `option` changes (a data refresh, a theme switch) carry
   * the map's current pan and zoom over instead of letting `setOption` reset
   * it to the initial fit. A different map must remount (`key`), since the
   * old centre means nothing on it.
   */
  keepView?: boolean;
  /** Receives the ECharts instance once created (and `null` on dispose), for zoom controls. */
  onChart?: (chart: echarts.ECharts | null) => void;
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

  React.useEffect(() => {
    if (!elRef.current) return;
    const chart = echarts.init(elRef.current, undefined, { renderer: "canvas" });
    chartRef.current = chart;
    chartCbRef.current?.(chart);
    chart.on("click", (p: unknown) => {
      const o = p as { name?: string; event?: { event?: MouseEvent } };
      const name = o?.name;
      if (name && clickRef.current) {
        const ev = o.event?.event;
        clickRef.current(name, { x: ev?.clientX ?? 0, y: ev?.clientY ?? 0 });
      }
    });
    const ro = new ResizeObserver(() => chart.resize());
    ro.observe(elRef.current);
    return () => {
      ro.disconnect();
      chartCbRef.current?.(null);
      chart.dispose();
      chartRef.current = null;
      hasOption.current = false;
    };
  }, []);

  React.useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    // The first option has no view to keep (getOption would be empty).
    const next = keepView && hasOption.current ? withView(option, readView(chart.getOption())) : option;
    // notMerge=true so theme/option changes are clean (no stacking of old series).
    chart.setOption(next, true);
    hasOption.current = true;
  }, [option, keepView]);

  return <div ref={elRef} style={{ width: "100%", height, cursor: onDataClick ? "pointer" : "default" }} />;
}
