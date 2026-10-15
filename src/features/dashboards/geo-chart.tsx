"use client";

import * as React from "react";
import type { ECharts } from "echarts";
import { Minus, Plus, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ChartClickHandler } from "@/lib/chart-click";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";
import { readView, stepZoom, ZOOM_MAX, ZOOM_MIN } from "@/lib/geo-view";
import { EChart } from "./echart";
import { ensureMap, mapEntry } from "./echarts-maps";
import { buildGeoOption, regionNamesFor, resolveMapId } from "./geo-option";

type Row = Record<string, unknown>;

/** Pan/zoom control: quiet until the tile is hovered or focused, always shown on touch. */
const CONTROLS_CLASS =
  "absolute top-1.5 right-1.5 flex flex-col gap-1 opacity-0 transition-opacity duration-200 group-hover/geo:opacity-100 group-focus-within/geo:opacity-100 [@media(hover:none)]:opacity-100 print:hidden";
const CONTROL_BUTTON_CLASS = "border-border bg-background/85 shadow-xs backdrop-blur-sm";

/**
 * The three map kinds (`geomap`, `pointmap`, `geoheat`). Loads the bundled
 * map first (local GeoJSON) and says plainly when it cannot be drawn: an id
 * the console has no map for, or a GeoJSON file that is missing.
 */
export function GeoChart({ spec, rows, dark, onDataClick }: {
  spec: ChartRenderSpec; rows: Row[]; dark: boolean; onDataClick?: ChartClickHandler;
}) {
  const mapId = resolveMapId(spec);
  const entry = mapEntry(mapId);
  // The outcome is keyed by the map it is for, so switching maps shows the
  // loading state until the new one answers (no state reset inside the effect).
  const [loaded, setLoaded] = React.useState<{ id: string; ok: boolean } | null>(null);
  React.useEffect(() => {
    if (!entry) return;
    let alive = true;
    void ensureMap(mapId).then((ok) => { if (alive) setLoaded({ id: mapId, ok }); });
    return () => { alive = false; };
  }, [mapId, entry]);

  if (!entry) {
    return (
      <div className="grid h-full place-content-center px-4 text-center text-xs text-muted-foreground">
        <p className="font-medium text-foreground">Map not available</p>
        <p className="mt-1">This console has no map called <code className="rounded bg-muted px-1 font-mono">{mapId}</code>.</p>
      </div>
    );
  }
  if (loaded?.id !== mapId) return <div className="h-full animate-pulse rounded bg-muted/40" />;
  if (!loaded.ok) {
    return (
      <div className="grid h-full place-content-center px-4 text-center text-xs text-muted-foreground">
        <p className="font-medium text-foreground">Map data not loaded</p>
        <p className="mt-1">Add <code className="rounded bg-muted px-1 font-mono">public{entry.url}</code> to enable this map.</p>
      </div>
    );
  }
  // A different map or kind starts from its own fit: the old pan and zoom
  // are coordinates of another map.
  return <ReadyMap key={`${spec.kind}:${mapId}`} spec={spec} rows={rows} dark={dark} onDataClick={onDataClick} />;
}

function ReadyMap({ spec, rows, dark, onDataClick }: {
  spec: ChartRenderSpec; rows: Row[]; dark: boolean; onDataClick?: ChartClickHandler;
}) {
  const { option, notices } = React.useMemo(() => buildGeoOption(spec, rows, dark), [spec, rows, dark]);
  // A region is drilled by the spelling the column stores, which only the
  // rows know (geo-option.ts `regionNamesFor`); computed once per data load.
  const regionNames = React.useMemo(() => regionNamesFor(spec, rows), [spec, rows]);
  const [chart, setChart] = React.useState<ECharts | null>(null);
  const [zoom, setZoom] = React.useState(ZOOM_MIN);

  // Keep the zoom buttons' disabled state in step with drags and zooms.
  React.useEffect(() => {
    if (!chart) return;
    const sync = () => {
      if (chart.isDisposed()) return;
      const view = readView(chart.getOption());
      if (view) setZoom(view.zoom);
    };
    chart.on("georoam", sync);
    return () => { if (!chart.isDisposed()) chart.off("georoam", sync); };
  }, [chart]);

  // Zoom through the same action a wheel or pinch would dispatch, about the
  // middle of the tile, so ECharts applies the 1x-20x limit and keeps the
  // view consistent. A `map` series answers to `seriesIndex`, a `geo`
  // component to `geoIndex` (checked against ECharts 6.1 in a headless run).
  function zoomBy(direction: 1 | -1) {
    if (!chart || chart.isDisposed()) return;
    const current = readView(chart.getOption())?.zoom ?? ZOOM_MIN;
    const next = stepZoom(current, direction);
    if (next === current) return;
    const target = "geo" in option ? { componentType: "geo", geoIndex: 0 } : { seriesIndex: 0 };
    chart.dispatchAction({
      type: "geoRoam", ...target, zoom: next / current,
      originX: chart.getWidth() / 2, originY: chart.getHeight() / 2,
    });
  }

  // `notMerge` with the option that has no view: back to the initial fit.
  function reset() {
    if (!chart || chart.isDisposed()) return;
    chart.setOption(option, true);
    setZoom(ZOOM_MIN);
  }

  return (
    <div className="flex h-full flex-col">
      <div className="group/geo relative min-h-0 flex-1">
        <div className="absolute inset-0">
          <EChart option={option} height="100%" keepView onChart={setChart}
            onDataClick={onDataClick ? (hit, pos) => onDataClick(hit, pos, { regionNames }) : undefined} />
        </div>
        <div className={CONTROLS_CLASS} role="group" aria-label="Map zoom">
          <Button variant="ghost" size="icon-xs" className={CONTROL_BUTTON_CLASS} aria-label="Zoom in" title="Zoom in"
            disabled={zoom >= ZOOM_MAX} onClick={() => zoomBy(1)}>
            <Plus />
          </Button>
          <Button variant="ghost" size="icon-xs" className={CONTROL_BUTTON_CLASS} aria-label="Zoom out" title="Zoom out"
            disabled={zoom <= ZOOM_MIN} onClick={() => zoomBy(-1)}>
            <Minus />
          </Button>
          <Button variant="ghost" size="icon-xs" className={CONTROL_BUTTON_CLASS} aria-label="Reset map view" title="Reset view"
            onClick={reset}>
            <RotateCcw />
          </Button>
        </div>
      </div>
      {notices.length ? (
        <p className="shrink-0 pt-1 text-[11px] leading-snug text-muted-foreground">
          {notices.map((n, i) => (
            <span key={n.text} title={n.title} className={n.title ? "cursor-help underline decoration-dotted underline-offset-2" : undefined}>
              {i > 0 ? " · " : ""}{n.text}
            </span>
          ))}
        </p>
      ) : null}
    </div>
  );
}
