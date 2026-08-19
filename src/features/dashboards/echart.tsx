"use client";

import * as React from "react";
import * as echarts from "echarts";

/**
 * Pembungkus tipis Apache ECharts untuk React 19 — tanpa echarts-for-react,
 * biar kendali penuh (React 19) & satu dependency saja. Menerima `option`
 * yang sudah jadi (dibangun theme-aware di chart-option.ts), lalu urus
 * init / setOption / resize / dispose.
 */
export function EChart({
  option,
  height = 280,
}: {
  option: echarts.EChartsOption;
  height?: number | string;
}) {
  const elRef = React.useRef<HTMLDivElement>(null);
  const chartRef = React.useRef<echarts.ECharts | null>(null);

  React.useEffect(() => {
    if (!elRef.current) return;
    const chart = echarts.init(elRef.current, undefined, { renderer: "canvas" });
    chartRef.current = chart;
    const ro = new ResizeObserver(() => chart.resize());
    ro.observe(elRef.current);
    return () => {
      ro.disconnect();
      chart.dispose();
      chartRef.current = null;
    };
  }, []);

  React.useEffect(() => {
    // notMerge=true agar ganti tema/opsi bersih (tidak menumpuk seri lama).
    chartRef.current?.setOption(option, true);
  }, [option]);

  return <div ref={elRef} style={{ width: "100%", height }} />;
}
