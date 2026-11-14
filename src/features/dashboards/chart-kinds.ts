/**
 * The chart types the builder offers (`BI-58` T3 moved them here from
 * `chart-builder.tsx`, which still exports `KIND_GROUPS`): the groups, a
 * one-sentence description of each, the extra words the type search
 * matches, and each type's icon. Shared by the canvas's type toolbar, its
 * gallery and the builder.
 */
import {
  CalendarDays, ChartArea, ChartBar, ChartCandlestick, ChartColumn, ChartColumnStacked, ChartGantt, ChartLine,
  ChartNoAxesColumn, ChartNoAxesCombined, ChartPie, ChartScatter, Flame, Funnel, Gauge, Globe, Grid3x3,
  Image as ImageIcon, LayoutGrid, Map, MapPin, SquareSigma, Table2, Target, Type, Workflow, type LucideIcon,
} from "lucide-react";
import type { ChartKind } from "@/lib/dashboard-specs";

/**
 * Chart types grouped by the question they answer (Metabase/Tableau-style
 * visualization picker). Exported so a caller can offer the type choice
 * earlier (e.g. a "New chart" dropdown in a header) without duplicating
 * the list.
 */
export const KIND_GROUPS: { group: string; items: { value: ChartKind; label: string }[] }[] = [
  { group: "Comparison", items: [
    { value: "bar", label: "Bar" },
    { value: "hbar", label: "Horizontal bar (ranking)" },
    { value: "stacked", label: "Stacked bar (≥2 measures)" },
    { value: "combo", label: "Combo (bar + line)" },
  ] },
  { group: "Trend", items: [
    { value: "line", label: "Line" },
    { value: "area", label: "Area" },
    { value: "waterfall", label: "Waterfall (cumulative)" },
    { value: "calendar", label: "Calendar heatmap (daily)" },
  ] },
  { group: "Composition", items: [
    { value: "pie", label: "Donut / pie" },
    { value: "rose", label: "Rose (nightingale)" },
    { value: "funnel", label: "Funnel" },
    { value: "treemap", label: "Treemap" },
    { value: "sunburst", label: "Sunburst (2 levels)" },
  ] },
  { group: "Relationship / distribution", items: [
    { value: "scatter", label: "Scatter (X vs Y)" },
    { value: "bubble", label: "Bubble (X, Y, size)" },
    { value: "heatmap", label: "Heatmap (2 dimensions)" },
    { value: "radar", label: "Radar" },
    { value: "boxplot", label: "Box plot (distribution)" },
    { value: "histogram", label: "Histogram (spread of a number)" },
  ] },
  { group: "Schedule", items: [
    { value: "gantt", label: "Gantt (tasks over time)" },
  ] },
  { group: "Flow", items: [
    { value: "sankey", label: "Sankey (flow between 2 dimensions)" },
  ] },
  { group: "Geographic", items: [
    { value: "geomap", label: "Map — regions (choropleth)" },
    { value: "pointmap", label: "Map — points / bubbles" },
    { value: "geoheat", label: "Map — density heatmap" },
  ] },
  { group: "Single value", items: [
    { value: "kpi", label: "KPI — big number" },
    { value: "gauge", label: "Gauge" },
    { value: "progress", label: "Progress bar (value vs goal)" },
    { value: "bullet", label: "Bullet graph (value, target, bands)" },
  ] },
  { group: "Other", items: [
    { value: "table", label: "Data table" },
    { value: "pivot", label: "Pivot table" },
    { value: "text", label: "Text / note" },
    { value: "image", label: "Image" },
    { value: "web", label: "Web page" },
  ] },
];
export const KIND_DESCRIPTIONS: Record<ChartKind, string> = {
  bar: "Compare values across categories", hbar: "Rank categories clearly",
  stacked: "Compare totals and their parts", combo: "Compare two metrics on different scales",
  line: "Show change over time", area: "Show trend and magnitude",
  waterfall: "Explain contributions to a total", pie: "Show parts of a whole",
  rose: "Compare composition with radial bars", funnel: "Show drop-off through stages",
  treemap: "Compare hierarchical proportions", scatter: "Reveal correlation between two metrics",
  bubble: "Compare relationships with a third metric", heatmap: "Find patterns across two dimensions",
  radar: "Compare profiles across metrics", geomap: "Colour the regions of a map by value",
  pointmap: "Plot locations as points sized and coloured by a value",
  geoheat: "Show where locations are concentrated",
  kpi: "Highlight one important number", gauge: "Track a value against a target",
  table: "Inspect detailed rows and values", pivot: "Cross-tabulate with totals and subtotals", text: "Add context, notes, or instructions",
  sankey: "Show how a total flows from one dimension to another",
  sunburst: "Break each category into its parts, as rings",
  boxplot: "Compare the spread of a measure across categories",
  calendar: "Spot daily patterns on a calendar",
  progress: "Show a value against its goal, with the percentage reached",
  histogram: "Count how many rows fall in each range of a number",
  gantt: "Lay tasks on a time axis, one bar from start to end, with a line at today",
  bullet: "Compare a value with a target and ranges such as poor, fair and good",
  image: "Show a picture from an allowed site", web: "Show a web page from an allowed site",
  // BI-18 part C: added from the dashboard's "Add" menu, never picked here; present only to satisfy the record.
  heading: "A heading between tiles", divider: "A rule between tiles", section: "A heading that folds the tiles below it",
};
/**
 * Extra words the type search matches, beyond label and description: the
 * Indonesian ones (peta = map, titik = point, lokasi = location, kepadatan =
 * density) a user of this console is as likely to type.
 */
export const KIND_KEYWORDS: Partial<Record<ChartKind, string>> = {
  geomap: "map peta region wilayah choropleth spatial geo",
  pointmap: "map peta points titik lokasi bubble spatial geo",
  geoheat: "map peta heatmap density kepadatan spatial geo",
  pivot: "pivot cross tab crosstab matrix totals subtotals",
  table: "table rows raw columns grid",
  histogram: "distribution spread frequency bins ranges buckets",
  gantt: "timeline schedule project plan tasks milestones jadwal",
  progress: "goal target percent percentage bar completion",
  bullet: "target bands goal poor fair good performance",
  image: "picture logo photo png jpg gambar",
  web: "iframe page embed website url status halaman",
};
/** Each kind's label, by value. */
export const KIND_LABELS: Record<string, string> = Object.fromEntries(
  KIND_GROUPS.flatMap((g) => g.items.map((i) => [i.value, i.label]))
);

/**
 * One icon per kind, so the eight in the toolbar read apart (layout A of
 * 2026-11-14); kinds of one family that are never side by side may share.
 */
const KIND_ICONS: Partial<Record<ChartKind, LucideIcon>> = {
  bar: ChartColumn, hbar: ChartBar, stacked: ChartColumnStacked, combo: ChartNoAxesCombined,
  line: ChartLine, area: ChartArea, waterfall: ChartColumn, calendar: CalendarDays,
  pie: ChartPie, rose: ChartPie, funnel: Funnel, treemap: LayoutGrid, sunburst: ChartPie,
  scatter: ChartScatter, bubble: ChartScatter, heatmap: Grid3x3, radar: ChartScatter, boxplot: ChartCandlestick,
  histogram: ChartNoAxesColumn, gantt: ChartGantt, sankey: Workflow,
  geomap: Map, pointmap: MapPin, geoheat: Flame,
  kpi: SquareSigma, gauge: Gauge, progress: Gauge, bullet: Target,
  table: Table2, pivot: Table2, text: Type, image: ImageIcon, web: Globe,
};
export function kindIcon(kind: ChartKind): LucideIcon {
  return KIND_ICONS[kind] ?? Type;
}

/**
 * BI-58 review round 3, SHOULD-FIX 8 (owner): the gallery's card titles, short enough never to be
 * cut off. The long labels above stay for the toolbar's names and tooltips and for the search.
 */
export const KIND_SHORT: Partial<Record<ChartKind, string>> = {
  bar: "Bar", hbar: "Horizontal bar", stacked: "Stacked bar", combo: "Combo",
  line: "Line", area: "Area", waterfall: "Waterfall", calendar: "Calendar heatmap",
  pie: "Donut", rose: "Rose", funnel: "Funnel", treemap: "Treemap", sunburst: "Sunburst",
  scatter: "Scatter", bubble: "Bubble", heatmap: "Heatmap", radar: "Radar", boxplot: "Box plot", histogram: "Histogram",
  gantt: "Gantt", sankey: "Sankey",
  geomap: "Region map", pointmap: "Point map", geoheat: "Density map",
  kpi: "KPI", gauge: "Gauge", progress: "Progress bar", bullet: "Bullet graph",
  table: "Data table", pivot: "Pivot table", text: "Text note", image: "Image", web: "Web page",
};

/** A kind's short name, or its label when it has none. */
export function kindShort(kind: ChartKind): string {
  return KIND_SHORT[kind] ?? KIND_LABELS[kind] ?? kind;
}

/**
 * BI-58 review round 4, SHOULD-FIX 9 (owner): the gallery's description of a kind, at most two
 * lines on its card (about 220 px at 1366). The kinds whose full sentence is longer say it shorter
 * here; the full one stays the tooltip and the search text.
 */
const KIND_BLURB: Partial<Record<ChartKind, string>> = {
  gantt: "Tasks on a time axis, from start to end",
  bullet: "A value against a target and its ranges",
  progress: "A value against its goal, in percent",
  pointmap: "Places as points, sized by a value",
  sankey: "How a total flows between two groups",
  boxplot: "The spread of a value in each category",
  histogram: "How many rows fall in each range",
  bubble: "Two values with a third as the size",
  stacked: "Totals and their parts",
  combo: "Two values on different scales",
};

export function kindBlurb(kind: ChartKind): string {
  return KIND_BLURB[kind] ?? KIND_DESCRIPTIONS[kind];
}
