"use client";

import * as React from "react";
import * as echarts from "echarts";

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
}: {
  option: echarts.EChartsOption;
  height?: number | string;
  /** Click a data point (bar/slice/etc.) → drill/cross-filter. `pos` = screen coordinates. */
  onDataClick?: (name: string, pos: { x: number; y: number }) => void;
}) {
  const elRef = React.useRef<HTMLDivElement>(null);
  const chartRef = React.useRef<echarts.ECharts | null>(null);
  // Keep the callback in a ref so the click handler always uses the latest
  // version without re-initializing the chart.
  const clickRef = React.useRef(onDataClick);
  React.useEffect(() => { clickRef.current = onDataClick; }, [onDataClick]);

  React.useEffect(() => {
    if (!elRef.current) return;
    const chart = echarts.init(elRef.current, undefined, { renderer: "canvas" });
    chartRef.current = chart;
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
      chart.dispose();
      chartRef.current = null;
    };
  }, []);

  React.useEffect(() => {
    // notMerge=true so theme/option changes are clean (no stacking of old series).
    chartRef.current?.setOption(option, true);
  }, [option]);

  return <div ref={elRef} style={{ width: "100%", height, cursor: onDataClick ? "pointer" : "default" }} />;
}
