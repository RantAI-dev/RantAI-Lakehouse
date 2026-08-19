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
  onDataClick,
}: {
  option: echarts.EChartsOption;
  height?: number | string;
  /** Klik titik data (bar/irisan/dll) → drill/cross-filter. `pos` = koordinat layar. */
  onDataClick?: (name: string, pos: { x: number; y: number }) => void;
}) {
  const elRef = React.useRef<HTMLDivElement>(null);
  const chartRef = React.useRef<echarts.ECharts | null>(null);
  // Simpan callback di ref supaya handler klik selalu pakai versi terbaru
  // tanpa re-init chart.
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
    // notMerge=true agar ganti tema/opsi bersih (tidak menumpuk seri lama).
    chartRef.current?.setOption(option, true);
  }, [option]);

  return <div ref={elRef} style={{ width: "100%", height, cursor: onDataClick ? "pointer" : "default" }} />;
}
