"use client";

import { governedPairLabels } from "@/features/catalog/governed-tags"
import { Pill } from "@/components/patterns/status-badge"

import * as React from "react";
import { useTheme } from "next-themes";
import { ArrowLeft, ChartColumn, CircleAlert, Code, Database, Pencil, RefreshCw, SlidersHorizontal, Table2 } from "lucide-react";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { EmptyState } from "@/components/patterns/page-states";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  Select, SelectContent, SelectGroup, SelectLabel, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import type { ChartKind, ChartRenderSpec } from "@/lib/dashboard-specs";
import {
  decodeSourceChoice, encodeSourceChoice, fieldsQuery, sourcePayload, sourceValueFromDef,
  type ChartSourceChoice,
} from "@/lib/chart-source";
import { useAuth } from "@/features/auth/auth-provider";
import { DeprecationNotice } from "@/features/catalog/asset-certification";
import { CertificationBadge } from "@/components/patterns/status-badge";
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
import { FreshnessField } from "./freshness-field";
import { draftOf, draftProblem, ruleOf, type CacheRule, type RuleDraft } from "@/lib/result-cache";
import { KpiCompareSection, PivotSection, TableSection, type SourceColumns } from "./table-settings";
import { draftFromDef, draftPayload, draftProblems, emptyTableDraft, type TableDraft } from "@/lib/table-draft";
import type { TableDefFields } from "@/lib/table-types";
import type { SqlSource, SqlSourceColumn } from "@/services/contracts/dashboards";
import { apiFetch } from "@/services/http";
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
  BREAKDOWN_GRAINS, GRAIN_LABELS, breakdownGrainChoices, columnKindOfType, grainChoices, isGrain, isTruncation, limitAfterBreakdownPick, limitAfterGrainPick,
  limitHasEffect, takesBreakdownGrain, type ColumnKind, type Grain,
} from "@/lib/time-grain";
import { cn } from "@/lib/utils";
import { MapPicker } from "./map-picker";
import { DEFAULT_CHOROPLETH_MAP, DEFAULT_POINT_MAP, mapCredit } from "./echarts-maps";
import { useMapCatalogue } from "./use-map-catalogue";
import { SqlRowsTable } from "@/components/patterns/sql-rows-table";
import { SqlSourcePanel } from "./sql-source-panel";
import { selectedMart, type Mart } from "./mart-marks";
import { JoinsSection, type JoinTableInfo } from "./joins-section";
import { LayersSection } from "./layers-section";
import { layerDraftsFromDef, layerProblems, layersPayload, takesLayers, type LayerDraft } from "@/lib/chart-layers";
import { displayColumnFull, joinedAlias, joinsPayload, pairsFromSuggestions, type JoinDraft, type JoinPairDef } from "@/lib/chart-joins";
import { columnGroup, type ColumnUse } from "@/lib/column-click";
import { ColumnList, type PanelColumn } from "./data-panel";
import { KindPicker, type GalleryPreview } from "./kind-picker";
import type { SourceFacts } from "@/lib/kind-gallery";

export { KIND_GROUPS } from "./chart-kinds";
import type { OwnSource } from "./filters/use-filter-values";
import { ColumnLabel, SettingGroup, SpanPicker, WholeNumberInput, columnItems, focusFirstEmpty } from "./builder-sections";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { asSentence, formatLocaleNumber } from "@/lib/format";
import { missingPlace, missingSection, sectionSummaries, type MissingPlace } from "@/lib/section-summary";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tabs as TabsPrimitive } from "@base-ui/react/tabs";
import { CustomFieldsMenu } from "./custom-fields-menu";
import { builderRules, carryOverOnKindSwitch, limitTakesParam } from "@/lib/builder-rules";
import type { StyleFields } from "@/lib/chart-style";
import { effectivePalette, formatRules, styleFromDef, stylePayload, withCompanyPalette } from "@/lib/style-draft";
import { FormatSection } from "./format-section";
import { FormulaEditorDialog } from "./formula-editor";
import { isValueGroup, type CalcField } from "@/services/contracts/calc-fields";
import type { Hierarchy } from "@/services/contracts/hierarchies";
import { ValueGroupDialog } from "./value-group-dialog";
import { HierarchyDialog } from "./hierarchy-dialog";
import { TileBody } from "./tile-body";
import { useReporting } from "./reporting-context";
import { BandFields, CardFields, GanttFields, GoalFields, HistogramFields } from "./type-b-fields";
import {
  emptyTypeDraft, goalMeasureColumn, takesGoal, typeDraftFromDef, typePayload, typeProblems, urlProblem,
  type TypeDefFields, type TypeDraft,
} from "@/lib/type-b";
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
  /** How long this chart's result may be remembered (BI-4 part B); absent follows the dashboard. */
  cache?: CacheRule;
  /** How a date or timestamp dimension is grouped (BI-9); absent = as stored. */
  grain?: string;
} & TableDefFields // raw tables, pivots and KPI comparisons (BI-16 part A)
  & TypeDefFields // goal, bands, histogram ranges, the address of an image or web card (BI-16 part B)
  & StyleFields // number and date formats, colours, axes, labels, notes, tooltip, conditional colours (BI-17 part A)
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
 * Value → label maps for the base-ui selects below. Without `items`, a
 * base-ui `Select.Value` renders the raw value (`hbar`, `b_54e05896`,
 * `desc`, `__none__`) in the closed trigger even though the open list shows
 * labels.
 */
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
const NO_LABEL_ITEMS: Record<string, string> = { [NO_BREAKDOWN]: "— no label —" };

/** One focus ring and easing for the title and description edited in place (owner taste: 150–250 ms, eased). */
const INLINE_EDIT = "outline-none transition-[background-color,color,box-shadow] duration-200 ease-[cubic-bezier(0.2,0,0,1)] focus-visible:ring-3 focus-visible:ring-ring/50";

/** The canvas's empty, loading and error states, centred where the chart will be. */
function StateBox({ children }: { children: React.ReactNode }) {
  return <div className="grid h-full place-content-center justify-items-center gap-3 p-6 text-center">{children}</div>;
}

/**
 * BI-58 review round 2, SHOULD-FIX 7 (owner; DESIGN.md §3, "Panel yang bersebelahan sejajar"): the
 * header rows of zones that sit side by side (the canvas's toolbar, the inspector's tabs) share one
 * height, 44 px, so their dividers are one line. Below the two-column width the toolbar may wrap,
 * since nothing sits beside it there. (The tab row repeats `h-11` under the tabs' group variant,
 * which the base TabsList sets its own height with.)
 */
const ZONE_HEADER = "min-h-11 shrink-0 @3xl:h-11";

/** The inspector's tabs (`BI-58` T4). */
type InspectorTab = "data" | "sort" | "style";

/** Marks a field the chart cannot be drawn without. */
function Req() {
  return <span aria-hidden className="text-destructive">*</span>;
}
/** Measure labels that vary by kind (X/Y/size, bar/line, etc.). */
const MEASURE_LABELS: Partial<Record<ChartKind, string[]>> = {
  scatter: ["X metric", "Y metric"],
  bubble: ["X metric", "Y metric", "Size metric"],
  combo: ["Bar metric", "Line metric"],
  stacked: ["Value", "2nd value"],
  pointmap: ["Value (size and colour)"],
  progress: ["Value"],
  bullet: ["Value"],
  geoheat: ["Value (weight)"],
};

/**
 * The measure rows of a saved chart. A progress bar or a bullet graph measured against a second
 * value was saved with that value as its second measure; the form keeps it in the goal
 * (`typeDraft`), so the rows keep only the value (BI-16 part B).
 */
function measureRowsOf(def: ChartDef): MeasureRow[] {
  const rows = rowsFromEntries(def.measures);
  return def.kind && takesGoal(def.kind) && def.goal && typeof def.goal === "object" ? rows.slice(0, 1) : rows;
}

/**
 * Chart builder — the MANUAL path (Tableau-style), as a page (`WEB-7` D6).
 *
 * It used to be a dialog opened over the dashboard and over a chat draft.
 * It is now the body of `/dashboards/[id]/charts/new` and
 * `.../[chartId]/edit`, inside `FormPage`: the chart-type sidebar, the form
 * and the live preview in the page's column, with Cancel and Save in its
 * footer. The small contextual dialogs it opens (parameters, formulas, value
 * groups, hierarchies) stay dialogs, because they are about one field.
 *
 * Saving writes the same artifact as the chat path (console.bi_chart) and
 * hands the payload to `onSaved`, which the page answers by going back to
 * the dashboard; the dashboard reloads, which is what used to be done by
 * re-fetching in place.
 */
export function ChartBuilder({
  onSaved, onCancel, board = "default", boards = [], initial, editId, paramValues,
}: {
  /** Receives the definition that was saved; the page leaves for the dashboard with it. */
  onSaved: (saved: ChartDef) => void;
  /** Leaving by the back arrow or Esc (after "Discard changes?" when something changed); the page goes back to where it came from. */
  onCancel: () => void;
  board?: string;
  boards?: BoardOpt[];
  initial?: ChartDef;
  editId?: string;
  /** The parameter values the dashboard shows now; the preview is drawn with them (BI-11). */
  paramValues?: Record<string, ParamValue>;
}) {

  const [marts, setMarts] = React.useState<Mart[]>([]);
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
  // `BI-58` T3: "Try again" on a failed preview asks once more with the same inputs.
  const [retry, setRetry] = React.useState(0);
  // The rows under the chart: null follows the chart (open with steps), a boolean is the person's choice.
  const [showData, setShowData] = React.useState<boolean | null>(null);
  const { resolvedTheme } = useTheme();

  const [title, setTitle] = React.useState("");
  // `BI-58` T3: the chart's description, edited under its title; saved as `subtitle`, which the
  // dashboard shows under a tile's title (and the assistant already writes).
  const [subtitle, setSubtitle] = React.useState("");
  const [describing, setDescribing] = React.useState(false);
  // Encoded data-source choice (`mart:<name>` / `sql:<id>`, lib/chart-source).
  const [source, setSource] = React.useState("");
  const choice = decodeSourceChoice(source);
  const chosenMart = selectedMart(marts, choice);
  const [kind, setKind] = React.useState<ChartKind>("hbar");
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
  // BI-4 part B: how long this chart's result may be remembered.
  const [cacheDraft, setCacheDraft] = React.useState<RuleDraft>(draftOf(null));
  // BI-17 part A: the chart's formats, colours, axes, labels, notes, tooltip and conditional colours (section 5).
  const [style, setStyle] = React.useState<StyleFields>({});
  // BI-13: the chart's other questions, and how its own series is drawn when it has them ("" = the kind's default).
  const [layers, setLayers] = React.useState<LayerDraft[]>([]);
  const [ownDraw, setOwnDraw] = React.useState("");
  const [ownAxis, setOwnAxis] = React.useState("");
  const [paramsOpen, setParamsOpen] = React.useState(false);
  // `BI-58` T4: the inspector's tab; every chart opens on Data.
  const [tab, setTab] = React.useState<InspectorTab>("data");
  // Where a missing field is: 1 the data panel, 2 the Data tab, 3 the Sort & filter tab.
  const panelRefs = React.useRef<Partial<Record<1 | 2 | 3, HTMLElement | null>>>({});
  const [focusAsk, setFocusAsk] = React.useState<{ section: 1 | 2 | 3; n: number } | null>(null);
  // `BI-58` T5: below 1280 px of builder width the data panel folds into "Pick data", which opens it as a sheet.
  const [dataSheet, setDataSheet] = React.useState(false);
  const sheetPanel = React.useRef<HTMLDivElement | null>(null);
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
  // BI-16 part B: the deployment's content sites, a progress bar's goal, a bullet's bands, a histogram's ranges, a card's address.
  const { contentSites, companyPalette } = useReporting();
  const sites = contentSites ?? [];
  const palette = companyPalette ?? [];
  const [typeDraft, setTypeDraft] = React.useState<TypeDraft>(emptyTypeDraft);
  const changeType = (patch: Partial<TypeDraft>) => setTypeDraft((d) => ({ ...d, ...patch }));
  const isText = kind === "text";
  const isImage = kind === "image";
  const isWeb = kind === "web";
  const isCard = isImage || isWeb;
  /** A tile with no data: a note, an image, a web page. */
  const isStaticTile = isText || isCard;
  const isKpi = kind === "kpi";
  const isGauge = kind === "gauge";
  const isProgress = kind === "progress";
  const isHistogram = kind === "histogram";
  const isBullet = kind === "bullet";
  // BI-16 part C1 (plan D1): the task is the dimension, the group the breakdown, the start, end and progress columns are the type draft's.
  const isGantt = kind === "gantt";
  // BI-16 C2b: the bundled maps and the uploaded ones, by name.
  const mapCatalogue = useMapCatalogue();
  const mapLabels = React.useMemo<Record<string, string>>(() => Object.fromEntries(mapCatalogue.map((m) => [m.id, m.label])), [mapCatalogue]);
  const isSingle = isKpi || isGauge || isProgress;         // no dimension (single number)
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
  // A histogram counts the rows of its column: it takes no measure either.
  const noMeasures = ownColumns || isHistogram || isGantt;
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
  // Fix the number of measure rows to what the kind takes. BI-17 B QA fix F2: it also runs when the
  // rows themselves change length, because picking the data source (and resetting the form) puts
  // back one blank row, which left a scatter with an X metric and no Y metric.
  const measureRowCount = measureRows.length;
  React.useEffect(() => {
    setMeasureRows((rows) => {
      if (rows.length === Math.min(Math.max(rows.length, limits.min), limits.max || 1) && rows.length >= limits.min) return rows;
      const next = rows.slice(0, Math.max(limits.max, 1));
      while (next.length < limits.min) next.push({ column: "", aggregate: "" });
      return next;
    });
  }, [limits.min, limits.max, measureRowCount]);
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
    // `BI-46` review round 1, R7: beside a breakdown the limit counts breakdown
    // values, so the untouched 20 stays 20.
    setLimit((l) => limitAfterGrainPick(next, l, limitTouched, capsBreakdown));
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
  const canBreakdown = kind === "bar" || kind === "hbar" || kind === "line" || kind === "area" || needsBreakdown || isGantt;
  // `BI-46` (QA `LH-BI-6-01`): a date or timestamp breakdown on a bar, hbar,
  // line or area chart can be grouped by a period, and on those charts the
  // limit keeps the N breakdown values with the largest total.
  const breakdownKind = breakdown ? fields?.kinds?.[breakdown] : undefined;
  const breakdownGrainOptions = React.useMemo(() => breakdownGrainChoices(kind, breakdownKind), [kind, breakdownKind]);
  // `BI-46` review round 1, R4: a chart whose measure is a table calculation
  // gets no breakdown cap on the server — its windows run over the whole
  // grouped result — so the limit keeps the label such a chart always had, and
  // does not promise a cut that does not happen.
  const measureIsTableCalc = (fields?.calculated ?? []).some((f) => f.level === "table" && f.name === measure);
  const capsBreakdown = !!breakdown && takesBreakdownGrain(kind) && !measureIsTableCalc;
  // BI-10: a number column can be the dimension or the breakdown when it is put in bins; a table does
  // that per grouping, a pivot on its first number row or column field.
  const onSqlSource = choice?.kind === "sql" || source === NEW_SQL_CHOICE;
  // Phase 2 QA R5: what applies to this kind and to what is already set, from one function (lib/builder-rules).
  const rules = builderRules({
    kind, rowsMode, grained: !!(grain && grainOptions.includes(grain)), breakdown: canBreakdown && !!breakdown,
    other: shape.other, steps: shape.steps.length, layers: layers.length,
    multiGrouped: isGroupedTable && shape.groupings.length > 1,
    limitHasEffect: limitHasEffect(grain, isCalendar ? "none" : order, capsBreakdown),
    hasSource: !!choice || source === NEW_SQL_CHOICE,
  });
  const binKind = takesBin(kind, rowsMode) && !isGroupedTable && !isPivot && rules.bins;
  const numberColumns = React.useMemo(
    () => (fields?.measures ?? []).filter((m) => !wholeMeasure.has(m) && fields?.kinds?.[m] === "number"),
    [fields, wholeMeasure],
  );
  // QA LH-BI-13-01: bins are off beside more questions (builder-rules), but a number column (a year) is still a
  // real column the server takes as the dimension, so it stays offered, unbinned (`pickDimension` clears the bin).
  const dimensionChoices = isHistogram ? numberColumns : binKind || (layers.length > 0 && takesLayers(kind) && !rowsMode) ? [...(fields?.dimensions ?? []), ...numberColumns] : fields?.dimensions ?? [];
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
    const pickedKind = column ? fields?.kinds?.[column] : undefined;
    // `BI-46` review round 1, R7: from here the limit counts breakdown values,
    // so an untouched grained maximum (1000) that a grain pick left behind goes
    // back to the default 20. A limit the person set is kept.
    const capsAfterPick = !!column && takesBreakdownGrain(kind) && !(fields?.calculated ?? []).some((f) => f.level === "table" && f.name === measure);
    if (capsAfterPick) setLimit((l) => limitAfterBreakdownPick(l, limitTouched));
    setShape((d) => ({
      ...d,
      breakdownBin: binKind && column && isNumber(column) ? d.breakdownBin ?? startingBin : null,
      // `BI-46`: a period only cuts a date or timestamp; another column drops it.
      breakdownGrain: pickedKind === "date" || pickedKind === "datetime" ? d.breakdownGrain : "",
    }));
  }
  const pivotNumberField = isPivot ? [...tableDraft.pivotRows, ...tableDraft.pivotColumns].find((c) => isNumber(c)) : undefined;
  // `BI-58` T6: the labels say "X axis", "Split" and "Value", the words of the data panel's chips;
  // the code keeps dimension, breakdown and measure, which the assistant and the docs use.
  const mLabels = isSingle ? ["Value"] : MEASURE_LABELS[kind] ?? ["Value"];
  const measureTitle = (i: number) => (limits.max > 1 && limits.max === limits.min ? mLabels[i] ?? `Value ${i + 1}` : i === 0 ? mLabels[0] : `Value ${i + 1}`);
  const dimensionLabel = isChoropleth ? "Region column" : isPoints ? "Label" : isHistogram ? "Number column" : isGantt ? "Task column" : isBullet ? "Category" : "X axis";
  const breakdownLabel = isGantt ? "Group" : kind === "sankey" ? "Flows to" : kind === "sunburst" ? "Outer ring" : isHeatmap ? "2nd dimension (Y)" : "Split into series";
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
  const sqlActive = !isStaticTile && draft.mode !== "none";
  const heldPreview = !isStaticTile && draft.holdsChartPreview;
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
    for (const mart of joinMarts.split(",").filter(Boolean)) {
      if (!joinInfo[mart]) void learnTable(mart, baseMart);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `joinInfo` and `learnTable` only cache what `joinMarts` names
  }, [joinMarts]);
  // The pickers list the flat relation's columns, so they follow the joins that are complete.
  const joinsKey = JSON.stringify(joinsPayload(shape.joins).joins ?? []);
  React.useEffect(() => {
    if (!baseMart || joinsSeen.current === joinsKey) return;
    joinsSeen.current = joinsKey;
    void loadFields(source).then((f) => {
      setDimension((v) => keepIfOffered(v, f.dimensions));
      keepMeasures(f.measures);
      setBreakdown((v) => keepIfOffered(v, f.dimensions));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reload only when the complete joins change
  }, [joinsKey]);
  // BI-13: questions can be added to a bar, line, area or combo chart; they are matched on its
  // dimension, so a breakdown, "Other", steps or bins on the chart are refused with the questions.
  const layersApply = takesLayers(kind) && !rowsMode && layers.length > 0;
  const layersBlocked = [
    ...(canBreakdown && breakdown ? ["the split"] : []),
    ...(shape.other ? ["the 'Other' group"] : []),
    ...(shape.steps.length ? ["steps"] : []),
    ...(shape.bin || shape.breakdownBin ? ["bins"] : []),
  ];
  const noFieldsHint = isNewSql ? "run the SQL first" : "pick a source first";
  // Required fields still empty: listed in the preview before the user
  // hits Create, and the same list is the save-time error.
  const missing: string[] = isStaticTile
    ? [...(title.trim() ? [] : ["Title"]), ...(isText ? (text.trim() ? [] : ["Content"]) : typeProblems(kind, typeDraft))]
    : [
        ...(choice || isNewSql ? [] : ["Data source"]),
        ...(!isSingle && !isPoints && !isBullet && !ownColumns && !isGroupedTable && !dimension ? [dimensionLabel] : []),
        ...(isPoints && !lat ? ["Latitude"] : []),
        ...(isPoints && !lon ? ["Longitude"] : []),
        ...(noMeasures ? [] : measureRows.slice(0, limits.min).flatMap((r, i) => (r.column ? [] : [measureTitle(i)]))),
        ...typeProblems(kind, typeDraft),
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
  const breakdownLabels: Record<string, string> = { [NO_BREAKDOWN]: "— no split —" };

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

  // On mount: load the marts, and prefill from initial — EDIT, or a draft
  // (e.g. from Copilot) whose kind is already chosen. The page mounts this
  // only once it holds the board, the chart and any draft, so `initial` is
  // settled by then (`WEB-7` D6).
  React.useEffect(() => {
    draft.cancel();
    void apiFetch("/api/dashboard/fields").then((r) => r.json()).then((j) => setMarts(j.marts ?? [])).catch(() => setMarts([]));
    // A viewer without the list (or with the API down) still gets marts.
    void dashboardService.listSqlSources().then(setSqlSources).catch(() => setSqlSources([]));
    if (initial) {
      setTitle(initial.title ?? "");
      setSubtitle(initial.subtitle ?? "");
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
      setMeasureRows(measureRowsOf(initial));
      setLimitParamName(limitParam(initial.limit) ?? "");
      setLimit(typeof initial.limit === "number" ? initial.limit : 20);
      setRefLines(refLinesFromDef(initial.referenceLines));
      setCacheDraft(draftOf(initial.cache));
      setTypeDraft(typeDraftFromDef(initial));
      setStyle(styleFromDef(initial));
      setLayers(layerDraftsFromDef(initial.layers));
      setOwnDraw(initial.draw ?? "");
      setOwnAxis(initial.axis ?? "");
      setLimitTouched(true);
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
          setMeasureRows(measureRowsOf(initial));
        });
      }
    } else {
      setTargetBoard(board);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // User changes the data source → reset the column selections and reload.
  function onSourceChange(value: string) {
    draft.cancel();
    setSource(value); setDimension(""); setMeasureRows([{ column: "", aggregate: "" }]); setBreakdown("");
    // A goal that is a second value names a column of the old source.
    setTypeDraft((d) => ({ ...d, goalMeasure: "" }));
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
  // BI-17 part A: which groups of section 5 this chart takes, and the names its formats, second axis,
  // tooltip and conditional colours can refer to (a table's columns, a pivot's value columns, else the values).
  const fmtRules = formatRules({ kind, breakdown: canBreakdown && !!breakdown, measures: resultNames.length });
  const formatValueNames = kind === "table" || kind === "pivot" ? (kind === "pivot" ? tableDraft.pivotValues.map((v) => v.column).filter(Boolean) : shownColumns) : resultNames;

  const shapeContext = { kind, rowsMode, grained: !!(grain && grainOptions.includes(grain)), breakdown: !!(canBreakdown && breakdown) };
  // BI-2: saved metrics of the source are offered where an aggregate field is (not maps, box plots, pivots or raw rows).
  const takesMetrics = !noMeasures && !isPoints && kind !== "boxplot" && kind !== "text";
  const measureColumns = [
    ...(fields?.measures ?? []).filter((m) => !isPoints || (m !== lat && m !== lon)),
    ...(takesMetrics ? (fields?.metrics ?? []).map((m) => metricColumn(m.name)) : []),
  ];
  // Columns a filter or a grouping may name: the source's, with its per-row calculated fields.
  // `BI-47` (plan D11): a dashboard parameter slot (`{ukuran}`) is a dimension,
  // a breakdown and a measure, never a filter or grouping column — the server
  // has no column to list its values from.
  const columnChoices = React.useMemo<ColumnChoice[]>(() => {
    const names = [...new Set([...(fields?.dimensions ?? []), ...(fields?.measures ?? [])])]
      .filter((n) => !wholeMeasure.has(n) && !slotParam(n));
    return names.map((name) => ({ name, kind: fields?.kinds?.[name] ?? "text" }));
  }, [fields, wholeMeasure]);
  // `BI-47` (QA `LH-BI-6-01`): a filter's value list reads the chart's OWN
  // relation, not every mart that has the column. SQL that is new or edited and
  // not saved has no list (the saved text would show the old values), and a
  // chart with no source chosen has none either.
  const filterSource: OwnSource = heldPreview || !choice
    ? { unsaved: true }
    : choice.kind === "sql"
      ? { sqlSource: choice.id }
      : { mart: choice.name, joins: joinsPayload(shape.joins).joins };
  // `BI-58` T2: the data panel's columns, each with its use now and the uses the chart's own
  // pickers offer it (a column a picker would not list is not offered there either).
  const takesX = !isStaticTile && !isSingle && !ownColumns && !isGroupedTable;
  const xChoices = isPoints || isBullet ? fields?.dimensions ?? [] : dimensionChoices;
  const panelColumns: PanelColumn[] = fields ? [...new Set([
    ...fields.dimensions, ...fields.measures,
    ...(takesMetrics ? (fields.metrics ?? []).map((m) => metricColumn(m.name)) : []),
  ])].filter((name) => !slotParam(name)).map((name) => ({
    name,
    group: isMetricColumn(name) ? "number" : columnGroup(fields.kinds?.[name] ?? (fields.measures.includes(name) ? "number" : "text")),
    table: joinedAlias(name),
    role: takesX && name === dimension ? (isGantt ? "Task" : "X axis")
      : !noMeasures && measureRows.some((r) => r.column === name) ? "Value"
      : canBreakdown && name === breakdown ? (isGantt ? "Group" : "Split")
      : null,
    offers: {
      x: takesX && xChoices.includes(name),
      value: !noMeasures && measureColumns.includes(name),
      split: canBreakdown && !rules.breakdownLocked && breakdownChoices.includes(name),
      filter: rules.filters && columnChoices.some((c) => c.name === name),
    },
  })) : [];
  // A filter asked for from the panel opens in the filters, which the sort-and-filter section shows.
  const [filterAsk, setFilterAsk] = React.useState<{ column: string; n: number } | null>(null);
  function applyColumnUse(name: string, use: ColumnUse) {
    if (use === "x") {
      // The X axis and the split are two different columns.
      if (name === breakdown) pickBreakdown("");
      if (isPoints || isBullet) setDimension(name); else pickDimension(name);
    } else if (use === "value") {
      // The first value, with the chart's default aggregate.
      setMeasureRows((rows) => [{ column: name, aggregate: "" }, ...rows.slice(1)]);
    } else if (use === "split") {
      pickBreakdown(name);
    } else {
      setTab("sort");
      setFilterAsk({ column: name, n: Date.now() });
    }
  }

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
  // `BI-58` T6 (QA 2026-11-14): the server refuses a parameter as the limit of a chart with steps or
  // of a kind that does not list groups along one dimension; the choice is hidden then, with why.
  const limitParamOk = limitTakesParam(kind, { steps: stepsOn, rowsMode });
  const limitParamUsed = limitParamOk ? limitParamName : "";
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
    mapLabel: isMap ? mapLabels[effectiveMap] ?? "" : "", lat, lon,
    sort: rules.sort === "choice" ? sortLabels[shape.sortChoice] ?? "" : rules.sort === "by-date" ? "by date" : rules.sort === "by-category" ? "by category" : "",
    limit: rules.limit ? (limitParamUsed ? "param" : Math.min(limit, maxLimit)) : null,
    filters: rules.filters ? shape.filters.length : 0, other: shape.other && rules.other.shown,
  });
  /** Shows the place that holds a missing field (the data panel or a tab) and moves focus to that field. */
  function revealMissing(place: MissingPlace) {
    if (place !== "source") setTab(place);
    // The data panel is folded away below 1280 px of builder width; then its sheet opens instead.
    else if (panelRefs.current[1]?.checkVisibility?.() === false) setDataSheet(true);
    setFocusAsk({ section: place === "source" ? 1 : place === "data" ? 2 : 3, n: Date.now() });
  }
  React.useEffect(() => {
    if (!focusAsk) return;
    if (!(focusAsk.section === 1 && dataSheet)) { focusFirstEmpty(panelRefs.current[focusAsk.section] ?? null); return; }
    // The sheet's content mounts in a portal on its first frame; the field is looked for after it.
    const timer = window.setTimeout(() => focusFirstEmpty(sheetPanel.current), 80);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `dataSheet` is read as it was when the focus was asked for
  }, [focusAsk]);
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
    } else if (isCard) {
      // BI-16 part B: an image or a web page has no data; the server checks the address against the allowed sites.
      const wrong = urlProblem(typeDraft.url, sites);
      if (wrong && !forPreview) throw new Error(wrong);
      payload = { title: payloadTitle, kind, span, board: targetBoard, ...typePayload(kind, typeDraft) };
    } else if (isGantt) {
      // BI-16 part C1 (plan D1): no measure, aggregate, order or limit; the statement is the server's `gantt_sql`.
      payload = {
        title: payloadTitle, ...sourcePayload(withSource), kind, dimension, span, board: targetBoard,
        breakdown: breakdown || undefined,
        ...typePayload(kind, typeDraft),
        ...shapeOut(),
      };
    } else if (isHistogram) {
      payload = {
        title: payloadTitle, ...sourcePayload(withSource), kind, dimension, span, board: targetBoard,
        ...typePayload(kind, typeDraft),
        ...shapeOut(),
      };
    } else if (isSingle) {
      const goalColumn = goalMeasureColumn(kind, typeDraft);
      // QA LH-BI-2-01: a reusable metric must go out as `{ metric }` like on the other paths; the raw row
      // column `"metric:…"` is refused by the server (400 "invalid or missing measure column"). A plain
      // column, and an empty row (kept as today so the preview says what is missing), still go as the name.
      const [firstMeasure] = entriesFromRows(measureRows.slice(0, 1), limits.own);
      payload = {
        title: payloadTitle, kind, ...sourcePayload(withSource), measures: [firstMeasure ?? measure, ...(goalColumn ? [goalColumn] : [])], aggregate, span, board: targetBoard,
        caption: (isKpi || isProgress) && caption ? caption : undefined,
        target: isGauge && Number(target) > 0 ? Number(target) : undefined,
        ...draftPayload(kind, tableDraft),
        ...typePayload(kind, typeDraft),
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
      const goalColumn = goalMeasureColumn(kind, typeDraft);
      const measures = [...entriesFromRows(measureRows, limits.own), ...(goalColumn ? [goalColumn] : [])];
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
        // A bullet graph without a category is one row and has no limit.
        limit: isPoints || (isBullet && !dimension) ? undefined : limitParamUsed ? { param: limitParamUsed } : Math.min(limit, maxLimit),
        referenceLines: takesLines && refLinesPayload(refLines).length ? refLinesPayload(refLines) : undefined,
        // BI-4 part B: left out when it follows the dashboard, which clears a rule the chart had.
        cache: forPreview || cacheDraft.mode === "default" || draftProblem(cacheDraft) ? undefined : ruleOf(cacheDraft),
        layers: layersApply ? layersPayload(layers) : undefined,
        draw: layersApply && ownDraw ? ownDraw : undefined,
        axis: layersApply && ownAxis ? ownAxis : undefined,
        // Only a grain the column and kind allow is sent (BI-9); the server checks again.
        grain: grain && grainOptions.includes(grain) ? grain : undefined,
        ...draftPayload(kind, tableDraft, shownColumns),
        ...typePayload(kind, typeDraft),
        ...shapeOut(),
      };
    }
    // BI-17 part A: only the groups this kind takes, with empty ones left out.
    // A new chart in a deployment with a company palette draws with it
    // (decision 6, `PR #199 review M SHOULD-FIX BI-17 4`).
    if (!isStaticTile) {
      const styled = stylePayload(style, fmtRules);
      Object.assign(payload, fmtRules.colors ? withCompanyPalette(styled, palette, isEdit) : styled);
    }
    // A chart without marks to click (text, KPI, gauge, table, density map)
    // has no click setting; one left over from another kind is dropped.
    const clickable = kindHasClickValue(kind);
    const problem = clickable ? clickProblem(click) : null;
    if (problem && !forPreview) throw new Error(problem);
    const freshProblem = draftProblem(cacheDraft);
    if (freshProblem && !forPreview && !isStaticTile) throw new Error(freshProblem);
    if (clickable && click && !problem) payload.click = click;
    if (isEdit) payload.id = editId;
    // Sent only when there is one, so a chart without a description is saved as it always was.
    if (!isStaticTile && subtitle.trim()) payload.subtitle = subtitle;
    // BI-11: the preview is drawn with the values the dashboard shows; they are not part of the chart.
    if (forPreview && paramValues && Object.keys(paramValues).length) payload.params = paramValues;
    return payload;
  }

  const previewInputsKey = previewKey({
    title, source, kind, dimension, measure, measure2, measure3, breakdown, mapId, lat, lon,
    aggregate, span, caption, target, text, order: shape.sortChoice, limit, targetBoard, grain,
    // BI-16 part A: every table, pivot and comparison setting reaches the SQL or the render spec.
    tables: JSON.stringify(draftPayload(kind, tableDraft, shownColumns)),
    // BI-16 part B: the goal, bands, ranges and address reach the statement or the drawing.
    types: JSON.stringify(typeDraft),
    // BI-6: each measure's own aggregate, the chart's own filters, the sort, "Other" and the groupings.
    shape: JSON.stringify({ measures: measureRows, shape }),
    // BI-8: a changed formula changes the rows a chart over the field returns.
    calc: JSON.stringify((fields?.calculated ?? []).map((f) => [f.name, f.formula])),
    // BI-11: the values the preview is drawn with, the limit's parameter and the reference lines.
    params: JSON.stringify({ values: paramValues ?? null, limit: limitParamUsed, lines: refLines }),
    // BI-17: formats and colours change how the preview is drawn, and the rules of a kind change what is sent.
    style: JSON.stringify(stylePayload(style, fmtRules)),
    // BI-13: each question, and how the chart's own series is drawn, reaches the statements and the drawing.
    layers: JSON.stringify(layersApply ? { layers, ownDraw, ownAxis } : null),
  });
  React.useEffect(() => {
    if (isText) {
      setPreview(text.trim() ? {
        spec: { id: "preview", title: title || "Chart preview", kind: "text", mart: "", x: "", y: "", source: "ui", text },
        result: { columns: [], rows: [] },
      } : null);
      setPreviewError(null);
      return;
    }
    // BI-16 part B: an image or a web page is drawn from what is typed, with nothing to ask the server.
    if (isCard) {
      const wrong = urlProblem(typeDraft.url, sites);
      setPreview(!wrong && typeDraft.url.trim() ? {
        spec: { id: "preview", title: title || "Chart preview", kind, mart: "", x: "", y: "", source: "ui", def: typePayload(kind, typeDraft) },
        result: { columns: [], rows: [] },
      } : null);
      setPreviewError(wrong);
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
  }, [previewInputsKey, heldPreview, previewSql, retry]);

  async function save() {
    setError(null);
    if (sqlActive && draft.blocker) { draft.markSaveAttempted(); setError(draft.blocker); return; }
    // Phase 2 QA R10: a required field that is missing opens its section and takes focus.
    if (missing.length) {
      // The first missing field in the order of the old sections.
      const first = missing.reduce((a, b) => (missingSection(b) < missingSection(a) ? b : a));
      revealMissing(missingPlace(first));
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
        // `WEB-7` D6: the write goes through the dashboard service, so no
        // feature file calls `/api/dashboard/specs` by hand.
        await dashboardService.saveChart(payload, isEdit);
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

  // `BI-46` (plan D20): the period a date or timestamp breakdown is grouped by.
  const breakdownGrainPicker = breakdownGrainOptions.length > 0 ? (
    <div className="grid gap-1.5">
      <Label>Group the split by</Label>
      <Select
        value={shape.breakdownGrain || NO_GRAIN} items={GRAIN_ITEMS}
        onValueChange={(v) => setShape((d) => ({
          ...d,
          breakdownGrain: v && v !== NO_GRAIN && (BREAKDOWN_GRAINS as readonly string[]).includes(v) ? (v as Grain) : "",
        }))}
      >
        <SelectTrigger className="w-full" aria-label="Group the split by"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value={NO_GRAIN}>No grouping</SelectItem>
          {breakdownGrainOptions.map((g) => <SelectItem key={g} value={g}>{GRAIN_LABELS[g]}</SelectItem>)}
        </SelectContent>
      </Select>
    </div>
  ) : null;

  // Phase 2 QA R15: which conditional controls the dimension and the breakdown bring (they sit in a row below the fields).
  const binDimension = !isGroupedTable && !!binKind && isNumber(dimension);
  const dimensionExtras = !isGroupedTable && (!!grainPicker || binDimension);
  const breakdownBinPicker = canBreakdown && !!binKind && !!breakdown && isNumber(breakdown);
  const breakdownExtras = !!breakdownGrainPicker || breakdownBinPicker;

  /**
   * `BI-58` T1: the source, its joins, its SQL and its custom fields sit in the data
   * panel at the left of the page (they were section 1 of the form).
   */
  const sourcePanel = (
            <div className="grid gap-1.5">
            <Label>Data source <Req /></Label>
            <Select value={source} items={sourceLabels} onValueChange={(v) => onSourceChange(v ?? "")}>
              <SelectTrigger className="w-full"><SelectValue placeholder={canWriteSql ? "pick a mart or SQL source, or write SQL" : "pick a mart or SQL source"} /></SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>Gold marts</SelectLabel>
                  {marts.map((m) => {
                    const governed = governedPairLabels(m.governedTags)
                    const tags = m.tags ?? []
                    return (
                      <SelectItem key={m.name} value={encodeSourceChoice({ kind: "mart", name: m.name })}>
                        <span className="flex items-center gap-1.5 flex-wrap">
                          <span>{m.name} · {formatLocaleNumber(m.rows)}</span>
                          {/* DATA-12 part 3 & PR #106 review R1 BLOCKER 2: mark and tags */}
                          <CertificationBadge certification={m.certification} />
                          {governed.map((pair) => (
                            <Pill key={`governed:${pair}`} tone="info" className="text-[10px]">
                              {pair}
                            </Pill>
                          ))}
                          {tags.map((tag) => (
                            <Pill key={tag} tone="neutral" className="text-[10px]">
                              {tag}
                            </Pill>
                          ))}
                        </span>
                      </SelectItem>
                    )
                  })}
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
            {/* QA LH-DATA-12-17 F1 (DATA-54): the trigger shows the mart's name
                only, so the mark of the list row was lost once a mart was
                chosen. A deprecated mart has the notice below. */}
            {chosenMart?.certification === "certified" ? (
              <p
                data-testid="chosen-mart-mark"
                className="flex items-center gap-1.5 text-xs text-muted-foreground"
              >
                <CertificationBadge certification="certified" />
                This mart is certified.
              </p>
            ) : null}
            {/* DATA-12 part 3: a deprecated mart still works; it warns, with the
                note and a link to its replacement, and never blocks. */}
            {chosenMart ? <DeprecationNotice asset={chosenMart} /> : null}
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
  );

  // Phase 2 QA R14, the cause of the sideways scroll: a grid with no column template has one `auto`
  // track, whose minimum is the widest unbreakable content in it (a trigger's `nowrap` value), so
  // one long column name widened the whole form. Every grid of the form without columns of its own
  // gets a `minmax(0, 1fr)` track, which the container, not the content, sizes.
  // The data panel's content: inline at the left from 1280 px of builder width, else in the "Pick data" sheet.
  const dataPanel = (
    <>
      {sourcePanel}
      <ColumnList
        columns={panelColumns} fieldLabel={fieldLabel} textOf={titleText}
        xEmpty={!dimension} valueEmpty={!measureRows[0]?.column} onUse={applyColumnUse}
      />
      {chosenMart ? <p className="px-2 text-xs text-muted-foreground tabular-nums">{formatLocaleNumber(chosenMart.rows)} rows in {chosenMart.name}</p> : null}
    </>
  );
  const FORM_GRID = "grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-5 px-5 py-4 [&_.grid:not([class*=grid-cols])]:grid-cols-[minmax(0,1fr)]";
  /** A group of settings under the main ones of a tab, set apart by a rule. */
  const group = (children: React.ReactNode) => <div className="grid grid-cols-[minmax(0,1fr)] gap-4 border-t border-border/60 pt-5">{children}</div>;

  // `BI-58` T4: every control of the old five sections, in three tabs. Data: what the chart shows
  // and where from (old sections 2 and 4 but steps); Sort & filter: old section 3 and the steps;
  // Style: the Format section as it is. A kind's own panels show only for it, as before.
  const inspector = isStaticTile ? (
    <div className={FORM_GRID}>
      {/* BI-58 review round 1, SHOULD-FIX 5 (owner): a note, an image or a web page keeps its width here. */}
      <SettingGroup title="Size on dashboard">
        <SpanPicker value={span} onChange={setSpan} />
      </SettingGroup>
      {isText ? (
        <div className="grid gap-1.5">
          <Label>Content (markdown) <Req /></Label>
          <Textarea rows={8} value={text} onChange={(e) => setText(e.target.value)} placeholder="Title **bold**, - bullets, or | table | GFM |." />
        </div>
      ) : (
        <CardFields kind={kind === "web" ? "web" : "image"} draft={typeDraft} onChange={changeType} sites={sites} />
      )}
    </div>
  ) : (
    <Tabs value={tab} onValueChange={(v) => setTab(v as InspectorTab)} className="flex min-h-0 flex-1 flex-col gap-0">
      {/* BI-58 review round 2, SHOULD-FIX 6 (owner): the tab row spans the inspector. Three equal tabs
          with centred labels, a 44 px row with its divider across the panel, and one underline the
          width of the active tab that slides to it (no slide under reduced motion). An inactive tab
          is muted but readable; hover and focus show. Round 1's padded row still looked cramped. */}
      <TabsList variant="line" className={cn("relative w-full gap-0 rounded-none border-b border-border/60 p-0", ZONE_HEADER, "group-data-horizontal/tabs:h-11")}>
        {([["data", "Data", summaries.show], ["sort", "Sort & filter", summaries.sortAndFilter], ["style", "Style", undefined]] as const).map(([value, label, title]) => (
          <TabsTrigger
            key={value} value={value} title={title} data-slot="inspector-tab"
            className={cn(
              "h-full min-w-0 flex-1 justify-center rounded-none border-0 px-1 text-[13px] whitespace-nowrap @[18rem]:text-sm",
              "text-muted-foreground transition-[color,background-color] duration-200 ease-[cubic-bezier(0.2,0,0,1)] hover:text-foreground group-data-[variant=line]/tabs-list:hover:bg-muted/50",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:ring-inset focus-visible:outline-none",
              "data-active:text-foreground dark:text-muted-foreground dark:data-active:text-foreground",
              // The sliding indicator below draws the underline; the tab's own one stays hidden.
              "after:hidden",
            )}
          >
            {label}
          </TabsTrigger>
        ))}
        <TabsPrimitive.Indicator
          aria-hidden
          className="absolute bottom-[-1px] left-0 h-0.5 w-(--active-tab-width) translate-x-(--active-tab-left) bg-foreground transition-[translate,width] duration-200 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:transition-none"
        />
      </TabsList>
      <div className="@container min-h-0 flex-1 overflow-y-auto">
        <TabsContent value="data">
          <div className={FORM_GRID} ref={(n) => { panelRefs.current[2] = n; }}>
          {isMap ? (
            <div className="grid gap-1.5">
              <Label>Map</Label>
              {/* BI-16 C2a follow-up (planner, picker search): about 250 maps need a search. */}
              <MapPicker maps={mapCatalogue} value={effectiveMap} onChange={setMapId} />
              {mapCredit(effectiveMap) ? <p className="text-xs text-muted-foreground">{mapCredit(effectiveMap)}</p> : null}
              {isChoropleth ? (
                <p className="text-xs text-muted-foreground">The region column is matched to the map by name; rows that match no region are counted under the map.</p>
              ) : null}
            </div>
          ) : null}

          {kind === "table" ? <TableSection part="shape" draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} groupedColumns={shownColumns} /> : null}
          {!isSingle && !ownColumns ? (
            <div className="grid grid-cols-1 items-start gap-3 @md:grid-cols-2">
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
                <Label>{dimensionLabel} {isPoints ? <span className="font-normal text-muted-foreground">(optional, shown in the tooltip)</span> : isBullet ? <span className="font-normal text-muted-foreground">(optional, one bullet per category)</span> : <Req />}</Label>
                {isPoints || isBullet ? (
                  <Select value={dimension || NO_BREAKDOWN} items={{ ...NO_LABEL_ITEMS, ...columnItems(fields?.dimensions ?? [], fieldLabel, dimension) }} onValueChange={(v) => setDimension(v === NO_BREAKDOWN ? "" : v ?? "")} disabled={!fields}>
                    <SelectTrigger className="w-full" title={dimension ? displayColumnFull(dimension) : undefined}><SelectValue placeholder={fields ? (isBullet ? "one bullet" : "no label") : noFieldsHint} /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NO_BREAKDOWN}>{isBullet ? "— no category —" : "— no label —"}</SelectItem>
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
                    <SelectTrigger className="w-full" title={breakdown ? displayColumnFull(breakdown) : undefined}><SelectValue placeholder={needsBreakdown ? "pick a 2nd column" : "no split"} /></SelectTrigger>
                    <SelectContent>
                      {needsBreakdown ? null : <SelectItem value={NO_BREAKDOWN}>— no split —</SelectItem>}
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
                <div className="grid grid-cols-1 gap-3 @md:col-span-2 @md:grid-cols-2" data-slot="conditional-row">
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
                    <div className="grid content-start gap-3 @md:col-start-2">
                      {breakdownGrainPicker}
                      {breakdownBinPicker ? (
                        <BinPicker label="Group the split in" value={shape.breakdownBin} sqlSource={onSqlSource} aria="Group the split in bins"
                          onChange={(breakdownBin) => setShape((d) => ({ ...d, breakdownBin }))} />
                      ) : null}
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}
          {isPoints ? (
            <div className="grid grid-cols-1 items-start gap-3 @md:grid-cols-2">
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
            <p className="text-xs text-muted-foreground">Pick a date column as the dimension; one cell per day, up to the last year of data.</p>
          ) : null}

          {noMeasures ? null : (
            <MeasureRows
              rows={measureRows} onChange={setMeasureRows} labels={measureTitle}
              min={limits.min} max={limits.max} own={limits.own} columns={measureColumns} defaultAggregate={aggregate}
              onDefaultAggregate={limits.own || isBoxplot ? undefined : setAggregate} fixedNote={isBoxplot ? "min, quartiles, max" : undefined}
              disabled={!fields} placeholder={fields ? "pick a column" : noFieldsHint} wholeMeasure={wholeMeasure} fieldLabel={fieldLabel}
            />
          )}
          {noMeasures ? null : isKpi || isProgress ? (
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
              Up to {formatLocaleNumber(pointLimit(choice?.kind === "sql" ? choice.id : undefined))} points, largest values first.
              One point per distinct latitude and longitude{dimension ? " and label" : ""}.
            </p>
          ) : null}
          {isProgress || isBullet ? (
            <GoalFields
              kind={isProgress ? "progress" : "bullet"} draft={typeDraft} onChange={changeType}
              columns={measureColumns.filter((c) => c !== measure)} fieldLabel={fieldLabel} disabled={!fields} placeholder={noFieldsHint}
            />
          ) : null}
          {isBullet ? <BandFields draft={typeDraft} onChange={changeType} /> : null}
          {isHistogram ? <HistogramFields draft={typeDraft} onChange={changeType} /> : null}
          {isGantt ? (
            <GanttFields
              draft={typeDraft} onChange={changeType} fieldLabel={fieldLabel} disabled={!fields} placeholder={noFieldsHint}
              dateColumns={coordinateColumns.filter((c) => fields?.kinds?.[c] === "date" || fields?.kinds?.[c] === "datetime")}
              numberColumns={numberColumns}
            />
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
            {rules.layers ? group(
              <LayersSection
                layers={layers} onChange={setLayers}
                baseKind={dimension ? fields?.kinds?.[dimension] : undefined} grained={!!(grain && grainOptions.includes(grain))}
                marts={marts.map((m) => m.name)} sqlSources={sqlSources.map((s) => ({ id: s.id, title: s.title }))}
                ownDraw={ownDraw} ownAxis={ownAxis} onOwnChange={(patch) => { if (patch.draw !== undefined) setOwnDraw(patch.draw); if (patch.axis !== undefined) setOwnAxis(patch.axis); }}
                blocked={layersBlocked.length ? `Remove ${layersBlocked.join(", ")} to add a second source.` : isNewSql ? "Save the SQL as a source first to add a second source to it." : undefined}
                disabled={!fields}
              />,
            ) : null}
            {isKpi ? group(<KpiCompareSection draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} metric={pickedMetric ? { label: pickedMetric.label, timeColumn: pickedMetric.timeColumn, grain: pickedMetric.grain } : undefined} />) : null}
            {isGroupedTable ? group(<TableSection part="format" draft={tableDraft} onChange={setTableDraft} columns={sourceColumns} groupedColumns={shownColumns} />) : null}
            {group(
              <>
                {rules.parameters ? (
                  <div>
                    <Button type="button" size="sm" variant="ghost" className="-ml-2 h-7 px-2 text-xs" onClick={() => setParamsOpen(true)}>
                      <SlidersHorizontal className="size-3" aria-hidden /> Parameters
                    </Button>
                  </div>
                ) : null}
                {rules.click ? <ClickSetting value={click} onChange={setClick} boards={boardOptions} /> : null}
                <FreshnessField id="chart-freshness" draft={cacheDraft} onChange={setCacheDraft}
                  help="How long this chart's result is remembered. Follow the default uses the dashboard's setting." />
              </>,
            )}
          </div>
        </TabsContent>
        <TabsContent value="sort">
          <div className={FORM_GRID} ref={(n) => { panelRefs.current[3] = n; }}>
            {rules.sort !== "none" ? (
              <div className="grid grid-cols-1 items-start gap-3 @md:grid-cols-2">
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
                  <Label>{capsBreakdown
                    ? (grain ? "Split values (Top N)" : "Limit (Top N, X axis and split)")
                    : grain ? "Buckets (max 1000, latest kept)" : isCalendar ? "Days (max 366)" : "Limit (Top N)"}</Label>
                  {limitParamUsed ? null : (
                    <WholeNumberInput aria-label="Limit" min={1} max={maxLimit} value={Math.min(limit, maxLimit)} onCommit={(n) => { setLimitTouched(true); setLimit(n); }} />
                  )}
                  {numberParamDefs.length && !limitParamOk ? (
                    <p className="text-xs text-muted-foreground">
                      {stepsOn ? "A parameter cannot set the limit of a chart with steps." : "A parameter cannot set the limit of this chart type."}
                    </p>
                  ) : numberParamDefs.length ? (
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

            {rules.filters ? <ChartFilters value={shape.filters} onChange={(filters) => setShape((d) => ({ ...d, filters }))} columns={columnChoices} disabled={!fields} segments={fields?.segments ?? []} source={filterSource} startWith={filterAsk} /> : null}
            {rules.steps ? group(<StepsEditor value={shape.steps} onChange={(steps) => setShape((d) => ({ ...d, steps }))} base={baseResult} disabled={!fields} />) : null}
          </div>
        </TabsContent>
        <TabsContent value="style">
          {/* BI-17 part A: formats, colours, axes, labels, reference lines, notes, tooltip; conditional colours for tables and KPIs. */}
          <div className={FORM_GRID}>
            {/* BI-58 review round 1, SHOULD-FIX 5 (owner): the width moved here from the header. */}
            <SettingGroup title="Size on dashboard">
              <SpanPicker value={span} onChange={setSpan} />
            </SettingGroup>
            <FormatSection rules={fmtRules} style={style} onChange={setStyle} valueNames={formatValueNames} seriesNames={formatValueNames} paletteValue={effectivePalette(style, palette, isEdit)}>
              {rules.referenceLines ? <ReferenceLinesEditor value={refLines} onChange={setRefLines} numberParams={numberParamDefs} valueNames={resultNames} takesForecast={kind !== "hbar"} /> : null}
            </FormatSection>
          </div>
        </TabsContent>
      </div>
      {paramsOpen ? <ParamDialog open onOpenChange={setParamsOpen} canWrite onChanged={() => { if (source) void loadFields(source); }} /> : null}
    </Tabs>
  );

  // `BI-58` T3: the preview's rows, for the "Data" view under the chart and for the type gallery.
  const resultRows = preview && "rows" in preview.result ? preview.result : null;
  const rowsView: PreviewRows | null = preview?.previewRows?.rows.length ? preview.previewRows
    : resultRows ? { columns: resultRows.columns, rows: resultRows.rows, truncated: false, limit: resultRows.rows.length } : null;
  const galleryPreview: GalleryPreview | null = preview && resultRows?.rows.length
    ? { spec: preview.spec, columns: resultRows.columns, rows: resultRows.rows } : null;
  const sourceFacts: SourceFacts | null = fields ? {
    dates: Object.values(fields.kinds ?? {}).filter((k) => k === "date" || k === "datetime").length,
    groups: fields.dimensions.filter((d) => !slotParam(d)).length,
    numbers: fields.measures.filter((m) => !slotParam(m)).length + (fields.metrics?.length ?? 0),
  } : null;
  // A chart with steps opens its rows, as the rows under its preview always did (BI-6 part 2); the person's toggle wins.
  const hasSteps = !!(preview?.spec as { def?: ShapeDefFields } | undefined)?.def?.steps?.length;
  const dataOpen = (showData ?? hasSteps) && !!rowsView && !showRows;
  /** What the preview holds, in plain words, under the chart. */
  const resultNote = showRows ? "Rows from your SQL"
    : !resultRows ? (previewBusy ? "Running the query…" : "No result yet")
    : isSingle ? "One value"
    : (() => {
        const n = resultRows.rows.length;
        const noun = rowsMode ? (n === 1 ? "row" : "rows") : isGantt ? (n === 1 ? "task" : "tasks") : n === 1 ? "group" : "groups";
        const before = preview?.previewRows;
        return before && before.rows.length > n
          ? `Showing ${formatLocaleNumber(n)} of ${before.truncated ? "more than " : ""}${formatLocaleNumber(before.rows.length)} ${noun}`
          : `${formatLocaleNumber(n)} ${noun}`;
      })();

  const previewBody = showRows ? (
    <div className="flex h-full flex-col gap-3 overflow-auto">
      {chartHold ? <p className="text-sm text-muted-foreground">{chartHold}</p> : null}
      {draft.result && draft.phase !== "empty" ? (
        <SqlRowsTable result={draft.result} stale={draft.phase === "stale"} className="min-h-0 flex-1" />
      ) : (
        <EmptyState className="py-6" title="Run the SQL" description="Its first rows appear here." />
      )}
    </div>
  ) : preview ? (
    <div className={cn("grid h-full min-h-0 gap-3", dataOpen ? "grid-rows-[minmax(0,3fr)_minmax(0,2fr)]" : "grid-rows-1")}>
      <div className="min-h-0">
        <TileBody spec={preview.spec} cell={preview.result} dark={resolvedTheme === "dark"} loading={previewBusy} />
      </div>
      {dataOpen && rowsView ? (
        <div className="min-h-0 overflow-auto" data-slot="preview-rows">
          <PreviewRowsTable data={rowsView} labels={finalResult.labels} values={finalResult.measures} open />
        </div>
      ) : null}
    </div>
  ) : previewBusy || (!previewError && !missing.length) ? (
    <StateBox>
      <Spinner className="size-5 text-muted-foreground" />
      <p className="text-sm text-muted-foreground">Running the query…</p>
    </StateBox>
  ) : previewError ? (
    <StateBox>
      <span className="grid size-10 place-items-center rounded-xl bg-destructive/10 text-destructive"><CircleAlert className="size-5" aria-hidden /></span>
      <div className="grid max-w-sm gap-1">
        <p className="text-sm font-medium text-foreground">Preview unavailable</p>
        <p className="text-sm text-muted-foreground">{asSentence(previewError)}</p>
      </div>
      <Button type="button" variant="outline" size="sm" onClick={() => setRetry((n) => n + 1)}>
        <RefreshCw aria-hidden /> Try again
      </Button>
    </StateBox>
  ) : (
    <StateBox>
      <span className="grid size-10 place-items-center rounded-xl bg-muted text-muted-foreground"><ChartColumn className="size-5" aria-hidden /></span>
      <div className="grid max-w-xs gap-1">
        <p className="text-sm font-medium text-foreground">Your chart appears here</p>
        <p className="text-sm text-muted-foreground">{`Fill in ${missing.join(", ")} to see real data here.`}</p>
      </div>
      {!choice && !isNewSql ? (
        <Button type="button" variant="outline" size="sm" className="@7xl:hidden" onClick={() => setDataSheet(true)}>
          <Database aria-hidden /> Pick data
        </Button>
      ) : null}
    </StateBox>
  );

  const canSubmit = !busy && !(sqlActive && !!draft.runBlocker);

  // BI-58 review round 1, SHOULD-FIX 4 (owner): with no Cancel button, leaving asks first when the
  // chart changed. Everything the chart is saved with is in this key; until the person first
  // touches the builder, the baseline follows it, so the loading and the builder's own settling of
  // an edited chart do not count as changes.
  const dirtyKey = JSON.stringify([previewInputsKey, subtitle, cacheDraft, click ?? null, sqlActive ? [draft.sql, draft.name] : null]);
  const [touched, setTouched] = React.useState(false);
  const [baseline, setBaseline] = React.useState<string | null>(null);
  React.useEffect(() => { if (!touched) setBaseline(dirtyKey); }, [dirtyKey, touched]);
  const dirty = touched && baseline !== null && baseline !== dirtyKey && !busy;
  const markTouched = () => { if (!touched) setTouched(true); };
  /** The confirm is open: for the back arrow or Esc (no anchor), or for a link that was held back. */
  const [leaving, setLeaving] = React.useState<{ anchor?: HTMLAnchorElement } | null>(null);
  const passLink = React.useRef(false);
  function leave() {
    if (dirty) setLeaving({}); else onCancel();
  }
  function discard() {
    const anchor = leaving?.anchor;
    setLeaving(null);
    if (!anchor) { onCancel(); return; }
    // The held-back link goes on as the person clicked it, past the guard this once.
    passLink.current = true;
    anchor.click();
    passLink.current = false;
  }
  // Esc leaves when nothing is open over the page (a popover, a list, a menu or a dialog closes first).
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector('[role="dialog"], [role="alertdialog"], [role="listbox"], [role="menu"]')) return;
      if (dirty) setLeaving({}); else onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dirty, onCancel]);
  // A link inside the console (the sidebar, a breadcrumb) is held back while there are changes.
  // Next has no way to stop its router, so the click is stopped before its handler sees it. The
  // browser's own back button cannot be held back this way; a reload or a tab close asks below.
  React.useEffect(() => {
    if (!dirty) return;
    const onClick = (e: MouseEvent) => {
      if (passLink.current || e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const anchor = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!anchor || anchor.target === "_blank" || anchor.hasAttribute("download")) return;
      if (new URL(anchor.href, window.location.href).origin !== window.location.origin) return;
      e.preventDefault();
      e.stopPropagation();
      setLeaving({ anchor });
    };
    const onUnload = (e: BeforeUnloadEvent) => { e.preventDefault(); e.returnValue = ""; };
    window.addEventListener("click", onClick, true);
    window.addEventListener("beforeunload", onUnload);
    return () => {
      window.removeEventListener("click", onClick, true);
      window.removeEventListener("beforeunload", onUnload);
    };
  }, [dirty]);

  return (
    // `BI-58` T1 (layout A of 2026-11-14): the chart is the page. A header row (where the chart
    // comes from and goes, and the two actions), then three zones: the data panel, the canvas and
    // the inspector. The zones follow the builder's own width (a container query), not the
    // window's, because the app's sidebar takes a different share of each window.
    // `BI-58` T5: from two columns up the builder fits the window instead of scrolling it: the
    // navbar (4rem) and `main`'s top padding (1.5rem) above, and the frame's `--content-bottom` below.
    // Owner QA 2026-11-14, composer: that bottom room is now just the composer and a small gap, the
    // same token `main` pads with, so the zones end about 14 px above the composer.
    <div className="@container" data-slot="chart-builder" onPointerDownCapture={markTouched} onKeyDownCapture={markTouched} onInputCapture={markTouched}>
      <div className="flex flex-col gap-4 @3xl:h-[calc(100dvh-5.5rem-var(--content-bottom,1.5rem))] @3xl:min-h-[30rem] @3xl:transition-[height] @3xl:duration-200 @3xl:ease-out motion-reduce:transition-none">
        {/*
          BI-58 review round 1, SHOULD-FIX 4 and 5 (owner): one action, one button. The back arrow
          leaves (asking first when something changed), the dashboard name in the breadcrumb is
          where the chart goes (the old "Add to" select), and Save is the only other button. The
          width moved to the Style tab.
        */}
        <header className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
          <div className="flex min-w-0 items-center gap-1">
            <Button type="button" variant="ghost" size="icon-sm" aria-label="Back to dashboard" title="Back to dashboard" onClick={leave}>
              <ArrowLeft aria-hidden />
            </Button>
            <Select value={targetBoard} items={boardLabels} onValueChange={(v) => setTargetBoard(v ?? "default")}>
              <SelectTrigger
                aria-label="Add to dashboard" title="The dashboard this chart is saved on"
                className="h-8 w-auto max-w-56 min-w-0 gap-1 border-transparent bg-transparent px-2 text-sm text-muted-foreground shadow-none transition-colors duration-200 hover:bg-muted hover:text-foreground dark:bg-transparent dark:hover:bg-muted"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {boardOptions.map((b) => (
                  <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span aria-hidden className="text-sm text-muted-foreground">/</span>
            <h1 className="ml-1.5 truncate text-sm font-medium text-foreground">{isEdit ? "Edit chart" : "New chart"}</h1>
          </div>
          {/* The SQL source of a new chart must be run once before the chart can be saved over it;
              until then the builder says so (`draft.runBlocker`). */}
          <Button type="button" className="min-w-28" disabled={!canSubmit} onClick={() => void save()}>
            {busy ? (isEdit ? "Saving…" : "Creating…") : isEdit ? "Save changes" : "Create chart"}
          </Button>
        </header>

        {error ? <p role="alert" className="text-sm text-destructive">{asSentence(error)}</p> : null}
        {/* Why the chart cannot be saved yet; the API's own error is the line above. */}
        {!error && sqlActive && draft.blocker ? <p className="text-sm text-muted-foreground">{draft.blocker}</p> : null}

        <div className={cn(
          "grid min-h-0 flex-1 gap-4",
          isStaticTile ? "@3xl:grid-cols-[minmax(0,1fr)_20rem]" : "@3xl:grid-cols-[minmax(0,1fr)_20rem] @7xl:grid-cols-[16rem_minmax(0,1fr)_20rem]",
        )}>
          {isStaticTile ? null : (
            <aside
              aria-label="Data" ref={(n) => { panelRefs.current[1] = n; }}
              className="hidden min-h-0 min-w-0 flex-col gap-4 overflow-y-auto rounded-xl bg-card p-3 ring-1 ring-foreground/6 @7xl:flex"
            >
              {dataPanel}
            </aside>
          )}

          <section aria-label="Chart" className="@container/canvas flex min-h-[26rem] min-w-0 flex-col rounded-xl bg-card ring-1 ring-foreground/6">
            <div data-slot="canvas-toolbar" className={cn("flex items-center gap-2 border-b border-border/60 px-3", ZONE_HEADER)}>
              {isStaticTile ? null : (
                <Button
                  type="button" variant="outline" size="sm" className="max-w-44 shrink-0 @7xl:hidden"
                  aria-haspopup="dialog" aria-label={choice || isNewSql ? `Data: ${sourceLabels[source] ?? source}` : "Pick data"} onClick={() => setDataSheet(true)}
                >
                  <Database aria-hidden /> <span className="truncate">{choice || isNewSql ? sourceLabels[source] ?? source : "Pick data"}</span>
                </Button>
              )}
              <KindPicker kind={kind} onPick={pickKind} facts={sourceFacts} preview={galleryPreview} dark={resolvedTheme === "dark"} />
            </div>
            <div className="flex min-h-0 flex-1 flex-col gap-3 px-5 pt-4 pb-4">
              <div className="grid gap-0.5">
                <Label htmlFor="ch-title" className="text-xs font-normal text-muted-foreground">Title {isStaticTile ? <Req /> : null}</Label>
                <input
                  id="ch-title" value={title} onChange={(e) => setTitle(e.target.value)}
                  placeholder={suggestedTitle || "e.g. Visitors by Region"}
                  className={cn("-mx-1.5 min-w-0 rounded-md bg-transparent px-1.5 py-0.5 text-xl font-semibold tracking-tight placeholder:text-muted-foreground/70 hover:bg-muted/70 focus-visible:bg-muted/70", INLINE_EDIT)}
                />
                {isStaticTile ? null : describing || subtitle ? (
                  <div className="mt-1 grid gap-0.5">
                    <Label htmlFor="ch-description" className="text-xs font-normal text-muted-foreground">Description</Label>
                    <input
                      id="ch-description" value={subtitle} autoFocus={describing} onChange={(e) => setSubtitle(e.target.value)}
                      onBlur={() => setDescribing(false)} placeholder="One line under the title"
                      className={cn("-mx-1.5 min-w-0 rounded-md bg-transparent px-1.5 py-0.5 text-sm text-muted-foreground placeholder:text-muted-foreground/60 hover:bg-muted/70 focus-visible:bg-muted/70", INLINE_EDIT)}
                    />
                  </div>
                ) : (
                  <button
                    type="button" onClick={() => setDescribing(true)}
                    className={cn("justify-self-start rounded text-xs text-muted-foreground hover:text-foreground", INLINE_EDIT)}
                  >
                    + Add a description
                  </button>
                )}
              </div>
              <section aria-label="Preview" className="relative min-h-56 flex-1">
                <div className="absolute inset-0">{previewBody}</div>
                <span
                  aria-live="polite"
                  className={cn(
                    "pointer-events-none absolute top-0 right-0 inline-flex items-center gap-1.5 rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground transition-opacity duration-200",
                    previewBusy && preview ? "opacity-100" : "opacity-0",
                  )}
                >
                  {previewBusy && preview ? <><Spinner className="size-3" /> Updating</> : null}
                </span>
              </section>
            </div>
            <footer className="flex min-h-11 flex-wrap items-center justify-between gap-x-4 gap-y-2 border-t border-border/60 px-3 py-2">
              <p className="text-xs text-muted-foreground tabular-nums" data-slot="result-note">{resultNote}</p>
              {rowsView && !heldPreview ? (
                <Button type="button" size="sm" variant={dataOpen ? "secondary" : "ghost"} aria-pressed={dataOpen} onClick={() => setShowData(!dataOpen)}>
                  <Table2 aria-hidden /> Data
                </Button>
              ) : null}
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
              ) : null}
            </footer>
          </section>

          <aside aria-label="Settings" className="@container flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl bg-card ring-1 ring-foreground/6">
            {inspector}
          </aside>
        </div>
      </div>
      <Dialog open={leaving !== null} onOpenChange={(open) => { if (!open) setLeaving(null); }}>
        <DialogContent className="sm:max-w-sm" showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>Discard changes?</DialogTitle>
            <DialogDescription>Your changes to this chart are not saved.</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setLeaving(null)}>Keep editing</Button>
            <Button type="button" variant="destructive" onClick={discard}>Discard</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      {isStaticTile ? null : (
        <Sheet open={dataSheet} onOpenChange={setDataSheet}>
          <SheetContent side="left" className="w-[min(22rem,calc(100vw-2rem))] gap-0 p-0 sm:max-w-none">
            <SheetHeader className="p-4 pb-2">
              <SheetTitle className="text-base">Data</SheetTitle>
              <SheetDescription>Pick the source, then click a column to use it.</SheetDescription>
            </SheetHeader>
            <div ref={sheetPanel} className="grid min-h-0 flex-1 content-start gap-4 overflow-y-auto p-3 pb-6">{dataPanel}</div>
          </SheetContent>
        </Sheet>
      )}
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
    </div>
  );
}
