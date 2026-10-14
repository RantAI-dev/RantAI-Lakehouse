"use client";

import * as React from "react";
import { useTheme } from "next-themes";
import {
  CalendarDays, ChartBar, ChartCandlestick, ChartColumn, ChartLine, ChartPie, ChevronDown, ChevronRight, Code, Flame, Gauge, Workflow,
  Map, MapPin, PanelLeftClose, PanelLeftOpen, Pencil, Plus, ScatterChart, Search, SlidersHorizontal, Table2, Type, type LucideIcon,
} from "lucide-react";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
  DialogFooter, DialogTrigger, DialogClose,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/patterns/page-states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";
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
import { dashboardService, metricService, paramService } from "@/services";
import type { Metric, Segment } from "@/services/contracts/metrics";
import {
  columnSwitchParams, limitParam, numberParams, refLinesFromDef, refLinesPayload, slotParam, slotText,
  type ParamDef, type ParamValue, type RefLineDef, type RefLineDraft,
} from "@/lib/params";
import { ParamDialog } from "./param-dialog";
import { ReferenceLinesEditor } from "./reference-lines-editor";
import type { ChartClick } from "@/services/clients/bi-store";
import { kindHasClickValue } from "@/lib/chart-click";
import { clickProblem } from "@/lib/click-destination";
import { ClickSetting } from "./click-setting";
import { KpiCompareSection, PivotSection, TableSection, type SourceColumns } from "./table-settings";
import { draftFromDef, draftPayload, draftProblems, emptyTableDraft, type TableDraft } from "@/lib/table-draft";
import type { TableDefFields } from "@/lib/table-types";
import type { SqlSource, SqlSourceColumn } from "@/services/contracts/dashboards";
import { apiFetch } from "@/services/http";
import { filterKindGroups } from "@/lib/chart-kind-search";
import { previewKey } from "@/lib/preview-key";
import {
  effectiveOrder, emptyShapeDraft, entriesFromRows, columnRef, isMetricColumn, metricColumn, metricNameOf, measureLabel, measureLimits, parseSortChoice, resultName,
  rowsFromEntries, shapeDraftFromDef, takesBin, shapePayload, shapeProblems, sortOptions, stepsApply, usesGroupings, columnsAfterSteps,
  type MeasureRow, type ResultColumns, type ShapeDefFields, type ShapeDraft,
} from "@/lib/chart-shape";
import { AUTO_BIN, binDraftFromDef, binPayload } from "@/lib/chart-bins";
import { BinPicker, GridPicker } from "./bin-picker";
import { ChartFilters, GroupingsEditor, MeasureRows, StepsEditor, PreviewRowsTable, type ColumnChoice } from "./chart-shape-section";
import { withPreviewDef } from "@/lib/preview-spec";
import { suggestChartTitle } from "@/lib/chart-title";
import { guessCoordinateColumns, pointLimit } from "@/lib/geo-points";
import {
  GRAIN_LABELS, columnKindOfType, grainChoices, isGrain, isTruncation, limitAfterGrainPick, limitHasEffect, type ColumnKind, type Grain,
} from "@/lib/time-grain";
import { cn } from "@/lib/utils";
import { DEFAULT_CHOROPLETH_MAP, DEFAULT_POINT_MAP, MAP_CATALOGUE, mapCredit } from "./echarts-maps";
import { SqlRowsTable, SqlSourcePanel } from "./sql-source-panel";
import { JoinsSection, type JoinTableInfo } from "./joins-section";
import { LayersSection } from "./layers-section";
import { layerDraftsFromDef, layerProblems, layersPayload, takesLayers, type LayerDraft } from "@/lib/chart-layers";
import { displayColumnFull, joinsPayload, pairsFromSuggestions, type JoinDraft, type JoinPairDef } from "@/lib/chart-joins";
import { AccordionSection, ColumnLabel, FoldSection, columnItems, focusFirstEmpty } from "./builder-sections";
import { missingSection, sectionSummaries, type SectionNumber } from "@/lib/section-summary";
import { readKindListPref, writeKindListPref } from "@/lib/kind-list-pref";
import { CustomFieldsMenu } from "./custom-fields-menu";
import { builderRules, carryOverOnKindSwitch, moreOptionsUsed } from "@/lib/builder-rules";
import { FormulaEditorDialog } from "./formula-editor";
import { isValueGroup, type CalcField } from "@/services/contracts/calc-fields";
import type { Hierarchy } from "@/services/contracts/hierarchies";
import { ValueGroupDialog } from "./value-group-dialog";
import { HierarchyDialog } from "./hierarchy-dialog";
import { TileBody } from "./tile-body";
import { useSqlSourceDraft } from "./use-sql-source-draft";
import type { TileFailure } from "@/services/contracts/dashboards";

/** `kinds`: each column's kind, from its type, so the grain picker knows which dimensions are dates (BI-9). */
type Fields = {
  dimensions: string[]; measures: string[]; kinds?: Record<string, ColumnKind>;
  /** The source's own columns, apart from the calculated fields merged into `dimensions` and `measures` (BI-8). */
  columns?: string[];
  calculated?: CalcField[];
  /** The source's hierarchies (BI-10). */
  hierarchies?: Hierarchy[];
  /** The metrics saved on this source (BI-2): picked as a measure, by name. */
  metrics?: Metric[];
  /** The segments saved on this source (BI-2): added to the chart's filters by name. */
  segments?: Segment[];
};
export type ChartDef = {
  title?: string; subtitle?: string; mart?: string; sqlSource?: string; kind?: ChartKind;
  dimension?: string; breakdown?: string;
  map?: string; lat?: string; lon?: string;
  aggregate?: string; span?: 1 | 2; board?: string; text?: string; caption?: string;
  order?: "desc" | "asc" | "none"; limit?: number | { param: string }; target?: number;
  /** Lines on the value axis, each a number or a number parameter (BI-11). */
  referenceLines?: RefLineDef[];
  /** What a click does instead of the drill menu (BI-18·B); absent = the menu. */
  click?: ChartClick;
  /** How a date or timestamp dimension is grouped (BI-9); absent = as stored. */
  grain?: string;
} & TableDefFields // raw tables, pivots and KPI comparisons (BI-16 part A)
  & ShapeDefFields; // measures with their own aggregate, the chart's own filters, sort, "Other", groupings (BI-6)
type BoardOpt = { id: string; name: string };
/** The rows of the chart's last result beside the chart (BI-6 part 2). */
type PreviewRows = { columns: string[]; rows: Record<string, unknown>[]; truncated: boolean; limit: number };
type Preview = {
  previewRows?: PreviewRows | null;
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
    { value: "pivot", label: "Pivot table" },
    { value: "text", label: "Text / note" },
  ] },
];
/**
 * Value → label maps for the base-ui selects below. Without `items`, a
 * base-ui `Select.Value` renders the raw value (`hbar`, `b_54e05896`,
 * `desc`, `__none__`) in the closed trigger even though the open list shows
 * labels.
 */
const KIND_LABELS: Record<string, string> = Object.fromEntries(
  KIND_GROUPS.flatMap((g) => g.items.map((i) => [i.value, i.label]))
);
const NO_BREAKDOWN = "__none__";
const FIXED_LIMIT = "__fixed_limit__";
const NO_GRAIN = "__no_grain__";
const GRAIN_ITEMS: Record<string, string> = { [NO_GRAIN]: "No grouping", ...GRAIN_LABELS };
/** Column name → kind, from the `{ name, type }` list a fields response or a SQL run returns. */
function kindsOf(columns: readonly { name: string; type: string }[]): Record<string, ColumnKind> {
  return Object.fromEntries(columns.map((c) => [c.name, columnKindOfType(c.type)]));
}
/**
 * The source's columns plus its calculated fields (BI-8). A per-row number,
 * an aggregate and a table calculation are offered as measures, any other
 * per-row field as a dimension; the server refuses the last two anywhere but
 * a measure, and says why (the preview shows its sentence).
 */
function withCalculated(base: Fields, columns: string[], calculated: CalcField[]): Fields {
  const kinds = { ...base.kinds };
  const dimensions = [...base.dimensions];
  const measures = [...base.measures];
  for (const f of calculated) {
    kinds[f.name] = f.type === "number" ? "number" : f.type === "date" ? "date" : f.type === "datetime" ? "datetime" : "text";
    (f.level === "aggregate" || f.level === "table" || f.type === "number" ? measures : dimensions).push(f.name);
  }
  return { dimensions, measures, kinds, columns, calculated };
}
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
  table: "Inspect detailed rows and values", pivot: "Cross-tabulate with totals and subtotals", text: "Add context, notes, or instructions",
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
  pivot: "pivot cross tab crosstab matrix totals subtotals",
  table: "table rows raw columns grid",
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
  return kind === "table" || kind === "pivot" ? Table2 : Type;
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
  onSaved, board = "default", boards = [], initial, editId, paramValues,
  open: openProp, onOpenChange, hideTrigger,
}: {
  /** Receives the definition that was saved. */
  onSaved: (saved: ChartDef) => void;
  board?: string;
  boards?: BoardOpt[];
  initial?: ChartDef;
  editId?: string;
  /** The parameter values the dashboard shows now; the preview is drawn with them (BI-11). */
  paramValues?: Record<string, ParamValue>;
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
  // BI-8: the formula box. `editing` is the field being changed, absent for a new one.
  const [formulaOpen, setFormulaOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<CalcField | undefined>(undefined);
  const calcNames = React.useMemo(() => new Set((fields?.calculated ?? []).filter((f) => !isValueGroup(f)).map((f) => f.name)), [fields]);
  // BI-10: value groups and hierarchies are edited from the same strip as the calculated fields.
  const groupNames = React.useMemo(() => new Set((fields?.calculated ?? []).filter(isValueGroup).map((f) => f.name)), [fields]);
  const [groupOpen, setGroupOpen] = React.useState(false);
  const [editingGroup, setEditingGroup] = React.useState<CalcField | undefined>(undefined);
  const [hierarchyOpen, setHierarchyOpen] = React.useState(false);
  const [editingHierarchy, setEditingHierarchy] = React.useState<Hierarchy | undefined>(undefined);
  // Phase 2 QA R14: every column picked or shown in the form reads through `columnRef` (a joined
  // column is its own name with the table as a small tag), and truncates inside its control.
  const fieldLabel = (name: string) => {
    const metric = isMetricColumn(name) ? (fields?.metrics ?? []).find((x) => x.name === metricNameOf(name)) : undefined;
    const ref = columnRef(name, {
      metricLabels: Object.fromEntries((fields?.metrics ?? []).map((m) => [m.name, m.label])),
      calculated: calcNames, groups: groupNames, isParam: (n) => !!slotParam(n),
    });
    return <ColumnLabel reference={ref} title={metric?.description || undefined} />;
  };
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
  // The measures, each with an aggregate of its own or the chart's (BI-6). The
  // first three are also read as `measure`, `measure2` and `measure3`.
  const [measureRows, setMeasureRows] = React.useState<MeasureRow[]>([{ column: "", aggregate: "" }]);
  const measure = measureRows[0]?.column ?? "";
  const measure2 = measureRows[1]?.column ?? "";
  const measure3 = measureRows[2]?.column ?? "";
  /** Keeps only the measures the source still offers (a re-run, a deleted field). */
  const keepMeasures = (offered: readonly string[]) => setMeasureRows((rows) => rows.map((r) => ({ ...r, column: keepIfOffered(r.column, offered) })));
  // BI-6: the chart's own filters, the sort (with the three orders a chart always had), "Other", and a table's groupings.
  const [shape, setShape] = React.useState<ShapeDraft>(emptyShapeDraft);
  const setSortChoice = (sortChoice: string) => setShape((d) => ({ ...d, sortChoice }));
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
  const [limit, setLimit] = React.useState(20);
  // BI-11: the parameters (for the column slots, the limit and the reference lines), the number
  // parameter the limit follows ("" = the number above), and the chart's reference lines.
  const [paramDefs, setParamDefs] = React.useState<ParamDef[]>([]);
  const [limitParamName, setLimitParamName] = React.useState("");
  const [refLines, setRefLines] = React.useState<RefLineDraft[]>([]);
  // BI-13: the chart's other questions, and how its own series is drawn when it has them ("" = the kind's default).
  const [layers, setLayers] = React.useState<LayerDraft[]>([]);
  const [ownDraw, setOwnDraw] = React.useState("");
  const [ownAxis, setOwnAxis] = React.useState("");
  const [paramsOpen, setParamsOpen] = React.useState(false);
  // Phase 2 QA R3: "More options" is folded unless the chart being edited uses something in it.
  const [moreOpen, setMoreOpen] = React.useState(false);
  // Phase 2 QA R10: sections 1 to 3 are an accordion (one open, or none); a new chart starts on 1, an edited one folded.
  const [active, setActive] = React.useState<1 | 2 | 3 | null>(1);
  const panelRefs = React.useRef<Partial<Record<SectionNumber, HTMLElement | null>>>({});
  const [focusAsk, setFocusAsk] = React.useState<{ section: SectionNumber; n: number } | null>(null);
  // Phase 2 QA R9: the type list at the left is open or folded; a remembered choice wins, else open for a new chart until a type is chosen.
  const [listOpen, setListOpen] = React.useState(true);
  const [kindMenu, setKindMenu] = React.useState(false);
  const numberParamDefs = React.useMemo(() => numberParams(paramDefs), [paramDefs]);
  const [targetBoard, setTargetBoard] = React.useState(board);
  // What a click does instead of the drill menu (BI-18·B); undefined = the menu.
  const [click, setClick] = React.useState<ChartClick | undefined>(undefined);
  // How a date or timestamp dimension is grouped (BI-9); "" = as stored.
  const [grain, setGrain] = React.useState<Grain | "">("");
  // Whether the person (or a saved chart) set the limit, so choosing a grain
  // only replaces the untouched default (BI-9 review fix R2).
  const [limitTouched, setLimitTouched] = React.useState(false);
  // Raw table, pivot and KPI comparison settings (BI-16 part A).
  const [tableDraft, setTableDraft] = React.useState<TableDraft>(emptyTableDraft);
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
  const isPivot = kind === "pivot";
  // A pivot and a raw table name their columns in their own fields, not as a dimension and measures.
  const ownColumns = isPivot || (kind === "table" && tableDraft.mode === "rows");
  const rowsMode = kind === "table" && tableDraft.mode === "rows";
  // BI-6: a grouped table lists its groupings; the first is also its `dimension`.
  const isGroupedTable = kind === "table" && !rowsMode;
  const groupingNames = isGroupedTable ? shape.groupings.map((g) => g.column) : [dimension].filter(Boolean);
  const limits = measureLimits(kind, rowsMode);
  /** The legacy order, read by the limit and grain rules: a sort by a grouping is "none", by a measure a value order. */
  const order = effectiveOrder(shape.sortChoice, groupingNames);
  const wholeMeasure = React.useMemo(
    () => new Set([
      ...(fields?.calculated ?? []).filter((f) => f.level === "aggregate" || f.level === "table").map((f) => f.name),
      // BI-2: a metric has its own aggregate, so the row's aggregate does not apply to it.
      ...(fields?.metrics ?? []).map((m) => metricColumn(m.name)),
    ]),
    [fields],
  );
  const metricRow = measureRows.find((r) => isMetricColumn(r.column));
  const pickedMetric = metricRow ? (fields?.metrics ?? []).find((m) => m.name === metricNameOf(metricRow.column)) : undefined;
  // Fix the number of measure rows to what the kind takes.
  React.useEffect(() => {
    setMeasureRows((rows) => {
      if (rows.length === Math.min(Math.max(rows.length, limits.min), limits.max || 1) && rows.length >= limits.min) return rows;
      const next = rows.slice(0, Math.max(limits.max, 1));
      while (next.length < limits.min) next.push({ column: "", aggregate: "" });
      return next;
    });
  }, [limits.min, limits.max]);
  const sourceColumns = React.useMemo<SourceColumns>(() => ({
    all: [...(fields?.dimensions ?? []), ...(fields?.measures ?? [])],
    dates: Object.entries(fields?.kinds ?? {}).filter(([, k]) => k === "date" || k === "datetime").map(([name]) => name),
  }), [fields]);
  // The grains this dimension and chart kind allow; empty hides the picker. A
  // pivot's grain applies to its first date row or column field.
  const pivotDateField = isPivot ? [...tableDraft.pivotRows, ...tableDraft.pivotColumns].find((c) => fields?.kinds?.[c] === "date" || fields?.kinds?.[c] === "datetime") : undefined;
  const dimensionKind = isPivot ? (pivotDateField ? fields?.kinds?.[pivotDateField] : undefined) : fields?.kinds?.[dimension];
  const grainOptions = React.useMemo(() => grainChoices(kind, dimensionKind), [kind, dimensionKind]);
  // A calendar on a timestamp groups by day: the server would otherwise keep one value per day silently.
  const grainRequired = isCalendar && dimensionKind === "datetime";
  // A calendar is one cell per day, so up to a year of rows; a grouped chart
  // may show up to a thousand buckets (the server keeps the latest).
  const maxLimit = grain ? 1000 : isCalendar ? 366 : 100;
  const takesLines = ["bar", "hbar", "line", "area", "stacked", "combo"].includes(kind);
  // Switching to a calendar starts from a full year; leaving it brings the
  // Top-N back inside 1–100.
  React.useEffect(() => {
    if (grain) return;
    if (isCalendar) setLimit((l) => (l === 20 ? 366 : l));
    else setLimit((l) => Math.min(l, 100));
  }, [isCalendar, grain]);
  // A grain the chosen column or chart kind does not allow is dropped; a
  // calendar on a timestamp is set to day. Waits for the columns to load so
  // an edit's saved grain is not cleared before its kinds are known.
  React.useEffect(() => {
    if (!fields?.kinds) return;
    if (grainRequired) { setGrain("day"); return; }
    setGrain((g) => (g && !grainOptions.includes(g) ? "" : g));
  }, [fields, grainOptions, grainRequired]);
  /**
   * Phase 2 QA R6: switching the kind keeps what still fits. A chart becoming a grouped table
   * makes its dimension (and breakdown) the first groupings; a grouped table becoming a chart
   * makes its first grouping the dimension. A kind that takes one chart-wide aggregate keeps the
   * first measure's.
   */
  function pickKind(next: ChartKind) {
    if (next === kind) return;
    const patch = carryOverOnKindSwitch({
      from: kind, to: next, rowsMode, dimension, breakdown, grain: grain && grainOptions.includes(grain) ? grain : "",
      dimensionBin: binPayload(shape.bin), breakdownBin: binPayload(shape.breakdownBin), groupings: shape.groupings,
    });
    if (patch.groupings) setShape((d) => ({ ...d, groupings: patch.groupings ?? d.groupings }));
    if (patch.dimension !== undefined) setDimension(patch.dimension);
    if (patch.grain !== undefined) setGrain(isGrain(patch.grain) ? patch.grain : "");
    if (patch.bin !== undefined) setShape((d) => ({ ...d, bin: binDraftFromDef(patch.bin) }));
    if (!measureLimits(next).own && measureRows[0]?.aggregate) {
      setAggregate(measureRows[0].aggregate);
      setMeasureRows((rows) => rows.map((r) => ({ ...r, aggregate: "" })));
    }
    setKind(next);
  }
  function pickGrain(next: Grain | "") {
    // A grouped chart reads best in date order; the editor can still change it.
    if (next && !grain && shape.sortChoice === "order:desc") setSortChoice("order:none");
    // BI-9 review fix (SHOULD-FIX) R2: a day-grained chart should not open at 20 days.
    setLimit((l) => limitAfterGrainPick(next, l, limitTouched));
    setGrain(next);
  }
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
  const canBreakdown = kind === "bar" || kind === "hbar" || kind === "line" || kind === "area" || needsBreakdown;
  // BI-10: a number column can be the dimension or the breakdown when it is put in bins; a table does
  // that per grouping, a pivot on its first number row or column field.
  const onSqlSource = choice?.kind === "sql" || source === NEW_SQL_CHOICE;
  // Phase 2 QA R5: what applies to this kind and to what is already set, from one function (lib/builder-rules).
  const rules = builderRules({
    kind, rowsMode, grained: !!(grain && grainOptions.includes(grain)), breakdown: canBreakdown && !!breakdown,
    other: shape.other, steps: shape.steps.length, layers: layers.length,
    multiGrouped: isGroupedTable && shape.groupings.length > 1,
    limitHasEffect: limitHasEffect(grain, isCalendar ? "none" : order),
    hasSource: !!choice || source === NEW_SQL_CHOICE,
  });
  const binKind = takesBin(kind, rowsMode) && !isGroupedTable && !isPivot && rules.bins;
  const numberColumns = React.useMemo(
    () => (fields?.measures ?? []).filter((m) => !wholeMeasure.has(m) && fields?.kinds?.[m] === "number"),
    [fields, wholeMeasure],
  );
  const dimensionChoices = binKind ? [...(fields?.dimensions ?? []), ...numberColumns] : fields?.dimensions ?? [];
  const breakdownChoices = (binKind ? [...(fields?.dimensions ?? []), ...numberColumns] : fields?.dimensions ?? []).filter((d) => d !== dimension);
  const startingBin = onSqlSource ? ({ mode: "width", count: "10", width: "" } as const) : AUTO_BIN;
  const isNumber = (column: string) => fields?.kinds?.[column] === "number";
  function pickDimension(column: string) {
    setDimension(column);
    // BI-10 review fix (SHOULD-FIX) R1: bins read best in bin order, as dates do; the
    // editor can still change it. Only the untouched default is replaced.
    setShape((d) => ({
      ...d,
      bin: binKind && isNumber(column) ? d.bin ?? startingBin : null,
      sortChoice: binKind && isNumber(column) && d.bin === null && d.sortChoice === "order:desc" ? "order:none" : d.sortChoice,
    }));
  }
  function pickBreakdown(column: string) {
    setBreakdown(column);
    setShape((d) => ({ ...d, breakdownBin: binKind && column && isNumber(column) ? d.breakdownBin ?? startingBin : null }));
  }
  const pivotNumberField = isPivot ? [...tableDraft.pivotRows, ...tableDraft.pivotColumns].find((c) => isNumber(c)) : undefined;
  const mLabels = isSingle ? ["Measure"] : MEASURE_LABELS[kind] ?? ["Measure (Y)"];
  const measureTitle = (i: number) => (limits.max > 1 && limits.max === limits.min ? mLabels[i] ?? `Measure ${i + 1}` : i === 0 ? mLabels[0] : `Measure ${i + 1}`);
  const dimensionLabel = isChoropleth ? "Region column" : isPoints ? "Label" : "Dimension (X)";
  const breakdownLabel = kind === "sankey" ? "Flows to" : kind === "sunburst" ? "Outer ring" : isHeatmap ? "2nd dimension (Y)" : "Breakdown / series";
  // Phase 2 QA R20: the title is built from the labels the pickers show (a metric's label, a
  // joined column's own name), never the stored reference (`metric:name`, `table__column`).
  const titleText = (name: string) => (name ? columnRef(name, {
    metricLabels: Object.fromEntries((fields?.metrics ?? []).map((m) => [m.name, m.label])),
    calculated: calcNames, groups: groupNames, isParam: (n) => !!slotParam(n),
  }).text : name);
  const suggestedTitle = suggestChartTitle({
    kind, dimension: isSingle ? undefined : titleText(dimension),
    measures: measureRows.slice(0, Math.max(limits.min, 1)).map((r) => titleText(r.column)),
    breakdown: canBreakdown ? titleText(breakdown) : undefined,
  });
  const canWriteSql = useAuth().hasPermission("dashboard:sql");
  const isNewSql = source === NEW_SQL_CHOICE;
  // Fields and picks follow the columns of the latest successful Run, and a
  // column a re-run no longer returns is dropped rather than kept dangling.
  function applyRunColumns(columns: SqlSourceColumn[]) {
    const f: Fields = { ...fieldsFromColumns(columns), kinds: kindsOf(columns) };
    const any = [...f.measures, ...f.dimensions];
    setFields(f);
    setDimension((v) => keepIfOffered(v, f.dimensions));
    keepMeasures(f.measures);
    setShape((d) => ({ ...d, groupings: d.groupings.filter((g) => f.dimensions.includes(g.column) || f.measures.includes(g.column)) }));
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
  // BI-7: what the join editor knows of each table (columns, sorting key), and the joins the pickers were last loaded for.
  const [joinInfo, setJoinInfo] = React.useState<Record<string, JoinTableInfo>>({});
  const joinsSeen = React.useRef("[]");
  const baseMart = choice?.kind === "mart" ? choice.name : "";
  async function learnTable(mart: string, against: string): Promise<{ info: JoinTableInfo; pairs?: JoinPairDef[] } | null> {
    try {
      const [meta, suggested] = await Promise.all([
        apiFetch(`/api/dashboard/fields?mart=${encodeURIComponent(mart)}`).then((r) => r.json()),
        against && against !== mart ? dashboardService.joinSuggestions(against, mart) : Promise.resolve(null),
      ]);
      const info = { columns: (meta.columns ?? []).map((c: { name: string }) => c.name), rightKey: suggested?.rightKey ?? [] };
      setJoinInfo((prev) => ({ ...prev, [mart]: info }));
      return { info, pairs: suggested ? pairsFromSuggestions(suggested) : undefined };
    } catch { return null; }
  }
  // A table was picked for join `index`: learn its columns and apply the suggested pairs.
  async function pickJoinTable(index: number, mart: string) {
    setShape((d) => ({ ...d, joins: d.joins.map((j, k) => (k === index ? { ...j, mart, on: [{ left: "", op: "=", right: "" }] } : j)) }));
    const learned = await learnTable(mart, baseMart);
    if (learned?.pairs) {
      setShape((d) => ({ ...d, joins: d.joins.map((j, k) => (k === index && j.mart === mart ? { ...j, on: learned.pairs ?? j.on } : j)) }));
    }
  }
  // The tables of a chart being edited (or a base table just picked) are learned once.
  const joinMarts = [...new Set([baseMart, ...shape.joins.map((j) => j.mart)].filter(Boolean))].join(",");
  React.useEffect(() => {
    if (!open) return;
    for (const mart of joinMarts.split(",").filter(Boolean)) {
      if (!joinInfo[mart]) void learnTable(mart, baseMart);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `joinInfo` and `learnTable` only cache what `joinMarts` names
  }, [open, joinMarts]);
  // The pickers list the flat relation's columns, so they follow the joins that are complete.
  const joinsKey = JSON.stringify(joinsPayload(shape.joins).joins ?? []);
  React.useEffect(() => {
    if (!open || !baseMart || joinsSeen.current === joinsKey) return;
    joinsSeen.current = joinsKey;
    void loadFields(source).then((f) => {
      setDimension((v) => keepIfOffered(v, f.dimensions));
      keepMeasures(f.measures);
      setBreakdown((v) => keepIfOffered(v, f.dimensions));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reload only when the complete joins change
  }, [open, joinsKey]);
  // BI-13: questions can be added to a bar, line, area or combo chart; they are matched on its
  // dimension, so a breakdown, "Other", steps or bins on the chart are refused with the questions.
  const layersApply = takesLayers(kind) && !rowsMode && layers.length > 0;
  const layersBlocked = [
    ...(canBreakdown && breakdown ? ["a breakdown"] : []),
    ...(shape.other ? ["the 'Other' group"] : []),
    ...(shape.steps.length ? ["steps"] : []),
    ...(shape.bin || shape.breakdownBin ? ["bins"] : []),
  ];
  const noFieldsHint = isNewSql ? "run the SQL first" : "pick a source first";
  // Required fields still empty: listed in the preview before the user
  // hits Create, and the same list is the save-time error.
  const missing: string[] = isText
    ? [...(title.trim() ? [] : ["Title"]), ...(text.trim() ? [] : ["Content"])]
    : [
        ...(choice || isNewSql ? [] : ["Data source"]),
        ...(!isSingle && !isPoints && !ownColumns && !isGroupedTable && !dimension ? [dimensionLabel] : []),
        ...(isPoints && !lat ? ["Latitude"] : []),
        ...(isPoints && !lon ? ["Longitude"] : []),
        ...(ownColumns ? [] : measureRows.slice(0, limits.min).flatMap((r, i) => (r.column ? [] : [measureTitle(i)]))),
        ...(needsBreakdown && !breakdown ? [breakdownLabel] : []),
        ...draftProblems(kind, tableDraft, !!pickedMetric?.timeColumn),
        ...(layersApply ? layerProblems(layers) : []),
        ...shapeProblems(shape, { kind, rowsMode, grained: !!(grain && grainOptions.includes(grain)), breakdown: !!(canBreakdown && breakdown) }),
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

  async function loadFields(value: string, joins: readonly JoinDraft[] = shape.joins): Promise<Fields> {
    const picked = decodeSourceChoice(value);
    if (!picked) { setFields(null); return { dimensions: [], measures: [] }; }
    // BI-7: with joins the columns are the flat relation's (`table__column`).
    const sent = picked.kind === "mart" ? joinsPayload(joins).joins : undefined;
    const query = sent ? `${fieldsQuery(picked)}&joins=${encodeURIComponent(JSON.stringify(sent))}` : fieldsQuery(picked);
    const res = await apiFetch(`/api/dashboard/fields?${query}`);
    // Joins the server refuses leave the pickers as they were; the preview says why.
    if (sent && !res.ok) return fields ?? { dimensions: [], measures: [] };
    const j = await res.json();
    const f = withCalculated(
      { dimensions: j.dimensions ?? [], measures: j.measures ?? [], kinds: kindsOf(j.columns ?? []) },
      (j.columns ?? []).map((c: { name: string }) => c.name),
      j.calculated ?? [],
    );
    f.hierarchies = j.hierarchies ?? [];
    // BI-2: the metrics of this source; a source with none (or a viewer who cannot list them) just has none.
    f.segments = (await metricService.listSegments().catch(() => [] as Segment[])).filter((g) =>
      picked.kind === "mart" ? g.sourceKind === "mart" && g.sourceId === picked.name : g.sourceKind === "sql_source" && g.sourceId === picked.id);
    f.metrics = (await metricService.list().catch(() => [] as Metric[])).filter((m) =>
      picked.kind === "mart" ? m.sourceKind === "mart" && m.sourceId === picked.name : m.sourceKind === "sql_source" && m.sourceId === picked.id);
    // BI-11: a list parameter can stand for a column; it is offered wherever a column is picked,
    // and the server checks every one of its values as a column of this chart's data when it is saved.
    const defs = await paramService.list().catch(() => [] as ParamDef[]);
    setParamDefs(defs);
    for (const p of columnSwitchParams(defs)) {
      f.dimensions.push(slotText(p.name));
      f.measures.push(slotText(p.name));
    }
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
    const pref = readKindListPref();
    setListOpen(pref ? pref === "expanded" : !editId);
    setKindMenu(false);
    setActive(editId ? null : 1);
    if (initial) {
      setTitle(initial.title ?? "");
      setKind((initial.kind as ChartKind) ?? "hbar");
      // Phase 2 QA R4: a measure that stores no aggregate shows this one; saving it back sends the same `aggregate`.
      setAggregate(initial.aggregate ?? "sum");
      setSpan(initial.span === 2 ? 2 : 1);
      setBreakdown(initial.breakdown ?? "");
      setMapId(initial.map ?? "");
      setLat(initial.lat ?? "");
      setLon(initial.lon ?? "");
      setCaption(initial.caption ?? "");
      setTarget(initial.target != null ? String(initial.target) : "");
      setText(initial.text ?? "");
      const savedShape = shapeDraftFromDef(initial);
      // A grouped table saved with a plain dimension has that one grouping.
      if (initial.kind === "table" && initial.tableMode !== "rows" && !savedShape.groupings.length && initial.dimension) savedShape.groupings = [{ column: initial.dimension }];
      setShape(savedShape);
      // At once, not after the columns load: a saved sort names a measure or a grouping, and the
      // check that drops a sort the chart no longer has must see them.
      setDimension(initial.dimension ?? savedShape.groupings[0]?.column ?? "");
      setMeasureRows(rowsFromEntries(initial.measures));
      setLimitParamName(limitParam(initial.limit) ?? "");
      setLimit(typeof initial.limit === "number" ? initial.limit : 20);
      setRefLines(refLinesFromDef(initial.referenceLines));
      setLayers(layerDraftsFromDef(initial.layers));
      setOwnDraw(initial.draw ?? "");
      setOwnAxis(initial.axis ?? "");
      setLimitTouched(true);
      setMoreOpen(moreOptionsUsed({
        steps: savedShape.steps.length, layers: initial.layers?.length ?? 0, referenceLines: initial.referenceLines?.length ?? 0,
        click: !!initial.click,
        columnSettings: initial.kind === "table" && initial.tableMode !== "rows" ? Object.keys(initial.columnSettings ?? {}).length : 0,
        compare: initial.kind === "kpi" && !!initial.compare,
      }));
      setTargetBoard(initial.board ?? board);
      setClick(initial.click);
      setGrain(isGrain(initial.grain) ? initial.grain : "");
      setTableDraft(draftFromDef(initial));
      const m = sourceValueFromDef(initial);
      setSource(m);
      if (m) {
        joinsSeen.current = JSON.stringify(joinsPayload(savedShape.joins).joins ?? []);
        void loadFields(m, savedShape.joins).then(() => {
          setDimension(initial.dimension ?? savedShape.groupings[0]?.column ?? "");
          setMeasureRows(rowsFromEntries(initial.measures));
        });
      }
    } else {
      setTargetBoard(board);
      setMoreOpen(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function reset() {
    setTitle(""); setSource(""); setKind("hbar"); setKindQuery(""); setDimension("");
    setMeasureRows([{ column: "", aggregate: "" }]); setBreakdown(""); setMapId(""); setLat(""); setLon("");
    setAggregate("sum"); setSpan(1);
    setCaption(""); setTarget(""); setText(""); setShape(emptyShapeDraft()); setLimit(20); setLimitTouched(false); setLimitParamName(""); setRefLines([]); setLayers([]); setOwnDraw(""); setOwnAxis("");
    setTargetBoard(board); setClick(undefined); setMoreOpen(false); setActive(1); setGrain(""); setTableDraft(emptyTableDraft()); setFields(null); setError(null); setPreview(null); setPreviewError(null);
    draft.cancel();
    joinsSeen.current = "[]";
  }

  // User changes the data source → reset the column selections and reload.
  function onSourceChange(value: string) {
    draft.cancel();
    setSource(value); setDimension(""); setMeasureRows([{ column: "", aggregate: "" }]); setBreakdown("");
    setLat(""); setLon("");
    // The filters and groupings name the old source's columns.
    setShape((d) => ({ ...emptyShapeDraft(), sortChoice: d.sortChoice, other: d.other }));
    // The columns of a raw table, a pivot or a comparison belong to the old source.
    setTableDraft((d) => ({ ...emptyTableDraft(), mode: d.mode, totals: d.totals, goodDirection: d.goodDirection, compareKind: d.compareKind }));
    if (value === NEW_SQL_CHOICE) { setFields(null); draft.startNew(); return; }
    if (value) void loadFields(value); else setFields(null);
  }

  /** Back to the saved source: the pickers list its stored columns again. */
  function cancelSqlEdit() {
    draft.cancel();
    if (!source) return;
    void loadFields(source).then((f) => {
      setDimension((v) => keepIfOffered(v, f.dimensions));
      keepMeasures(f.measures);
      setBreakdown((v) => keepIfOffered(v, f.dimensions));
    });
  }

  // The result names of the measures ("avg_stay") and the columns a grouped table shows,
  // which its column settings are kept for.
  const resultNames = measureRows.filter((r) => r.column).map((r) => resultName(limits.own ? r : { ...r, aggregate: "" }, wholeMeasure));
  const shownColumns = [...new Set([...groupingNames, ...resultNames])];
  const shapeContext = { kind, rowsMode, grained: !!(grain && grainOptions.includes(grain)), breakdown: !!(canBreakdown && breakdown) };
  // BI-2: saved metrics of the source are offered where an aggregate field is (not maps, box plots, pivots or raw rows).
  const takesMetrics = !ownColumns && !isPoints && kind !== "boxplot" && kind !== "text";
  const measureColumns = [
    ...(fields?.measures ?? []).filter((m) => !isPoints || (m !== lat && m !== lon)),
    ...(takesMetrics ? (fields?.metrics ?? []).map((m) => metricColumn(m.name)) : []),
  ];
  // Columns a filter or a grouping may name: the source's, with its per-row calculated fields.
  const columnChoices = React.useMemo<ColumnChoice[]>(() => {
    const names = [...new Set([...(fields?.dimensions ?? []), ...(fields?.measures ?? [])])].filter((n) => !wholeMeasure.has(n));
    return names.map((name) => ({ name, kind: fields?.kinds?.[name] ?? "text" }));
  }, [fields, wholeMeasure]);
  // The chart's grouped result, and (BI-6 part 2) what is left after its steps: the sort names the last one's columns.
  const baseResult: ResultColumns = {
    groups: (isGroupedTable ? shape.groupings : [{ column: dimension } as { column: string; grain?: string }]).filter((g) => g.column).map((g) => {
      const kind = fields?.kinds?.[g.column] ?? "text";
      return { name: g.column, kind: g.grain ? (isGrain(g.grain) && !isTruncation(g.grain) ? "number" : kind === "datetime" && (g.grain === "minute" || g.grain === "hour") ? "datetime" : "date") : kind };
    }),
    measures: resultNames,
    labels: Object.fromEntries(measureRows.filter((r) => r.column).map((r) => {
      const row = limits.own ? r : { ...r, aggregate: "" };
      return [resultName(row, wholeMeasure), measureLabel(row, wholeMeasure, aggregate)];
    })),
    // Phase 2 QA R8: a later step names its output from the column it was computed from.
    bases: Object.fromEntries(measureRows.filter((r) => r.column).map((r) => [resultName(limits.own ? r : { ...r, aggregate: "" }, wholeMeasure), r.column])),
  };
  const stepsOn = stepsApply(shape, shapeContext);
  const finalResult = stepsOn ? columnsAfterSteps(baseResult, shape.steps) : baseResult;
  const sortChoices = sortOptions(
    finalResult.groups.map((g) => g.name),
    finalResult.measures.map((name) => ({ name, label: finalResult.labels[name] ?? name })),
  );
  const sortLabels = Object.fromEntries(sortChoices.map((o) => [o.value, o.label]));
  // A sort by a measure or grouping the chart no longer has goes back to the default order.
  React.useEffect(() => {
    if (!fields) return;
    if (shape.sortChoice.startsWith("by:") && !sortChoices.some((o) => o.value === shape.sortChoice)) setSortChoice("order:desc");
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `sortChoices` is rebuilt every render; its content is what matters
  }, [fields, shape.sortChoice, JSON.stringify(sortChoices)]);

  // Phase 2 QA R10: the one-line summaries a folded section shows (lib/section-summary), in the labels used everywhere else.
  const summaries = sectionSummaries({
    kind, rowsMode, source: sourceLabels[source] ?? "", joins: shape.joins.length,
    dimension, grain: grain && grainOptions.includes(grain) ? grain : "", breakdown: canBreakdown ? breakdown : "",
    groupings: shape.groupings.map((g) => g.column), rowColumns: tableDraft.columns,
    pivot: { rows: tableDraft.pivotRows, columns: tableDraft.pivotColumns, values: tableDraft.pivotValues.filter((v) => v.column).map((v) => v.column) },
    measures: measureRows.filter((r) => r.column).map((r) => measureLabel(limits.own ? r : { ...r, aggregate: "" }, wholeMeasure, aggregate)),
    mapLabel: isMap ? MAP_LABELS[effectiveMap] ?? "" : "", lat, lon,
    sort: rules.sort === "choice" ? sortLabels[shape.sortChoice] ?? "" : rules.sort === "by-date" ? "by date" : rules.sort === "by-category" ? "by category" : "",
    limit: rules.limit ? (limitParamName ? "param" : Math.min(limit, maxLimit)) : null,
    filters: rules.filters ? shape.filters.length : 0, other: shape.other && rules.other.shown,
  });
  /** Opens the section that holds a missing field and moves focus to that field. */
  function revealSection(n: SectionNumber) {
    if (n === 4) setMoreOpen(true); else setActive(n);
    setFocusAsk({ section: n, n: Date.now() });
  }
  React.useEffect(() => {
    if (!focusAsk) return;
    focusFirstEmpty(panelRefs.current[focusAsk.section] ?? null);
    setFocusAsk(null);
  }, [focusAsk]);
  /** A type was chosen from the list or the menu; with no remembered choice the list folds away. */
  function chooseKind(next: ChartKind) {
    pickKind(next);
    setKindMenu(false);
    if (!readKindListPref()) setListOpen(false);
  }
  function toggleList() {
    const next = !listOpen;
    setListOpen(next);
    setKindMenu(false);
    writeKindListPref(next ? "expanded" : "collapsed");
  }

  /** The shape fields to send; a pivot's bin only while it still has a number field to bin (BI-10). */
  function shapeOut(): Record<string, unknown> {
    const out = shapePayload(shape, shapeContext);
    if (isPivot && !pivotNumberField) delete out.bin;
    return out;
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
        ...draftPayload(kind, tableDraft),
        ...shapeOut(),
      };
    } else if (ownColumns) {
      // BI-16 part A: no dimension, measures or aggregate; the columns travel in the table's own fields.
      payload = {
        title: payloadTitle, ...sourcePayload(withSource), kind, span, board: targetBoard,
        ...draftPayload(kind, tableDraft),
        grain: isPivot && grain && grainOptions.includes(grain) ? grain : undefined,
        ...shapeOut(),
      };
    } else {
      const measures = entriesFromRows(measureRows, limits.own);
      const legacyOrder = parseSortChoice(shape.sortChoice).order;
      payload = {
        title: payloadTitle, ...sourcePayload(withSource), kind,
        // Several groupings (or a grain on one) travel in `groupings`, which takes the place of the dimension.
        dimension: isGroupedTable && usesGroupings(shape.groupings) ? "" : dimension,
        measures, aggregate, span, board: targetBoard,
        breakdown: canBreakdown && breakdown ? breakdown : undefined,
        map: isMap ? effectiveMap : undefined,
        lat: isPoints ? lat : undefined, lon: isPoints ? lon : undefined,
        // A point map has a fixed cap of its own (lakehouse_bi::builder::POINT_LIMIT) and is not sorted.
        order: isCalendar ? "none" : isPoints ? undefined : legacyOrder,
        limit: isPoints ? undefined : limitParamName ? { param: limitParamName } : Math.min(limit, maxLimit),
        referenceLines: takesLines && refLinesPayload(refLines).length ? refLinesPayload(refLines) : undefined,
        layers: layersApply ? layersPayload(layers) : undefined,
        draw: layersApply && ownDraw ? ownDraw : undefined,
        axis: layersApply && ownAxis ? ownAxis : undefined,
        // Only a grain the column and kind allow is sent (BI-9); the server checks again.
        grain: grain && grainOptions.includes(grain) ? grain : undefined,
        ...draftPayload(kind, tableDraft, shownColumns),
        ...shapeOut(),
      };
    }
    // A chart without marks to click (text, KPI, gauge, table, density map)
    // has no click setting; one left over from another kind is dropped.
    const clickable = kindHasClickValue(kind);
    const problem = clickable ? clickProblem(click) : null;
    if (problem && !forPreview) throw new Error(problem);
    if (clickable && click && !problem) payload.click = click;
    if (isEdit) payload.id = editId;
    // BI-11: the preview is drawn with the values the dashboard shows; they are not part of the chart.
    if (forPreview && paramValues && Object.keys(paramValues).length) payload.params = paramValues;
    return payload;
  }

  const previewInputsKey = previewKey({
    title, source, kind, dimension, measure, measure2, measure3, breakdown, mapId, lat, lon,
    aggregate, span, caption, target, text, order: shape.sortChoice, limit, targetBoard, grain,
    // BI-16 part A: every table, pivot and comparison setting reaches the SQL or the render spec.
    tables: JSON.stringify(draftPayload(kind, tableDraft, shownColumns)),
    // BI-6: each measure's own aggregate, the chart's own filters, the sort, "Other" and the groupings.
    shape: JSON.stringify({ measures: measureRows, shape }),
    // BI-8: a changed formula changes the rows a chart over the field returns.
    calc: JSON.stringify((fields?.calculated ?? []).map((f) => [f.name, f.formula])),
    // BI-11: the values the preview is drawn with, the limit's parameter and the reference lines.
    params: JSON.stringify({ values: paramValues ?? null, limit: limitParamName, lines: refLines }),
    // BI-13: each question, and how the chart's own series is drawn, reaches the statements and the drawing.
    layers: JSON.stringify(layersApply ? { layers, ownDraw, ownAxis } : null),
  });
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
        if (!controller.signal.aborted) setPreview({ ...next, spec: withPreviewDef(next.spec, payload) });
      }).catch((e: unknown) => {
        if (!controller.signal.aborted) { setPreview(null); setPreviewError(e instanceof Error ? e.message : String(e)); }
      }).finally(() => { if (!controller.signal.aborted) setPreviewBusy(false); });
    }, 450);
    return () => { window.clearTimeout(timer); controller.abort(); };
    // BI-9 review fix (BLOCKER) R3: `previewInputsKey` holds every field that affects the
    // generated SQL or render spec, `grain` included (lib/preview-key.ts).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, previewInputsKey, heldPreview, previewSql]);

  async function save() {
    setError(null);
    if (sqlActive && draft.blocker) { draft.markSaveAttempted(); setError(draft.blocker); return; }
    // Phase 2 QA R10: a required field that is missing opens its section and takes focus.
    if (missing.length) {
      revealSection(missing.map(missingSection).reduce((a, b) => (b < a ? b : a)));
      setError(`Still needed: ${missing.join(", ")}.`);
      return;
    }
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

  const grainPicker = grainOptions.length > 0 ? (
    <div className="grid gap-1.5">
      <Label>Group by</Label>
      <Select value={grain || NO_GRAIN} items={GRAIN_ITEMS} onValueChange={(v) => pickGrain(v === NO_GRAIN || !isGrain(v) ? "" : v)}>
        <SelectTrigger className="w-full" aria-label="Group by"><SelectValue /></SelectTrigger>
        <SelectContent>
          {grainRequired ? null : <SelectItem value={NO_GRAIN}>No grouping</SelectItem>}
          {grainOptions.map((g) => <SelectItem key={g} value={g}>{GRAIN_LABELS[g]}</SelectItem>)}
        </SelectContent>
      </Select>
    </div>
  ) : null;

  // Phase 2 QA R15: which conditional controls the dimension and the breakdown bring (they sit in a row below the fields).
  const binDimension = !isGroupedTable && !!binKind && isNumber(dimension);
  const dimensionExtras = !isGroupedTable && (!!grainPicker || binDimension);
  const breakdownExtras = canBreakdown && !!binKind && !!breakdown && isNumber(breakdown);

  const formFields = (
    // Phase 2 QA R14, the cause of the sideways scroll: a grid with no column template has one `auto`
    // track, whose minimum is the widest unbreakable content in it (a trigger's `nowrap` value), so
    // one long column name widened the whole form. Every grid of the form without columns of its own
    // gets a `minmax(0, 1fr)` track, which the container, not the content, sizes.
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-5 px-5 pt-1 pb-5 [&_.grid:not([class*=grid-cols])]:grid-cols-[minmax(0,1fr)]">
      {/* Phase 2 QA R11: title wide, dashboard and width narrow, on one row. */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_9rem_8rem]">
        <div className="grid gap-1.5">
          <Label htmlFor="ch-title">Title {isText ? <Req /> : null}</Label>
          <Input
            id="ch-title" value={title} onChange={(e) => setTitle(e.target.value)}
            placeholder={suggestedTitle || "e.g. Visitors by Region"}
          />
        </div>
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
          {/* Phase 2 QA R3: four ordered sections; the last is folded. */}
          <AccordionSection number={1} title="Data" summary={summaries.data} open={active === 1} onOpenChange={(o) => setActive(o ? 1 : null)} panelRef={(n) => { panelRefs.current[1] = n; }}>
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
            {choice?.kind === "mart" ? (
              <JoinsSection
                baseMart={baseMart}
                baseColumns={joinInfo[baseMart]?.columns ?? []}
                marts={marts.map((m) => m.name)}
                joins={shape.joins}
                onChange={(joins) => setShape((d) => ({ ...d, joins }))}
                onPickTable={(i, mart) => void pickJoinTable(i, mart)}
                infoOf={(mart) => joinInfo[mart]}
                disabled={false}
              />
            ) : null}
            {choice?.kind === "sql" && draft.mode !== "edit" ? (
              canWriteSql ? (
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                  <p className="text-xs text-muted-foreground">Custom SQL source.</p>
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
                  Custom SQL source. You need the dashboard:sql permission to edit it.
                </p>
              )
            ) : null}
            {sqlActive ? <SqlSourcePanel draft={draft} onCancelEdit={cancelSqlEdit} /> : null}
              {fields?.columns && !isNewSql && !isText && choice ? (
                <CustomFieldsMenu
                  calculated={fields.calculated ?? []} hierarchies={fields.hierarchies ?? []}
                  onEditField={(f) => { setEditing(f); setFormulaOpen(true); }}
                  onEditGroup={(f) => { setEditingGroup(f); setGroupOpen(true); }}
                  onEditHierarchy={(h) => { setEditingHierarchy(h); setHierarchyOpen(true); }}
                  onNewField={() => { setEditing(undefined); setFormulaOpen(true); }}
                  onNewGroup={() => { setEditingGroup(undefined); setGroupOpen(true); }}
                  onNewHierarchy={() => { setEditingHierarchy(undefined); setHierarchyOpen(true); }}
                />
              ) : null}
            </div>
          </AccordionSection>

          <AccordionSection number={2} title="What to show" summary={summaries.show} open={active === 2} onOpenChange={(o) => setActive(o ? 2 : null)} panelRef={(n) => { panelRefs.current[2] = n; }}>
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

          {kind === "table" ? <TableSection part="shape" draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} groupedColumns={shownColumns} /> : null}
          {!isSingle && !ownColumns ? (
            <div className="grid grid-cols-1 items-start gap-3 sm:grid-cols-2">
              {isGroupedTable ? (
                <GroupingsEditor
                  value={shape.groupings}
                  onChange={(groupings) => { setShape((d) => ({ ...d, groupings })); setDimension(groupings[0]?.column ?? ""); }}
                  columns={columnChoices}
                  disabled={!fields}
                  placeholder={fields ? "pick a column" : noFieldsHint}
                  fieldLabel={fieldLabel}
                  sqlSource={onSqlSource}
                />
              ) : (
              <div className="grid content-start gap-1.5" data-slot="dimension-field">
                <Label>{dimensionLabel} {isPoints ? <span className="font-normal text-muted-foreground">(optional, shown in the tooltip)</span> : <Req />}</Label>
                {isPoints ? (
                  <Select value={dimension || NO_BREAKDOWN} items={{ ...NO_LABEL_ITEMS, ...columnItems(fields?.dimensions ?? [], fieldLabel, dimension) }} onValueChange={(v) => setDimension(v === NO_BREAKDOWN ? "" : v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full" title={dimension ? displayColumnFull(dimension) : undefined}><SelectValue placeholder={fields ? "no label" : noFieldsHint} /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NO_BREAKDOWN}>— no label —</SelectItem>
                      {fields?.dimensions.map((d) => <SelectItem key={d} value={d}>{fieldLabel(d)}</SelectItem>)}
                    </SelectContent>
                  </Select>
                ) : (
                  <Select value={dimension} items={columnItems(dimensionChoices, fieldLabel, dimension)} onValueChange={(v) => pickDimension(v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full" title={dimension ? displayColumnFull(dimension) : undefined}><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                    <SelectContent>{dimensionChoices.map((d) => <SelectItem key={d} value={d}>{fieldLabel(d)}</SelectItem>)}</SelectContent>
                  </Select>
                )}
              </div>
              )}
              {canBreakdown ? (
                <div className="grid content-start gap-1.5" data-slot="breakdown-field">
                  <Label>{breakdownLabel} {needsBreakdown ? <Req /> : <span className="font-normal text-muted-foreground">(optional)</span>}</Label>
                  <Select value={breakdown || NO_BREAKDOWN} items={{ ...breakdownLabels, ...columnItems(breakdownChoices, fieldLabel, breakdown) }} onValueChange={(v) => pickBreakdown(v === NO_BREAKDOWN ? "" : v ?? "")} disabled={!fields || !!rules.breakdownLocked}>
                    <SelectTrigger className="w-full" title={breakdown ? displayColumnFull(breakdown) : undefined}><SelectValue placeholder={needsBreakdown ? "pick a 2nd column" : "no breakdown"} /></SelectTrigger>
                    <SelectContent>
                      {needsBreakdown ? null : <SelectItem value={NO_BREAKDOWN}>— no breakdown —</SelectItem>}
                      {breakdownChoices.map((d) => <SelectItem key={d} value={d}>{fieldLabel(d)}</SelectItem>)}
                    </SelectContent>
                  </Select>
                  {rules.breakdownLocked ? <p className="text-xs text-muted-foreground">{rules.breakdownLocked}</p> : null}
                </div>
              ) : null}
              {/*
                Phase 2 QA R15: a control that exists only for some columns (the bin of a number, the
                grain of a date) is never inside the field it belongs to, since it made that column
                taller than its neighbour and the neighbour's select drifted out of line with it. It
                sits in a row of its own below both fields, in its parent's column (the breakdown's
                in the second), and the fields above keep `content-start` so a taller neighbour
                cannot stretch them.
              */}
              {dimensionExtras || breakdownExtras ? (
                <div className="grid grid-cols-1 gap-3 sm:col-span-2 sm:grid-cols-2" data-slot="conditional-row">
                  {dimensionExtras ? (
                    <div className="grid content-start gap-3">
                      {grainPicker}
                      {binDimension ? (
                        <BinPicker label="Group in" value={shape.bin} sqlSource={onSqlSource} aria="Group in bins"
                          onChange={(bin) => setShape((d) => ({ ...d, bin }))} />
                      ) : null}
                    </div>
                  ) : null}
                  {breakdownExtras ? (
                    <div className="grid content-start gap-3 sm:col-start-2">
                      <BinPicker label="Group the breakdown in" value={shape.breakdownBin} sqlSource={onSqlSource} aria="Group the breakdown in bins"
                        onChange={(breakdownBin) => setShape((d) => ({ ...d, breakdownBin }))} />
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}
          {isPoints ? (
            <div className="grid grid-cols-1 items-start gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label>Latitude <Req /></Label>
                <Select value={lat} items={columnItems(coordinateColumns, fieldLabel, lat)} onValueChange={(v) => setLat(v ?? "")} disabled={!fields}>
                  <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                  <SelectContent>{coordinateColumns.filter((c) => c !== lon).map((c) => <SelectItem key={c} value={c}>{fieldLabel(c)}</SelectItem>)}</SelectContent>
                </Select>
              </div>
              <div className="grid gap-1.5">
                <Label>Longitude <Req /></Label>
                <Select value={lon} items={columnItems(coordinateColumns, fieldLabel, lon)} onValueChange={(v) => setLon(v ?? "")} disabled={!fields}>
                  <SelectTrigger className="w-full"><SelectValue placeholder={fields ? "pick a column" : noFieldsHint} /></SelectTrigger>
                  <SelectContent>{coordinateColumns.filter((c) => c !== lat).map((c) => <SelectItem key={c} value={c}>{fieldLabel(c)}</SelectItem>)}</SelectContent>
                </Select>
              </div>
            </div>
          ) : null}
          {isPoints ? <GridPicker value={shape.grid} onChange={(grid) => setShape((d) => ({ ...d, grid }))} disabled={!fields} /> : null}
          {isCalendar ? (
            <p className="-mt-2 text-xs text-muted-foreground">Pick a date column as the dimension; one cell per day, up to the last year of data.</p>
          ) : null}

          {isPoints ? <GridPicker value={shape.grid} onChange={(grid) => setShape((d) => ({ ...d, grid }))} disabled={!fields} /> : null}
          {isCalendar ? (
            <p className="text-xs text-muted-foreground">Pick a date column as the dimension; one cell per day, up to the last year of data.</p>
          ) : null}

          {ownColumns ? null : (
            <MeasureRows
              rows={measureRows} onChange={setMeasureRows} labels={measureTitle}
              min={limits.min} max={limits.max} own={limits.own} columns={measureColumns} defaultAggregate={aggregate}
              onDefaultAggregate={limits.own || isBoxplot ? undefined : setAggregate} fixedNote={isBoxplot ? "min, quartiles, max" : undefined}
              disabled={!fields} placeholder={fields ? "pick a column" : noFieldsHint} wholeMeasure={wholeMeasure} fieldLabel={fieldLabel}
            />
          )}
          {ownColumns ? null : isKpi ? (
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
          ) : null}
          {isPivot ? (
            <>
              <PivotSection draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} />
              {grainPicker}
              {pivotNumberField ? (
                <BinPicker label={`Group ${pivotNumberField} in`} value={shape.bin} sqlSource={onSqlSource} aria="Group the pivot field in bins"
                  onChange={(bin) => setShape((d) => ({ ...d, bin }))} />
              ) : null}
            </>
          ) : null}
          </AccordionSection>

          <AccordionSection number={3} title="Sort and filter" summary={summaries.sortAndFilter} open={active === 3} onOpenChange={(o) => setActive(o ? 3 : null)} panelRef={(n) => { panelRefs.current[3] = n; }}>
            {rules.sort !== "none" ? (
              <div className="grid grid-cols-1 items-start gap-3 sm:grid-cols-2">
                {rules.sort === "by-date" || rules.sort === "by-category" ? (
                  <div className="grid gap-1.5">
                    <Label>Sort</Label>
                    <p className="flex h-8 items-center text-xs text-muted-foreground">{rules.sort === "by-date" ? "By date" : "By category"}</p>
                  </div>
                ) : (
                  <div className="grid gap-1.5">
                    <Label>Sort</Label>
                    <Select value={shape.sortChoice} items={sortLabels} onValueChange={(v) => setSortChoice(v ?? "order:desc")}>
                      <SelectTrigger className="w-full" aria-label="Sort"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {sortChoices.map((o) => <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>)}
                      </SelectContent>
                    </Select>
                  </div>
                )}
                {rules.limit ? <div className="grid gap-1.5">
                  <Label>{grain ? "Buckets (max 1000, latest kept)" : isCalendar ? "Days (max 366)" : "Limit (Top N)"}</Label>
                  {limitParamName ? null : (
                    <Input type="number" min={1} max={maxLimit} value={limit} onChange={(e) => { setLimitTouched(true); setLimit(Math.max(1, Math.min(maxLimit, Number(e.target.value) || 20))); }} />
                  )}
                  {numberParamDefs.length ? (
                    <Select value={limitParamName || FIXED_LIMIT} items={{ [FIXED_LIMIT]: "A fixed number", ...Object.fromEntries(numberParamDefs.map((p) => [p.name, p.label || p.name])) }}
                      onValueChange={(v) => setLimitParamName(v && v !== FIXED_LIMIT ? v : "")}>
                      <SelectTrigger className="w-full" aria-label="Limit from a parameter"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        <SelectItem value={FIXED_LIMIT}>A fixed number</SelectItem>
                        {numberParamDefs.map((p) => <SelectItem key={p.name} value={p.name}>{p.label || p.name}</SelectItem>)}
                      </SelectContent>
                    </Select>
                  ) : null}
                </div> : null}
              </div>
            ) : null}

            {rules.other.shown ? (
              <div className="grid gap-1">
                <Label className="gap-2 text-sm font-normal">
                  <Switch size="sm" checked={shape.other} disabled={rules.other.disabled} onCheckedChange={(c) => setShape((d) => ({ ...d, other: c === true }))} aria-label="Group the rest as Other" />
                  Group the rest as “Other”
                </Label>
                {rules.other.hint ? <p className="text-xs text-muted-foreground">{rules.other.hint}</p> : null}
              </div>
            ) : null}

            {rules.filters ? <ChartFilters value={shape.filters} onChange={(filters) => setShape((d) => ({ ...d, filters }))} columns={columnChoices} disabled={!fields} segments={fields?.segments ?? []} /> : null}
          </AccordionSection>

          <FoldSection number={4} title="More options" open={moreOpen} onOpenChange={setMoreOpen} panelRef={(n) => { panelRefs.current[4] = n; }}>
            {rules.steps ? <StepsEditor value={shape.steps} onChange={(steps) => setShape((d) => ({ ...d, steps }))} base={baseResult} disabled={!fields} /> : null}

            {rules.layers ? (
              <LayersSection
                layers={layers} onChange={setLayers}
                baseKind={dimension ? fields?.kinds?.[dimension] : undefined} grained={!!(grain && grainOptions.includes(grain))}
                marts={marts.map((m) => m.name)} sqlSources={sqlSources.map((s) => ({ id: s.id, title: s.title }))}
                ownDraw={ownDraw} ownAxis={ownAxis} onOwnChange={(patch) => { if (patch.draw !== undefined) setOwnDraw(patch.draw); if (patch.axis !== undefined) setOwnAxis(patch.axis); }}
                blocked={layersBlocked.length ? `Remove ${layersBlocked.join(", ")} to combine questions.` : isNewSql ? "Save the SQL as a source first to combine it with other questions." : undefined}
                disabled={!fields}
              />
            ) : null}

            {rules.referenceLines ? <ReferenceLinesEditor value={refLines} onChange={setRefLines} numberParams={numberParamDefs} /> : null}
            {isKpi ? <KpiCompareSection draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} metric={pickedMetric ? { label: pickedMetric.label, timeColumn: pickedMetric.timeColumn, grain: pickedMetric.grain } : undefined} /> : null}
            {isGroupedTable ? <TableSection part="format" draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} groupedColumns={shownColumns} /> : null}
            {rules.click ? <ClickSetting value={click} onChange={setClick} boards={boardOptions} /> : null}
            {rules.parameters ? (
              <div>
                <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={() => setParamsOpen(true)}>
                  <SlidersHorizontal className="size-3" aria-hidden /> Parameters
                </Button>
              </div>
            ) : null}
            {paramsOpen ? <ParamDialog open onOpenChange={setParamsOpen} canWrite onChanged={() => { if (source) void loadFields(source); }} /> : null}
          </FoldSection>
        </>
      )}
    </div>
  );
  /** The type search and the grouped type buttons, shared by the open list and the menu of the folded one (phase 2 QA R9). */
  const kindSearch = (
    <>
      <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
      <Input
        value={kindQuery}
        onChange={(e) => setKindQuery(e.target.value)}
        onKeyDown={(e) => {
          // Enter picks the first match, so "sank⏎" is enough.
          const first = kindGroups[0]?.items[0];
          if (e.key === "Enter" && first) { e.preventDefault(); chooseKind(first.value); }
        }}
        placeholder="Search chart types"
        aria-label="Search chart types"
        className="h-8 bg-background pl-8"
      />
    </>
  );
  const kindList = (direction: string) => (
    <>
      {kindGroups.length === 0 ? (
        <p className="px-2 py-1.5 text-xs text-muted-foreground">No chart type matches “{kindQuery.trim()}”.</p>
      ) : null}
      {kindGroups.map((group) => (
        <div key={group.group} className={cn("flex shrink-0 gap-1", direction)}>
          <p className={cn("px-2 pb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground", direction === "md:flex-col" && "hidden md:block")}>
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
                onClick={() => chooseKind(item.value)}
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
    </>
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
            <div className="flex h-full flex-col gap-2">
              <div className="min-h-0 flex-1">
                <TileBody spec={preview.spec} cell={preview.result} dark={resolvedTheme === "dark"} loading={previewBusy} />
              </div>
              {preview.previewRows && preview.previewRows.rows.length ? <PreviewRowsTable data={preview.previewRows} labels={finalResult.labels} open={(preview.spec as { def?: ShapeDefFields }).def?.steps?.length ? true : false} /> : null}
            </div>
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
        <div className={cn("grid h-full min-h-0", listOpen ? "grid-rows-[auto_minmax(0,1fr)] md:grid-cols-[14rem_minmax(0,1fr)] md:grid-rows-1" : "grid-rows-[minmax(0,1fr)]")}>
          {listOpen ? (
          <nav aria-label="Chart type" className="flex min-h-0 flex-col border-b border-border bg-muted/30 md:border-r md:border-b-0">
            <div className="flex items-center gap-1 p-2 md:p-3 md:pb-2">
              <div className="relative min-w-0 flex-1">{kindSearch}</div>
              <Button type="button" size="icon-sm" variant="ghost" className="shrink-0" aria-label="Fold the chart type list" title="Fold the chart type list" onClick={toggleList}>
                <PanelLeftClose aria-hidden />
              </Button>
            </div>
            <div className="flex min-h-0 gap-1 overflow-x-auto px-2 pb-2 md:flex-col md:gap-3 md:overflow-y-auto md:px-3 md:pb-3">
              {kindList("md:flex-col")}
            </div>
          </nav>
          ) : null}

          <div className="flex min-h-0 flex-col">
            <div className="px-5 pt-5 pb-4">
              <DialogHeader>
                <DialogTitle className="flex items-center gap-1.5 text-base">
                  <span className="text-muted-foreground">{isEdit ? "Edit chart" : "New chart"}</span>
                  {/* Phase 2 QA R20: the chevron only when the kind follows it (folded, the kind is on the menu button). */}
                  {listOpen ? (
                    <>
                      <ChevronRight className="size-4 text-muted-foreground" aria-hidden />
                      <span>{KIND_LABELS[kind]}</span>
                    </>
                  ) : null}
                </DialogTitle>
                <DialogDescription className="sr-only">{KIND_DESCRIPTIONS[kind]}. Review live data before adding it to the dashboard.</DialogDescription>
              </DialogHeader>
              {listOpen ? null : (
                // Phase 2 QA R9: with the list folded, the type is changed from here, and the search stays.
                <div className="mt-2 flex items-center gap-1.5">
                  {/*
                    Phase 2 QA R17: a popover of the base-ui family the dialog is made of, so it nests in the
                    dialog's layer stack: an outside press and Escape close only the popover, and focus
                    returns to the trigger. (The hand-made inline panel closed on neither.)
                  */}
                  <PopoverPrimitive.Root open={kindMenu} onOpenChange={setKindMenu}>
                    <PopoverPrimitive.Trigger
                      render={<Button type="button" size="sm" variant="outline" aria-label={`Change chart type, now ${KIND_LABELS[kind]}`} />}
                    >
                      {React.createElement(kindIcon(kind), { className: "size-4", "aria-hidden": true })}
                      {KIND_LABELS[kind]} <ChevronDown className="size-3.5" aria-hidden />
                    </PopoverPrimitive.Trigger>
                    <PopoverPrimitive.Portal>
                      <PopoverPrimitive.Positioner side="bottom" align="start" sideOffset={4} className="z-[60]">
                        <PopoverPrimitive.Popup
                          role="group" aria-label="Chart type"
                          className="flex max-h-80 w-72 flex-col gap-2 rounded-lg border border-border bg-popover p-2 text-popover-foreground shadow-md outline-hidden"
                        >
                          <div className="relative">{kindSearch}</div>
                          <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">{kindList("flex-col")}</div>
                        </PopoverPrimitive.Popup>
                      </PopoverPrimitive.Positioner>
                    </PopoverPrimitive.Portal>
                  </PopoverPrimitive.Root>
                  <Button type="button" size="icon-sm" variant="ghost" aria-label="Show the chart type list" title="Show the chart type list" onClick={toggleList}>
                    <PanelLeftOpen aria-hidden />
                  </Button>
                </div>
              )}
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
                  <div className="h-full min-w-0 overflow-y-auto">{formFields}</div>
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
        {fields?.columns && choice ? (
          <FormulaEditorDialog
            open={formulaOpen}
            onOpenChange={setFormulaOpen}
            source={choice.kind === "mart" ? { mart: choice.name } : { source: choice.id }}
            columns={fields.columns}
            params={paramDefs}
            fields={(fields.calculated ?? []).filter((f) => !isValueGroup(f))}
            existing={editing}
            onSaved={() => { void loadFields(source); }}
            onDeleted={(gone) => {
              void loadFields(source).then((f) => {
                setDimension((v) => (v === gone.name ? "" : keepIfOffered(v, f.dimensions)));
                keepMeasures(f.measures);
                setBreakdown((v) => keepIfOffered(v, f.dimensions));
              });
            }}
          />
        ) : null}
        {fields?.columns && choice ? (
          <>
            <ValueGroupDialog
              open={groupOpen}
              onOpenChange={setGroupOpen}
              source={choice.kind === "mart" ? { mart: choice.name } : { source: choice.id }}
              textColumns={fields.columns.filter((c) => fields.kinds?.[c] === "text")}
              existing={editingGroup}
              onSaved={() => { void loadFields(source); }}
              onDeleted={(gone) => {
                void loadFields(source).then((f) => {
                  setDimension((v) => (v === gone.name ? "" : keepIfOffered(v, f.dimensions)));
                  setBreakdown((v) => keepIfOffered(v, f.dimensions));
                });
              }}
            />
            <HierarchyDialog
              open={hierarchyOpen}
              onOpenChange={setHierarchyOpen}
              source={choice.kind === "mart" ? { mart: choice.name } : { source: choice.id }}
              columns={fields.columns}
              existing={editingHierarchy}
              onSaved={() => { void loadFields(source); }}
              onDeleted={() => { void loadFields(source); }}
            />
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
