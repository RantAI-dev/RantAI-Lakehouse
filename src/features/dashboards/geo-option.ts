import type { EChartsOption } from "echarts";
import type { ChartSpec } from "@/lib/dashboard-specs";
import { escapeHtml, pointLimit, pointSize, toPoints } from "@/lib/geo-points";
import { matchRegionRows } from "@/lib/geo-regions";
import { ZOOM_MAX, ZOOM_MIN } from "@/lib/geo-view";
import { fmtInt } from "./chart-option";
import { DEFAULT_CHOROPLETH_MAP, DEFAULT_POINT_MAP, mapEntry, mapFeatureNames } from "./echarts-maps";

/** What the three map kinds need of a chart spec. */
export type GeoSpec = Pick<ChartSpec, "kind" | "x" | "y" | "map" | "lat" | "lon" | "sqlSource">;

/** A quiet line under the map, saying what the map does not show. */
export type GeoNotice = { text: string; title?: string };

export type GeoBuild = { option: EChartsOption; notices: GeoNotice[] };

type Row = Record<string, unknown>;

/** The map a spec is drawn on: its own, else the default for its kind. */
export function resolveMapId(spec: Pick<ChartSpec, "kind" | "map">): string {
  return spec.map || (spec.kind === "geomap" ? DEFAULT_CHOROPLETH_MAP : DEFAULT_POINT_MAP);
}

/** A cell as a number; a missing or non-numeric cell is NaN, never 0. */
const cellNumber = (v: unknown) => (v === null || v === undefined || v === "" ? Number.NaN : Number(v));

const plural = (n: number, one: string, many: string) => `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;

/** How many unmatched names the tooltip of the notice lists. */
const NAMES_SHOWN = 5;

function unmatchedNotice(rows: number, names: string[]): GeoNotice {
  const shown = names.slice(0, NAMES_SHOWN).join(", ");
  const more = names.length > NAMES_SHOWN ? ` and ${names.length - NAMES_SHOWN} more` : "";
  return { text: `${plural(rows, "row", "rows")} matched no region`, title: `Not matched: ${shown}${more}` };
}

/** Indigo ramp shared with the other maps and heatmaps of the console. */
const RAMP = (dark: boolean) =>
  dark ? ["#1e1b4b", "#4f46e5", "#a5b4fc"] : ["#eef2ff", "#818cf8", "#3730a3"];
/**
 * Symbol ramp: unlike a region, a point has no fill of its own to stand out
 * from the land, so its palest colour must still show on the land colour
 * (#f4f4f5 light, #27272a dark); the choropleth ramp's would vanish.
 */
const POINT_RAMP = (dark: boolean) =>
  dark ? ["#6366f1", "#a5b4fc", "#e0e7ff"] : ["#a5b4fc", "#4f46e5", "#312e81"];
/**
 * Density ramp: faint where there is little, so the outline still shows
 * through, then warmer. Every stop stays visible on both the dark
 * (#27272a) and the light (#f4f4f5) land colour.
 */
const HEAT_RAMP = ["#6366f1", "#0ea5e9", "#10b981", "#facc15", "#ef4444"];

export function buildGeoOption(spec: GeoSpec, rows: Row[], dark: boolean): GeoBuild {
  const mapId = resolveMapId(spec);
  const axis = dark ? "#a1a1aa" : "#71717a";
  const tooltip = {
    backgroundColor: dark ? "#18181b" : "#ffffff",
    borderWidth: 0,
    textStyle: { color: dark ? "#e4e4e7" : "#18181b", fontSize: 12 },
  };
  const yCol = Array.isArray(spec.y) ? spec.y[0] : spec.y;
  const level = mapEntry(mapId)?.level ?? "province";
  // Region outlines are fine-grained on the kabupaten map, so thinner there.
  const border = level === "regency" ? 0.4 : 1;
  const land = {
    areaColor: dark ? "#27272a" : "#f4f4f5",
    // A shade off the land colour, not the page colour: on a choropleth
    // most regions often have no value, and with borders the colour of the
    // page the map read as an empty tile (seen on the kabupaten map).
    borderColor: dark ? "#52525b" : "#d4d4d8",
    borderWidth: border,
  };
  // Drag to pan; NO wheel zoom (a wheel over a tile must keep scrolling the
  // dashboard page, and ECharts only swallows the wheel when roam includes
  // zoom). Zoom comes from the buttons of GeoChart, within 1x-20x.
  const roam = { roam: "move" as const, scaleLimit: { min: ZOOM_MIN, max: ZOOM_MAX }, aspectScale: 1 };
  const base = { textStyle: { color: axis, fontFamily: "inherit" } };

  if (spec.kind === "geomap") {
    const named = rows.map((r) => ({ name: String(r[spec.x] ?? ""), value: cellNumber(r[yCol]) }));
    const match = matchRegionRows(named, level, mapFeatureNames(mapId));
    const maxV = Math.max(1, ...match.data.map((d) => d.value));
    const notices = match.unmatchedRows > 0 ? [unmatchedNotice(match.unmatchedRows, match.unmatchedNames)] : [];
    const option: EChartsOption = {
      ...base,
      tooltip: {
        ...tooltip, trigger: "item",
        formatter: (p: unknown) => {
          const o = p as { name: string; value?: number };
          return `${escapeHtml(o.name)}<br/><b>${o.value != null && !Number.isNaN(o.value) ? fmtInt(o.value) : "—"}</b>`;
        },
      },
      visualMap: {
        min: 0, max: maxV, left: "left", bottom: 6, calculable: true, itemHeight: 70,
        formatter: (v: unknown) => fmtInt(Number(v)),
        textStyle: { color: axis, fontSize: 10 }, inRange: { color: RAMP(dark) },
      },
      series: [{
        type: "map", map: mapId, ...roam,
        itemStyle: land,
        emphasis: { label: { show: true, color: dark ? "#fff" : "#111", fontSize: 10 }, itemStyle: { areaColor: "#f59e0b" } },
        select: { disabled: true }, label: { show: false }, data: match.data,
      }],
    };
    return { option, notices };
  }

  // Point kinds: the outline is a `geo` component, the rows are drawn on it.
  const limit = pointLimit(spec.sqlSource);
  const set = toPoints(rows, { lat: spec.lat ?? "", lon: spec.lon ?? "", value: yCol, label: spec.x || undefined }, limit);
  const notices: GeoNotice[] = [];
  if (set.dropped > 0) {
    notices.push({
      text: `${plural(set.dropped, "row", "rows")} left out: no usable latitude, longitude or value`,
      title: "Latitude must be within -90 to 90 and longitude within -180 to 180.",
    });
  }
  if (set.capped) {
    // Said as the rule the query follows, not as a count of what is missing:
    // a result of exactly the limit cannot tell how many rows were cut.
    notices.push({ text: `The map shows the top ${limit.toLocaleString("en-US")} rows by value` });
  }
  const values = set.points.map((p) => p.value);
  const maxV = values.length ? Math.max(...values) : 0;
  const minV = values.length ? Math.min(...values) : 0;
  const lo = Math.min(0, minV);
  const hi = maxV > lo ? maxV : lo + 1;
  const visualMap = (colors: string[], calculable: boolean) => ({
    type: "continuous" as const, min: lo, max: hi, dimension: 2, seriesIndex: 0,
    left: "left" as const, bottom: 6, calculable, itemHeight: 70,
    // A bar without handles shows its min and max as text instead; one with
    // handles already labels them.
    ...(calculable ? { formatter: (v: unknown) => fmtInt(Number(v)) } : { text: [fmtInt(hi), fmtInt(lo)] }),
    textStyle: { color: axis, fontSize: 10 }, inRange: { color: colors },
  });
  const geo = { map: mapId, ...roam, silent: true, label: { show: false }, itemStyle: land };

  if (spec.kind === "geoheat") {
    const option: EChartsOption = {
      ...base, tooltip: { show: false }, geo,
      visualMap: visualMap(HEAT_RAMP, false),
      series: [{
        type: "heatmap", coordinateSystem: "geo", pointSize: 9, blurSize: 16,
        data: set.points.map((p) => [p.lon, p.lat, p.value]),
      }],
    };
    return { option, notices };
  }

  // Largest first, so the small symbols are drawn on top of the big ones.
  const ordered = [...set.points].sort((a, b) => b.value - a.value);
  const option: EChartsOption = {
    ...base,
    tooltip: {
      ...tooltip, trigger: "item",
      formatter: (p: unknown) => {
        const o = p as { name: string; value: number[] };
        const [lon, lat, value] = o.value;
        // No label column: the coordinates are the only honest name.
        const title = o.name ? escapeHtml(o.name) : `${lat.toFixed(4)}, ${lon.toFixed(4)}`;
        return `${title}<br/><b>${fmtInt(value)}</b>`;
      },
    },
    geo,
    visualMap: visualMap(POINT_RAMP(dark), true),
    series: [{
      type: "scatter", coordinateSystem: "geo",
      symbolSize: (v: unknown) => pointSize((v as number[])[2], maxV),
      itemStyle: { opacity: 0.85, borderColor: dark ? "#09090b" : "#ffffff", borderWidth: 0.6 },
      data: ordered.map((p) => ({ name: p.label, value: [p.lon, p.lat, p.value] })),
    }],
  };
  return { option, notices };
}
