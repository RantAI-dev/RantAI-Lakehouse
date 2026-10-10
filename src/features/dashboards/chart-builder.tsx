"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import {
  CalendarDays, ChartBar, ChartCandlestick, ChartColumn, ChartLine, ChartPie, ChevronRight, Code, Flame, Gauge, Workflow,
  Map, MapPin, Pencil, Plus, ScatterChart, Search, Table2, Type, type LucideIcon,
} from "lucide-react";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
  DialogFooter, DialogTrigger, DialogClose,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/patterns/page-states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import {
  Select, SelectContent, SelectGroup, SelectLabel, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import {
  decodeSourceChoice, encodeSourceChoice, fieldsQuery, sourcePayload, sourceValueFromDef,
  type ChartSourceChoice,
} from "@/lib/chart-source";
import { useAuth } from "@/features/auth/auth-provider";
import {
  NEW_SQL_CHOICE, fieldsFromColumns, keepIfOffered,
} from "@/lib/sql-source-draft";
import { dashboardService } from "@/services";
import type { SqlSource, SqlSourceColumn } from "@/services/contracts/dashboards";
import { apiFetch } from "@/services/http";
import { filterKindGroups } from "@/lib/chart-kind-search";
import { suggestChartTitle } from "@/lib/chart-title";
import { guessCoordinateColumns, pointLimit } from "@/lib/geo-points";
import { cn } from "@/lib/utils";
import { DEFAULT_CHOROPLETH_MAP, DEFAULT_POINT_MAP, MAP_CATALOGUE, mapCredit } from "./echarts-maps";
import { SqlRowsTable, SqlSourcePanel } from "./sql-source-panel";
import { TileBody } from "./tile-body";
import { useSqlSourceDraft } from "./use-sql-source-draft";
import type { TileFailure } from "@/services/contracts/dashboards";

type Fields = { dimensions: string[]; measures: string[] };
export type ChartDef = {
  title?: string; subtitle?: string; mart?: string; sqlSource?: string; kind?: ChartKind;
  dimension?: string; measures?: string[]; breakdown?: string;
  map?: string; lat?: string; lon?: string;
  aggregate?: string; span?: 1 | 2; board?: string; text?: string; caption?: string;
  order?: "desc" | "asc" | "none"; limit?: number; target?: number;
};
type BoardOpt = { id: string; name: string };
type Preview = {
  spec: ChartRenderSpec & { text?: string; caption?: string };
  result: { columns: string[]; rows: Record<string, unknown>[] } | TileFailure;
};

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
  ] },
  { group: "Other", items: [
    { value: "table", label: "Data table" },
    { value: "text", label: "Text / note" },
  ] },
];
const AGGS = ["sum", "avg", "max", "min", "count"];
/**
 * Value → label maps for the base-ui selects below. Without `items`, a
 * base-ui `Select.Value` renders the raw value (`hbar`, `b_54e05896`,
 * `desc`, `__none__`) in the closed trigger even though the open list shows
 * labels.
 */
const KIND_LABELS: Record<string, string> = Object.fromEntries(
  KIND_GROUPS.flatMap((g) => g.items.map((i) => [i.value, i.label]))
);
const ORDER_LABELS: Record<string, string> = {
  desc: "Highest first", asc: "Lowest first", none: "Natural (by dimension)",
};
const NO_BREAKDOWN = "__none__";
/** Map id → label for the base-ui select (see KIND_LABELS). */
const MAP_LABELS: Record<string, string> = Object.fromEntries(MAP_CATALOGUE.map((m) => [m.id, m.label]));
const NO_LABEL_ITEMS: Record<string, string> = { [NO_BREAKDOWN]: "— no label —" };
const SPAN_LABELS: Record<string, string> = { "1": "Half width", "2": "Full width" };
/** Whether the viewport is at least `px` wide; false on the server. */
function useMinWidth(px: number): boolean {
  const query = `(min-width: ${px}px)`;
  return React.useSyncExternalStore(
    (onChange) => {
      const mql = window.matchMedia(query);
      mql.addEventListener("change", onChange);
      return () => mql.removeEventListener("change", onChange);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

/** Marks a field the chart cannot be drawn without. */
function Req() {
  return <span aria-hidden className="text-destructive">*</span>;
}
const KIND_DESCRIPTIONS: Record<ChartKind, string> = {
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
  table: "Inspect detailed rows and values", text: "Add context, notes, or instructions",
  sankey: "Show how a total flows from one dimension to another",
  sunburst: "Break each category into its parts, as rings",
  boxplot: "Compare the spread of a measure across categories",
  calendar: "Spot daily patterns on a calendar",
};
/**
 * Extra words the type search matches, beyond label and description: the
 * Indonesian ones (peta = map, titik = point, lokasi = location, kepadatan =
 * density) a user of this console is as likely to type.
 */
const KIND_KEYWORDS: Partial<Record<ChartKind, string>> = {
  geomap: "map peta region wilayah choropleth spatial geo",
  pointmap: "map peta points titik lokasi bubble spatial geo",
  geoheat: "map peta heatmap density kepadatan spatial geo",
};
function kindIcon(kind: ChartKind): LucideIcon {
  if (["bar", "hbar", "stacked", "combo"].includes(kind)) return kind === "bar" ? ChartColumn : ChartBar;
  if (["line", "area", "waterfall"].includes(kind)) return ChartLine;
  if (["pie", "rose", "funnel", "treemap"].includes(kind)) return ChartPie;
  if (["scatter", "bubble", "heatmap", "radar"].includes(kind)) return ScatterChart;
  if (kind === "geomap") return Map;
  if (kind === "pointmap") return MapPin;
  if (kind === "geoheat") return Flame;
  if (kind === "sankey") return Workflow;
  if (kind === "sunburst") return ChartPie;
  if (kind === "boxplot") return ChartCandlestick;
  if (kind === "calendar") return CalendarDays;
  if (kind === "kpi" || kind === "gauge") return Gauge;
  return kind === "table" ? Table2 : Type;
}
/** Measure labels that vary by kind (X/Y/size, bar/line, etc.). */
const MEASURE_LABELS: Partial<Record<ChartKind, string[]>> = {
  scatter: ["X metric", "Y metric"],
  bubble: ["X metric", "Y metric", "Size metric"],
  combo: ["Bar metric", "Line metric"],
  stacked: ["Measure", "2nd measure"],
  pointmap: ["Measure (size and colour)"],
  geoheat: ["Measure (weight)"],
};

/**
 * Chart builder — the MANUAL path (Tableau-style). Used in two modes:
 *  - CREATE (has its own "New chart" trigger),
 *  - EDIT (controlled by the parent: `open`, `initial` carries id+def).
 * Writes to the SAME artifact as the chat path (console.bi_chart).
 */
export function ChartBuilder({
  onSaved, board = "default", boards = [], initial, editId,
  open: openProp, onOpenChange, hideTrigger,
}: {
  /** Receives the definition that was saved. */
  onSaved: (saved: ChartDef) => void;
  board?: string;
  boards?: BoardOpt[];
  initial?: ChartDef;
  editId?: string;
  open?: boolean;
  onOpenChange?: (o: boolean) => void;
  hideTrigger?: boolean;
}) {
  const controlled = openProp !== undefined;
  const [openState, setOpenState] = React.useState(false);
  const open = controlled ? openProp! : openState;
  const setOpen = (o: boolean) => { onOpenChange?.(o); if (!controlled) setOpenState(o); };

  const [marts, setMarts] = React.useState<{ name: string; rows: number }[]>([]);
  const [sqlSources, setSqlSources] = React.useState<SqlSource[]>([]);
  const [fields, setFields] = React.useState<Fields | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [preview, setPreview] = React.useState<Preview | null>(null);
  const [previewBusy, setPreviewBusy] = React.useState(false);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const { resolvedTheme } = useTheme();
  // Side-by-side, resizable form and preview from the `lg` breakpoint.
  const wide = useMinWidth(1024);

  const [title, setTitle] = React.useState("");
  // Encoded data-source choice (`mart:<name>` / `sql:<id>`, lib/chart-source).
  const [source, setSource] = React.useState("");
  const choice = decodeSourceChoice(source);
  const [kind, setKind] = React.useState<ChartKind>("hbar");
  const [kindQuery, setKindQuery] = React.useState("");
  const kindGroups = React.useMemo(() => filterKindGroups(KIND_GROUPS, KIND_DESCRIPTIONS, kindQuery, KIND_KEYWORDS), [kindQuery]);
  const [dimension, setDimension] = React.useState("");
  const [measure, setMeasure] = React.useState("");
  const [measure2, setMeasure2] = React.useState("");
  const [measure3, setMeasure3] = React.useState("");
  const [breakdown, setBreakdown] = React.useState("");
  // Map kinds. `mapId` empty = "the default for this kind", so switching
  // kind before picking a map follows the kind (Jakarta cities for a
  // choropleth, Indonesian provinces behind points).
  const [mapId, setMapId] = React.useState("");
  const [lat, setLat] = React.useState("");
  const [lon, setLon] = React.useState("");
  const [aggregate, setAggregate] = React.useState("sum");
  const [span, setSpan] = React.useState<1 | 2>(1);
  const [caption, setCaption] = React.useState("");
  const [target, setTarget] = React.useState("");
  const [text, setText] = React.useState("");
  const [order, setOrder] = React.useState<"desc" | "asc" | "none">("desc");
  const [limit, setLimit] = React.useState(20);
  const [targetBoard, setTargetBoard] = React.useState(board);
  const isText = kind === "text";
  const isKpi = kind === "kpi";
  const isGauge = kind === "gauge";
  const isSingle = isKpi || isGauge;         // no dimension (single number)
  const isHeatmap = kind === "heatmap";
  // Kinds drawn from the breakdown shape (x, series, value): a 2nd
  // dimension is required (lakehouse_bi::store::breakdown_required).
  const needsBreakdown = isHeatmap || kind === "sankey" || kind === "sunburst";
  const isChoropleth = kind === "geomap";
  const isPoints = kind === "pointmap" || kind === "geoheat";
  const isMap = isChoropleth || isPoints;
  const effectiveMap = mapId || (isChoropleth ? DEFAULT_CHOROPLETH_MAP : DEFAULT_POINT_MAP);
  const isBoxplot = kind === "boxplot";
  const isCalendar = kind === "calendar";
  // A calendar is one cell per day, so up to a year of rows.
  const maxLimit = isCalendar ? 366 : 100;
  // Switching to a calendar starts from a full year; leaving it brings the
  // Top-N back inside 1–100.
  React.useEffect(() => {
    if (isCalendar) setLimit((l) => (l === 20 ? 366 : l));
    else setLimit((l) => Math.min(l, 100));
  }, [isCalendar]);
  // Latitude and longitude can be any column the source returns (numbers are
  // listed as measures, but a coordinate stored as text is still pickable).
  const coordinateColumns = React.useMemo(() => [...(fields?.measures ?? []), ...(fields?.dimensions ?? [])], [fields]);
  // A point map pre-fills its coordinate columns from their names, only
  // where the user has not chosen one (an edit keeps what was saved).
  React.useEffect(() => {
    if (!isPoints || !fields) return;
    const guessed = guessCoordinateColumns(coordinateColumns);
    setLat((l) => l || guessed.lat || "");
    setLon((l) => l || guessed.lon || "");
  }, [isPoints, fields, coordinateColumns]);
  const needsM2 = kind === "stacked" || kind === "scatter" || kind === "combo" || kind === "bubble";
  const needsM3 = kind === "bubble";
  const canBreakdown = kind === "bar" || kind === "hbar" || kind === "line" || kind === "area" || needsBreakdown;
  const mLabels = isSingle ? ["Measure"] : MEASURE_LABELS[kind] ?? ["Measure (Y)"];
  const dimensionLabel = isChoropleth ? "Region column" : isPoints ? "Label" : "Dimension (X)";
  const breakdownLabel = kind === "sankey" ? "Flows to" : kind === "sunburst" ? "Outer ring" : isHeatmap ? "2nd dimension (Y)" : "Breakdown / series";
  const suggestedTitle = suggestChartTitle({
    kind, dimension: isSingle ? undefined : dimension,
    measures: needsM3 ? [measure, measure2, measure3] : needsM2 ? [measure, measure2] : [measure],
    breakdown: canBreakdown ? breakdown : undefined,
  });
  const canWriteSql = useAuth().hasPermission("dashboard:sql");
  const isNewSql = source === NEW_SQL_CHOICE;
  // Fields and picks follow the columns of the latest successful Run, and a
  // column a re-run no longer returns is dropped rather than kept dangling.
  function applyRunColumns(columns: SqlSourceColumn[]) {
    const f = fieldsFromColumns(columns);
    const any = [...f.measures, ...f.dimensions];
    setFields(f);
    setDimension((v) => keepIfOffered(v, f.dimensions));
    setMeasure((v) => keepIfOffered(v, f.measures));
    setMeasure2((v) => keepIfOffered(v, f.measures));
    setMeasure3((v) => keepIfOffered(v, f.measures));
    setBreakdown((v) => keepIfOffered(v, f.dimensions));
    setLat((v) => keepIfOffered(v, any));
    setLon((v) => keepIfOffered(v, any));
  }
  const draft = useSqlSourceDraft({ chartTitle: title.trim() || suggestedTitle, editChartId: editId, onColumns: applyRunColumns });
  // A text note has no data source, so a SQL draft left behind is ignored.
  const sqlActive = !isText && draft.mode !== "none";
  const heldPreview = !isText && draft.holdsChartPreview;
  // The SQL a chart preview may be drawn over: only text that was run
  // successfully and not edited since, so the chart never reads columns the
  // SQL may no longer return.
  const previewSql = heldPreview && draft.phase === "fresh" ? draft.sql : null;
  const [paneTab, setPaneTab] = React.useState<"chart" | "rows">("chart");
  const noFieldsHint = isNewSql ? "run the SQL first" : "pick a source first";
  // Required fields still empty: listed in the preview before the user
  // hits Create, and the same list is the save-time error.
  const missing: string[] = isText
    ? [...(title.trim() ? [] : ["Title"]), ...(text.trim() ? [] : ["Content"])]
    : [
        ...(choice || isNewSql ? [] : ["Data source"]),
        ...(!isSingle && !isPoints && !dimension ? [dimensionLabel] : []),
        ...(isPoints && !lat ? ["Latitude"] : []),
        ...(isPoints && !lon ? ["Longitude"] : []),
        ...(measure ? [] : [mLabels[0]]),
        ...(needsM2 && !measure2 ? [mLabels[1] ?? "2nd measure"] : []),
        ...(needsM3 && !measure3 ? [mLabels[2] ?? "3rd measure"] : []),
        ...(needsBreakdown && !breakdown ? [breakdownLabel] : []),
      ];
  const isEdit = !!editId;
  // Why no chart can be drawn yet while the SQL is new or edited (null = it
  // can). The rows of the Run fill the pane meanwhile.
  const chartHold: string | null = !heldPreview ? null
    : draft.phase === "stale" ? "The SQL changed since it was run. Run it again to preview the chart; until then, these are the rows of the earlier run."
    : draft.phase === "failed" ? "The SQL did not run, so there is no chart to preview."
    : draft.phase === "running" ? "Running the SQL…"
    : draft.phase !== "fresh" ? "Write the SQL and run it to preview a chart over it."
    : missing.length ? `Fill in ${missing.join(", ")} to preview the chart. Until then, these are the rows your SQL returns.`
    : null;
  const showRows = heldPreview && (chartHold !== null || paneTab === "rows");
  const boardOptions = boards.length ? boards : [{ id: "default", name: "Main" }];
  const boardLabels: Record<string, string> = Object.fromEntries(boardOptions.map((b) => [b.id, b.name]));
  // The data-source value is encoded (lib/chart-source); label it by name.
  const sourceLabels: Record<string, string> = Object.fromEntries([
    ...marts.map((m) => [encodeSourceChoice({ kind: "mart", name: m.name }), m.name]),
    ...sqlSources.map((s) => [encodeSourceChoice({ kind: "sql", id: s.id }), `SQL · ${s.title}`]),
    [NEW_SQL_CHOICE, "Custom SQL · new source"],
  ]);
  const breakdownLabels: Record<string, string> = { [NO_BREAKDOWN]: "— no breakdown —" };

  async function loadFields(value: string): Promise<Fields> {
    const picked = decodeSourceChoice(value);
    if (!picked) { setFields(null); return { dimensions: [], measures: [] }; }
    const j = await apiFetch(`/api/dashboard/fields?${fieldsQuery(picked)}`).then((r) => r.json());
    const f = { dimensions: j.dimensions ?? [], measures: j.measures ?? [] };
    setFields(f);
    return f;
  }

  // On open: load the marts, and prefill from initial — EDIT, or a draft
  // (e.g. from Copilot) whose kind is already chosen.
  React.useEffect(() => {
    if (!open) return;
    draft.cancel();
    void apiFetch("/api/dashboard/fields").then((r) => r.json()).then((j) => setMarts(j.marts ?? [])).catch(() => setMarts([]));
    // A viewer without the list (or with the API down) still gets marts.
    void dashboardService.listSqlSources().then(setSqlSources).catch(() => setSqlSources([]));
    if (initial) {
      setTitle(initial.title ?? "");
      setKind((initial.kind as ChartKind) ?? "hbar");
      setAggregate(initial.aggregate ?? "sum");
      setSpan(initial.span === 2 ? 2 : 1);
      setBreakdown(initial.breakdown ?? "");
      setMapId(initial.map ?? "");
      setLat(initial.lat ?? "");
      setLon(initial.lon ?? "");
      setCaption(initial.caption ?? "");
      setTarget(initial.target != null ? String(initial.target) : "");
      setText(initial.text ?? "");
      setOrder((initial.order as "desc" | "asc" | "none") ?? "desc");
      setLimit(initial.limit ?? 20);
      setTargetBoard(initial.board ?? board);
      const m = sourceValueFromDef(initial);
      setSource(m);
      if (m) {
        void loadFields(m).then(() => {
          setDimension(initial.dimension ?? "");
          setMeasure(initial.measures?.[0] ?? "");
          setMeasure2(initial.measures?.[1] ?? "");
          setMeasure3(initial.measures?.[2] ?? "");
        });
      }
    } else {
      setTargetBoard(board);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function reset() {
    setTitle(""); setSource(""); setKind("hbar"); setKindQuery(""); setDimension("");
    setMeasure(""); setMeasure2(""); setMeasure3(""); setBreakdown(""); setMapId(""); setLat(""); setLon("");
    setAggregate("sum"); setSpan(1);
    setCaption(""); setTarget(""); setText(""); setOrder("desc"); setLimit(20);
    setTargetBoard(board); setFields(null); setError(null); setPreview(null); setPreviewError(null);
    draft.cancel();
  }

  // User changes the data source → reset the column selections and reload.
  function onSourceChange(value: string) {
    draft.cancel();
    setSource(value); setDimension(""); setMeasure(""); setMeasure2(""); setMeasure3(""); setBreakdown("");
    setLat(""); setLon("");
    if (value === NEW_SQL_CHOICE) { setFields(null); draft.startNew(); return; }
    if (value) void loadFields(value); else setFields(null);
  }

  /** Back to the saved source: the pickers list its stored columns again. */
  function cancelSqlEdit() {
    draft.cancel();
    if (!source) return;
    void loadFields(source).then((f) => {
      setDimension((v) => keepIfOffered(v, f.dimensions));
      setMeasure((v) => keepIfOffered(v, f.measures));
      setMeasure2((v) => keepIfOffered(v, f.measures));
      setMeasure3((v) => keepIfOffered(v, f.measures));
      setBreakdown((v) => keepIfOffered(v, f.dimensions));
    });
  }

  function buildPayload(forPreview = false, withSource: ChartSourceChoice | null = choice): Record<string, unknown> {
    if (missing.length) throw new Error(`Still needed: ${missing.join(", ")}.`);
    const payloadTitle = title.trim() || suggestedTitle || (forPreview ? "Chart preview" : "");
    if (!payloadTitle) throw new Error("Title is required.");
    let payload: Record<string, unknown>;
    if (isText) {
      payload = { title: payloadTitle, kind, text, span, board: targetBoard };
    } else if (isSingle) {
      payload = {
        title: payloadTitle, kind, ...sourcePayload(withSource), measures: [measure], aggregate, span, board: targetBoard,
        caption: isKpi && caption ? caption : undefined,
        target: isGauge && Number(target) > 0 ? Number(target) : undefined,
      };
    } else {
      const measures = (needsM3 ? [measure, measure2, measure3] : needsM2 ? [measure, measure2] : [measure]).filter(Boolean);
      payload = {
        title: payloadTitle, ...sourcePayload(withSource), kind, dimension, measures, aggregate, span, board: targetBoard,
        breakdown: canBreakdown && breakdown ? breakdown : undefined,
        map: isMap ? effectiveMap : undefined,
        lat: isPoints ? lat : undefined, lon: isPoints ? lon : undefined,
        // A point map has a fixed cap of its own (lakehouse_bi::builder::POINT_LIMIT) and is not sorted.
        order: isCalendar ? "none" : isPoints ? undefined : order, limit: isPoints ? undefined : Math.min(limit, maxLimit),
      };
    }
    if (isEdit) payload.id = editId;
    return payload;
  }

  React.useEffect(() => {
    if (!open) return;
    if (isText) {
      setPreview(text.trim() ? {
        spec: { id: "preview", title: title || "Chart preview", kind: "text", mart: "", x: "", y: "", source: "ui", text },
        result: { columns: [], rows: [] },
      } : null);
      setPreviewError(null);
      return;
    }
    // `/specs/preview` reads the SAVED source by id, so while the SQL is new
    // or edited it would draw the old rows (or none). The chart is drawn by
    // the SQL-source preview instead, over the SQL as written, once that SQL
    // has been run unchanged; before then the pane shows the Run's rows.
    if (heldPreview && previewSql === null) { setPreview(null); setPreviewError(null); setPreviewBusy(false); return; }
    let payload: Record<string, unknown>;
    // No source in the chart input: the SQL travels beside it.
    try { payload = buildPayload(true, heldPreview ? null : choice); } catch { setPreview(null); setPreviewError(null); return; }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      setPreviewBusy(true); setPreviewError(null);
      const request: Promise<Preview> = previewSql !== null
        ? dashboardService.previewSqlSource(previewSql, payload, controller.signal).then((res) => {
            if (!res.chart) throw new Error("Preview failed");
            return res.chart as unknown as Preview;
          })
        : apiFetch("/api/dashboard/specs/preview", {
            method: "POST", headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload), signal: controller.signal,
          }).then(async (res) => {
            const json = await res.json();
            if (!res.ok) throw new Error(json?.error ?? "Preview failed");
            return json as Preview;
          });
      void request.then((next) => {
        if (!controller.signal.aborted) setPreview(next);
      }).catch((e: unknown) => {
        if (!controller.signal.aborted) { setPreview(null); setPreviewError(e instanceof Error ? e.message : String(e)); }
      }).finally(() => { if (!controller.signal.aborted) setPreviewBusy(false); });
    }, 450);
    return () => { window.clearTimeout(timer); controller.abort(); };
    // Every field below affects the generated SQL or render spec.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, title, source, kind, dimension, measure, measure2, measure3, breakdown, mapId, lat, lon, aggregate, span, caption, target, text, order, limit, targetBoard, heldPreview, previewSql]);

  async function save() {
    setError(null);
    if (sqlActive && draft.blocker) { draft.markSaveAttempted(); setError(draft.blocker); return; }
    let payload: Record<string, unknown>;
    // A new source has no id yet; a placeholder lets the same checks run
    // before anything is written, and the real id replaces it below.
    try { payload = buildPayload(false, draft.mode === "new" && sqlActive ? { kind: "sql", id: "pending" } : choice); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); return; }
    setBusy(true);
    let savedSource: SqlSource | null = null;
    let sourceVerb = "created";
    try {
      if (sqlActive) {
        const name = draft.name.trim();
        const original = draft.original;
        try {
          if (draft.mode === "new") {
            savedSource = await dashboardService.createSqlSource({ title: name, sql: draft.sql });
          } else if (original && (draft.changed || name !== original.title)) {
            // `folderId` is sent back: the API replaces the folder with what
            // the body says, and an absent one would move the source to the root.
            sourceVerb = "updated";
            savedSource = await dashboardService.updateSqlSource({ id: original.id, title: name, sql: draft.sql, folderId: original.folderId });
          }
        } catch (e) {
          // The API's own message (guard, probe or name error), shown as it is.
          setError(e instanceof Error ? e.message : String(e));
          return;
        }
        if (savedSource) {
          const saved = savedSource;
          setSqlSources((prev) => prev.some((s) => s.id === saved.id) ? prev.map((s) => (s.id === saved.id ? saved : s)) : [...prev, saved]);
          payload = buildPayload(false, { kind: "sql", id: saved.id });
        }
      }
      try {
        const res = await apiFetch("/api/dashboard/specs", {
          method: isEdit ? "PUT" : "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        });
        const json = await res.json();
        if (!res.ok) throw new Error(json?.error ?? "Failed to save chart");
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        if (!savedSource) { setError(message); return; }
        // The source exists now. Move the builder onto it so that retrying
        // saves only the chart instead of creating a second source.
        const value = encodeSourceChoice({ kind: "sql", id: savedSource.id });
        draft.cancel();
        setSource(value);
        void loadFields(value);
        setError(`The SQL source “${savedSource.title}” was ${sourceVerb}, but the chart was not saved: ${message}. The builder now uses that source, so saving again will not create it twice.`);
        return;
      }
      setOpen(false);
      if (!isEdit) reset();
      onSaved(payload as ChartDef);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const formFields = (
    <div className="grid content-start gap-4 px-5 pt-1 pb-5">
      <div className="grid gap-1.5">
        <Label htmlFor="ch-title">Title {isText ? <Req /> : null}</Label>
        <Input
          id="ch-title" value={title} onChange={(e) => setTitle(e.target.value)}
          placeholder={suggestedTitle || "e.g. Visitors by Region"}
        />
        {!title.trim() && suggestedTitle ? (
          <p className="text-xs text-muted-foreground">Left empty, the chart is saved as “{suggestedTitle}”.</p>
        ) : null}
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div className="grid gap-1.5">
          <Label>Dashboard</Label>
          <Select value={targetBoard} items={boardLabels} onValueChange={(v) => setTargetBoard(v ?? "default")}>
            <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
            <SelectContent>
              {boardOptions.map((b) => (
                <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid gap-1.5">
          <Label>Width</Label>
          <Select value={String(span)} items={SPAN_LABELS} onValueChange={(v) => setSpan(v === "2" ? 2 : 1)}>
            <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="1">{SPAN_LABELS["1"]}</SelectItem>
              <SelectItem value="2">{SPAN_LABELS["2"]}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {isText ? (
        <div className="grid gap-1.5">
          <Label>Content (markdown) <Req /></Label>
          <Textarea rows={8} value={text} onChange={(e) => setText(e.target.value)} placeholder="Title **bold**, - bullets, or | table | GFM |." />
        </div>
      ) : (
        <>
          <div className="grid gap-1.5">
            <Label>Data source <Req /></Label>
            <Select value={source} items={sourceLabels} onValueChange={(v) => onSourceChange(v ?? "")}>
              <SelectTrigger className="w-full"><SelectValue placeholder={canWriteSql ? "pick a mart or SQL source, or write SQL" : "pick a mart or SQL source"} /></SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>Gold marts</SelectLabel>
                  {marts.map((m) => (
                    <SelectItem key={m.name} value={encodeSourceChoice({ kind: "mart", name: m.name })}>
                      {m.name} · {m.rows.toLocaleString("id-ID")}
                    </SelectItem>
                  ))}
                </SelectGroup>
                {sqlSources.length ? (
                  <SelectGroup>
                    <SelectLabel>SQL sources</SelectLabel>
                    {sqlSources.map((s) => (
                      <SelectItem key={s.id} value={encodeSourceChoice({ kind: "sql", id: s.id })}>
                        {s.title}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                ) : null}
                {canWriteSql ? (
                  <SelectGroup>
                    <SelectLabel>New</SelectLabel>
                    <SelectItem value={NEW_SQL_CHOICE}>
                      <Code className="size-4" aria-hidden /> Write custom SQL…
                    </SelectItem>
                  </SelectGroup>
                ) : null}
              </SelectContent>
            </Select>
            {choice?.kind === "sql" && draft.mode !== "edit" ? (
              canWriteSql ? (
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                  <p className="text-xs text-muted-foreground">
                    Custom SQL source. Drill-down to records is not available for SQL sources yet.
                  </p>
                  <Button
                    type="button" size="sm" variant="outline"
                    disabled={!sqlSources.some((s) => s.id === choice.id)}
                    onClick={() => { const src = sqlSources.find((s) => s.id === choice.id); if (src) draft.startEdit(src); }}
                  >
                    <Pencil className="size-3.5" aria-hidden /> Edit SQL
                  </Button>
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">
                  Custom SQL source. You need the dashboard:sql permission to edit it. Drill-down to records is not available for SQL sources yet.
                </p>
              )
            ) : null}
            {sqlActive ? <SqlSourcePanel draft={draft} onCancelEdit={cancelSqlEdit} /> : null}
          </div>

          {isMap ? (
            <div className="grid gap-1.5">
              <Label>Map</Label>
              <Select value={effectiveMap} items={MAP_LABELS} onValueChange={(v) => setMapId(v ?? "")}>
                <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>{MAP_CATALOGUE.map((m) => <SelectItem key={m.id} value={m.id}>{m.label}</SelectItem>)}</SelectContent>
              </Select>
              {mapCredit(effectiveMap) ? <p className="text-xs text-muted-foreground">{mapCredit(effectiveMap)}</p> : null}
              {isChoropleth ? (
                <p className="text-xs text-muted-foreground">The region column is matched to the map by name; rows that match no region are counted under the map.</p>
              ) : null}
            </div>
          ) : null}

          {/* Dimensions together, then the measure next to how it is aggregated. */}
          {!isSingle ? (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label>{dimensionLabel} {isPoints ? <span className="font-normal text-muted-foreground">(optional, shown in the tooltip)</span> : <Req />}</Label>
                {isPoints ? (
                  <Select value={dimension || NO_BREAKDOWN} items={NO_LABEL_ITEMS} onValueChange={(v) => setDimension(v === NO_BREAKDOWN ? "" : v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "no label" : noFieldsHint} /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NO_BREAKDOWN}>— no label —</SelectItem>
                      {fields?.dimensions.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                    </SelectContent>
                  </Select>
                ) : (
                  <Select value={dimension} onValueChange={(v) => setDimension(v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                    <SelectContent>{fields?.dimensions.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}</SelectContent>
                  </Select>
                )}
              </div>
              {canBreakdown ? (
                <div className="grid gap-1.5">
                  <Label>{breakdownLabel} {needsBreakdown ? <Req /> : <span className="font-normal text-muted-foreground">(optional)</span>}</Label>
                  <Select value={breakdown || NO_BREAKDOWN} items={breakdownLabels} onValueChange={(v) => setBreakdown(v === NO_BREAKDOWN ? "" : v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full"><SelectValue placeholder={needsBreakdown ? "pick a 2nd column" : "no breakdown"} /></SelectTrigger>
                    <SelectContent>
                      {needsBreakdown ? null : <SelectItem value={NO_BREAKDOWN}>— no breakdown —</SelectItem>}
                      {fields?.dimensions.filter((d) => d !== dimension).map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
              ) : null}
            </div>
          ) : null}
          {isPoints ? (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label>Latitude <Req /></Label>
                <Select value={lat} onValueChange={(v) => setLat(v ?? "")} disabled={!fields}>
                  <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                  <SelectContent>{coordinateColumns.filter((c) => c !== lon).map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5">
                <Label>Longitude <Req /></Label>
                <Select value={lon} onValueChange={(v) => setLon(v ?? "")} disabled={!fields}>
                  <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                  <SelectContent>{coordinateColumns.filter((c) => c !== lat).map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
                </Select>
              </div>
            </div>
          ) : null}
          {isCalendar ? (
            <p className="-mt-2 text-xs text-muted-foreground">Pick a date column as the dimension; one cell per day, up to the last year of data.</p>
          ) : null}

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label>{mLabels[0]} <Req /></Label>
              <Select value={measure} onValueChange={(v) => setMeasure(v ?? "")} disabled={!fields}>
                <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                <SelectContent>{fields?.measures.filter((m) => !isPoints || (m !== lat && m !== lon)).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            {isBoxplot ? (
              <div className="grid gap-1.5">
                <Label>Aggregation</Label>
                <p className="flex h-8 items-center text-xs text-muted-foreground">Distribution: min, quartiles, max</p>
              </div>
            ) : (
              <div className="grid gap-1.5">
                <Label>Aggregation</Label>
                <Select value={aggregate} onValueChange={(v) => setAggregate(v ?? "")}>
                  <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                </Select>
              </div>
            )}
          </div>

          {needsM2 ? (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label>{mLabels[1] ?? "2nd measure"} <Req /></Label>
                <Select value={measure2} onValueChange={(v) => setMeasure2(v ?? "")} disabled={!fields}>
                  <SelectTrigger className="w-full"><SelectValue placeholder="pick a column" /></SelectTrigger>
                  <SelectContent>{fields?.measures.filter((m) => m !== measure).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                </Select>
              </div>
              {needsM3 ? (
                <div className="grid gap-1.5">
                  <Label>{mLabels[2] ?? "3rd measure"} <Req /></Label>
                  <Select value={measure3} onValueChange={(v) => setMeasure3(v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full"><SelectValue placeholder="pick a column" /></SelectTrigger>
                    <SelectContent>{fields?.measures.filter((m) => m !== measure && m !== measure2).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
              ) : null}
            </div>
          ) : null}

          {isKpi ? (
            <div className="grid gap-1.5">
              <Label>Caption <span className="font-normal text-muted-foreground">(optional)</span></Label>
              <Input value={caption} onChange={(e) => setCaption(e.target.value)} placeholder="e.g. foreign visits (cumulative)" />
            </div>
          ) : isGauge ? (
            <div className="grid gap-1.5">
              <Label>Target / max <span className="font-normal text-muted-foreground">(optional)</span></Label>
              <Input type="number" min={0} value={target} onChange={(e) => setTarget(e.target.value)} placeholder="auto from value if empty" />
            </div>
          ) : isPoints ? (
            <p className="text-xs text-muted-foreground">
              Up to {pointLimit(choice?.kind === "sql" ? choice.id : undefined).toLocaleString("en-US")} points, largest values first.
              One point per distinct latitude and longitude{dimension ? " and label" : ""}.
            </p>
          ) : (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              {isBoxplot || isCalendar ? (
                <div className="grid gap-1.5">
                  <Label>Sort</Label>
                  <p className="flex h-8 items-center text-xs text-muted-foreground">{isCalendar ? "By date" : "By category"}</p>
                </div>
              ) : (
                <div className="grid gap-1.5">
                  <Label>Sort</Label>
                  <Select value={order} items={ORDER_LABELS} onValueChange={(v) => setOrder((v as "desc" | "asc" | "none") ?? "desc")}>
                    <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="desc">Highest first</SelectItem>
                      <SelectItem value="asc">Lowest first</SelectItem>
                      <SelectItem value="none">Natural (by dimension)</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              )}
              <div className="grid gap-1.5">
                <Label>{isCalendar ? "Days (max 366)" : "Limit (Top N)"}</Label>
                <Input type="number" min={1} max={maxLimit} value={limit} onChange={(e) => setLimit(Math.max(1, Math.min(maxLimit, Number(e.target.value) || 20)))} />
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
  const previewPane = (
    <section aria-label="Preview" className={cn("flex flex-col gap-3 px-5 pb-5", wide ? "h-full pt-1" : "min-h-96 border-t border-border pt-4")}>
      <div className="flex flex-wrap items-center justify-between gap-x-3">
        <p className="text-sm font-medium">Preview</p>
        {heldPreview && chartHold === null && draft.result ? (
          <div role="group" aria-label="Preview content" className="flex gap-1">
            {(["chart", "rows"] as const).map((tab) => (
              <Button
                key={tab} type="button" size="xs" variant={paneTab === tab ? "secondary" : "ghost"}
                aria-pressed={paneTab === tab} onClick={() => setPaneTab(tab)}
              >
                {tab === "chart" ? "Chart" : "Rows"}
              </Button>
            ))}
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">{showRows ? "Rows from your SQL" : "Live data, updates as you edit"}</p>
        )}
      </div>
      <div className="relative min-h-72 flex-1">
        <div className="absolute inset-0">
          {showRows ? (
            <div className="flex h-full flex-col gap-3 overflow-auto">
              {chartHold ? <p className="text-sm text-muted-foreground">{chartHold}</p> : null}
              {draft.result && draft.phase !== "empty" ? (
                <SqlRowsTable result={draft.result} stale={draft.phase === "stale"} className="min-h-0 flex-1" />
              ) : (
                <EmptyState className="py-6" title="Run the SQL" description="Its first rows appear here." />
              )}
            </div>
          ) : preview ? (
            <TileBody spec={preview.spec} cell={preview.result} dark={resolvedTheme === "dark"} loading={previewBusy} />
          ) : previewBusy ? (
            <div className="h-full animate-pulse rounded-md bg-muted/50" />
          ) : (
            <EmptyState
              className="h-full py-6"
              title={previewError ? "Preview unavailable" : missing.length ? "Nothing to preview yet" : "Loading preview…"}
              description={previewError ?? (missing.length ? `Fill in ${missing.join(", ")} to see real data here.` : undefined)}
            />
          )}
        </div>
      </div>
    </section>
  );

  return (
    <Dialog open={open} onOpenChange={(o) => { setOpen(o); if (!o && !isEdit) reset(); }}>
      {hideTrigger ? null : (
        <DialogTrigger render={<Button variant="outline" size="sm" />}>
          <Plus className="size-4" /> New chart
        </DialogTrigger>
      )}
      <DialogContent className="h-[min(90vh,52rem)] overflow-hidden p-0 sm:max-w-6xl">
        {/*
          Chart types live in a sidebar next to the form (like a settings
          dialog) instead of a separate gallery step: switching type keeps
          everything else filled in and the live preview follows at once.
          The dialog has a fixed height so the actions stay at the bottom
          edge whatever the form's length.
        */}
        <div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)] md:grid-cols-[14rem_minmax(0,1fr)] md:grid-rows-1">
          <nav aria-label="Chart type" className="flex min-h-0 flex-col border-b border-border bg-muted/30 md:border-r md:border-b-0">
            <div className="relative p-2 md:p-3 md:pb-2">
              <Search className="pointer-events-none absolute top-1/2 left-4 size-4 -translate-y-1/2 text-muted-foreground md:left-5" aria-hidden />
              <Input
                value={kindQuery}
                onChange={(e) => setKindQuery(e.target.value)}
                onKeyDown={(e) => {
                  // Enter picks the first match, so "sank⏎" is enough.
                  const first = kindGroups[0]?.items[0];
                  if (e.key === "Enter" && first) { e.preventDefault(); setKind(first.value); }
                }}
                placeholder="Search chart types"
                aria-label="Search chart types"
                className="h-8 bg-background pl-8"
              />
            </div>
            <div className="flex min-h-0 gap-1 overflow-x-auto px-2 pb-2 md:flex-col md:gap-3 md:overflow-y-auto md:px-3 md:pb-3">
            {kindGroups.length === 0 ? (
              <p className="px-2 py-1.5 text-xs text-muted-foreground">No chart type matches “{kindQuery.trim()}”.</p>
            ) : null}
            {kindGroups.map((group) => (
              <div key={group.group} className="flex shrink-0 gap-1 md:flex-col">
                <p className="hidden px-2 pb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground md:block">
                  {group.group}
                </p>
                {group.items.map((item) => {
                  const Icon = kindIcon(item.value);
                  const active = item.value === kind;
                  return (
                    <button
                      key={item.value}
                      type="button"
                      aria-pressed={active}
                      title={KIND_DESCRIPTIONS[item.value]}
                      onClick={() => setKind(item.value)}
                      className={cn(
                        "flex shrink-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm whitespace-nowrap transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                        active ? "bg-primary/10 font-medium text-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground",
                      )}
                    >
                      <Icon className={cn("size-4 shrink-0", active ? "text-primary" : "opacity-70")} />
                      <span className="md:truncate">{item.label}</span>
                    </button>
                  );
                })}
              </div>
            ))}
            </div>
          </nav>

          <div className="flex min-h-0 flex-col">
            <div className="px-5 pt-5 pb-4">
              <DialogHeader>
                <DialogTitle className="flex items-center gap-1.5 text-base">
                  <span className="text-muted-foreground">{isEdit ? "Edit chart" : "New chart"}</span>
                  <ChevronRight className="size-4 text-muted-foreground" aria-hidden />
                  <span>{KIND_LABELS[kind]}</span>
                </DialogTitle>
                <DialogDescription>{KIND_DESCRIPTIONS[kind]}. Review live data before adding it to the dashboard.</DialogDescription>
              </DialogHeader>
            </div>
            {/*
              Form and preview are split by a divider rather than a card
              around the preview, so the chart gets the whole column height.
              On a wide screen the divider is a drag handle (double-click
              resets it); a narrow one stacks them and scrolls instead.
            */}
            {wide ? (
              <ResizablePanelGroup className="min-h-0 flex-1">
                <ResizablePanel defaultSize="50" minSize="30">
                  <div className="h-full overflow-y-auto">{formFields}</div>
                </ResizablePanel>
                <ResizableHandle withHandle />
                <ResizablePanel defaultSize="50" minSize="30">{previewPane}</ResizablePanel>
              </ResizablePanelGroup>
            ) : (
              <div className="min-h-0 flex-1 overflow-y-auto">
                {formFields}
                {previewPane}
              </div>
            )}

            {/* Error sits beside the button that caused it, not at the end of a long form. */}
            <DialogFooter className="items-center border-t border-border px-5 py-3">
              {error ? <p role="alert" className="text-sm text-destructive sm:mr-auto">{error}</p> : null}
              {!error && sqlActive && draft.blocker ? <p className="text-sm text-muted-foreground sm:mr-auto">{draft.blocker}</p> : null}
              <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
              <Button size="sm" onClick={() => void save()} disabled={busy || (sqlActive && !!draft.runBlocker)}>
                {busy ? "Saving…" : isEdit ? "Save changes" : "Create chart"}
              </Button>
            </DialogFooter>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
