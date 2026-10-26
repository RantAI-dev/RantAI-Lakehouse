"use client";

import * as React from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useTheme } from "next-themes";
import { ArrowRightLeft, Bell, MessageSquare, ChartColumn, Download, GitFork, ImageDown, Heading, Maximize2, Minus, MousePointerClick, Move, Moon, Pencil, Plus, RefreshCw, Smartphone, SlidersHorizontal, Sparkles, Square, Table2, Trash2, Rows3 } from "lucide-react";
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { PageHeader } from "@/components/patterns/page-header";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@rantai/design-system/ui/empty";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { dashboardDestination, queryDestination, urlDestination } from "@/lib/click-destination";
import { binRangeFilter, isLastBin, toggleBinFilter } from "@/lib/chart-bins";
import { bucketActions, bucketRangeFilter, dashboardRangeDestination, toggleBucketFilter } from "@/lib/bucket-click";
import { ParamControls } from "./param-controls";
import { ParamDialog } from "./param-dialog";
import { PARAMS_PARAM, normalizeDef, paramsDiffer, paramsQuery, readParamsParam, savedParamsBody, withParamsParam, withValue, type BoardParam, type ParamControl, type ParamValue } from "@/lib/params";
import { GRAIN_PARAM, differsFromSaved, grainQuery, readGrainParam, savedGrainBody, shownGrain, withGrainParam, type GrainChoice } from "@/lib/grain-switch";
import { PART_NO_CLICK_REASON, isGrain, isTruncation, switchChoices, type ColumnKind, type Grain, type ReportingContext } from "@/lib/time-grain";
import { effectiveRefresh, canSaveRefresh as refreshIsSaveable, listedInterval } from "@/lib/dashboard-refresh";
import { enterFullscreen, leaveFullscreen, readFullscreenDark, writeFullscreenDark } from "@/lib/dashboard-fullscreen";
import { notifyFailure, notifyInfo, notifySuccess } from "@/lib/notify";
import { readableErrorMessage } from "@/lib/notify-message";
import { summarizeFilters, summarizeFilterJson, summarizeGrain, summarizeParams, summarizeTabs, summarizeTiles } from "@/lib/page-context-summary";
import { FILTER_PARAM, decodeFilters, dropInertFilters, enforceRequired, filtersEqual, filtersToParam, isActiveFilter, normalizeFilters, toggleValue } from "@/lib/dashboard-filter-state";
import { extractViewFiltersFromChat } from "@/lib/dashboard-view-filter-from-chat";
import { canDrill, drillTarget, offersTileRecords, type ChartClickHandler, type ClickSpec } from "@/lib/chart-click";
import type { ChartRenderSpec, ChartSource } from "@/lib/dashboard-specs";
import {
  TAB_PARAM, addTab, bottomOfTab, defaultTileHeight, deleteTab, filtersOnTab, isSingleValueChart, isStaticKind, moveTab, moveTileToTab, placeUnboxed, renameTab, selectedTab, tabOf, tilesOnTab, withTabParam, type TabEdit,
} from "@/lib/dashboard-tabs";
import type { BoardTab, LayoutMap, PhoneTile } from "@/services";
import type { ChartClick, FilterDef, FilterField } from "@/services/clients/bi-store";
import { useAuth } from "@/features/auth/auth-provider";
import { useCopilot } from "@/features/copilot/use-copilot";
import { useService } from "@/hooks/use-service";
import { dashboardService, queryService } from "@/services";
import { newRunId, RUN_ID_HEADER } from "@/lib/run-id";
import { STOPPED_SENTENCE } from "@/lib/query-limits";
import { apiFetch } from "@/services/http";
import { useAutoRefresh } from "./auto-refresh";
import { BoardSwitcher } from "./board-switcher";
import { ChartBuilder, type ChartDef } from "./chart-builder";
import { fmtInt } from "./chart-option";
import { DashboardActionsMenu, EditLayoutButton, NewChartButton, RenameDashboardDialog } from "./dashboard-actions";
import { chartWriteItems } from "./tile-write-items";
import { ThemeFontDialog } from "./theme-font-dialog";
import { FreshnessDialog } from "./freshness-dialog";
import type { CacheRule } from "@/lib/result-cache";
import { dashboardLook, type DashboardTheme } from "@/lib/dashboard-look";
import { NOTE_KINDS, withNote } from "@/lib/style-draft";
import { SHIPPED_FONT_IDS } from "@/lib/formatting-settings";
import { labelledFolders } from "@/lib/dashboard-organise";
import { FavouriteButton } from "./favourite-button";
import { FilterRow } from "./filter-row";
import { PrivateBadge } from "./private-badge";
import { VersionHistoryDialog } from "./version-history-dialog";
import { notifyDashboardsChanged, useDashboardsChanged } from "./dashboard-events";
import { GrainMarkers } from "./grain-marker";
import { OwnFiltersMarker } from "./filters/own-filters-marker";
import { GrainSwitch } from "./grain-switch";
import { ReportingProvider } from "./reporting-context";
import { ChartLineagePanel } from "./chart-lineage-panel";
import { FilterBar } from "./filters/filter-bar";
import { SkippedFiltersMarker } from "./filters/skipped-marker";
import { DashboardGrid, type GridItem, type TileMenuItem } from "./dashboard-grid";
import { LineDialog } from "./line-dialog";
import { TabBar } from "./tab-bar";
import { isSectionKind } from "./tile-card";
import { ChartAlertDialog, type AlertChart } from "./chart-alert-dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { DashboardTilesSkeleton } from "./dashboard-skeleton";
import { DrillMenu, NoteDialog, RecordsDialog, type DrillTarget } from "./drill";
import { DrillTrail, useTileDrills } from "./tile-drills";
import type { RecordsRequest } from "./records";
import { forgetLastBoard, rememberLastBoard } from "./last-board";
import { offerFor } from "./access-level";
import { ShareDialog } from "./share-dialog";
import { CommentsDialog } from "./comments-panel";
import { TileBody } from "./tile-body";
import { exportNotice } from "./table-export";
import { downloadChartData, downloadChartPng, downloadDashboardFile, downloadDashboardPdf, downloadMenuKind, type DataChoice, type PdfChoice, type ReadState } from "./chart-download";
import { DownloadDataDialog, DownloadPdfDialog } from "./download-dialogs";
import { SubscriptionsDialog } from "./subscriptions-dialog";
import { TileDataDialog, TileExpandDialog, downloadRowsCsv, hasRows, type Cell } from "./tile-dialogs";

/**
 * The sentence for a refused dashboard write: the server's own, without the
 * `code:` prefix it may carry (the code is for the log, not for the reader:
 * BI-18 part C review fix (SHOULD-FIX) R20).
 */
function writeError(e: unknown): string | undefined {
  return e instanceof Error ? readableErrorMessage(e.message) : undefined;
}

type KpiMeta = { id: string; title: string; caption?: string; format: string };
type BoardOpt = { id: string; name: string; folderId?: string | null; ownerId?: string };
/** `resolved`: the columns a chart's parameter slots took in this response (BI-11). */
type ChartCard = ChartRenderSpec & { board?: string; def?: ChartDef; resolved?: { dimension?: string; breakdown?: string | null } };
type Payload = {
  board: string; layout: LayoutMap;
  /** The filters in force for this response (the address's, else the default). */
  filters: FilterDef[];
  /** The board's saved default, which `filters` is not while the address overrides it. */
  defaultFilters?: FilterDef[];
  /** The tabs of the board and the phone layout; absent when it has none (BI-18 part C). */
  tabs?: BoardTab[]; phoneLayout?: PhoneTile[];
  /** The board's saved auto-refresh in seconds; 0 when none (BI-18·B). */
  refreshSeconds?: number;
  /** BI-4 part B: the freshness rule in force for this board, and the board's own (absent follows the install). */
  cacheRule?: CacheRule | null;
  boardCacheRule?: CacheRule | null;
  /** The grain the board saved ("" when none), and the one this response used (BI-9). */
  grain?: string; appliedGrain?: string | null;
  /** The zone and first weekday the buckets were cut with (BI-9). */
  reporting?: ReportingContext;
  /** False when every role of the viewer is switched off from downloading (BI-1); the download items are hidden. Absent from an older server: allowed. */
  canDownload?: boolean;
  /** What the caller may do with this dashboard (`BI-19` B2); absent from an older server: the global permission decides. */
  access?: { level: string };
  /** False when the deployment has no PDF renderer (`BI-1`, feature decision 6). Absent from an older server: not offered. */
  pdfSupported?: boolean;
  /** The dashboard's own theme and font (BI-17 part A); absent when it follows the console. */
  theme?: string; font?: string;
  /** The parameters the board shows with this response's value, the value each has saved (or by default), and every value in force (BI-11). */
  params?: ParamControl[]; defaultParams?: Record<string, ParamValue>; appliedParams?: Record<string, ParamValue>;
  filterColumns: string[]; filterFields?: FilterField[];
  boards: BoardOpt[]; kpis: KpiMeta[];
  charts: ChartCard[]; results: Record<string, Cell>; storeError?: string | null;
};

const SOURCE_BADGE: Record<ChartSource, { label: string; cls: string } | null> = {
  builtin: null,
  ai: { label: "AI", cls: "bg-violet-500/15 text-violet-600 dark:text-violet-400" },
  ui: { label: "Manual", cls: "bg-sky-500/15 text-sky-600 dark:text-sky-400" },
};
const NO_CHARTS: ChartCard[] = [];
const NO_KPIS: KpiMeta[] = [];
const NO_RESULTS: Record<string, Cell> = {};
const NO_FILTERS: FilterDef[] = [];
const NO_FIELDS: FilterField[] = [];
const NO_CONTROLS: ParamControl[] = [];
const NO_TABS: BoardTab[] = [];
const SECTION_TEXT_MAX = 120;

/** "open the Sales dashboard", for the hover cue of a chart with a saved click. */
function clickVerb(click: ChartClick, boards: readonly BoardOpt[]): string {
  if (click.kind === "dashboard") return `open ${boards.find((b) => b.id === click.board)?.name ?? "another dashboard"}`;
  if (click.kind === "query") return "open a saved query";
  return "open a link";
}

/**
 * What `lib/chart-click` needs to know about a tile. A stored chart's
 * category column is its definition's `dimension`; a built-in tile has no
 * definition, but its `x` is a real mart column (`canDrill` leaves out the
 * built-in line and area, whose axis is a derived period).
 */
/**
 * The tile's id when it has filters of its own (BI-6) or joined tables (BI-7), so the records list it
 * opens carries them: the server reads the tile's definition and lists the same relation.
 */
function ownFilterChart(spec: ChartCard): string | undefined {
  return spec.source !== "builtin" && (spec.def?.filters?.length || spec.def?.joins?.length) ? spec.id : undefined;
}

function clickSpecOf(spec: ChartCard, level?: string): ClickSpec {
  const own = spec.source !== "builtin";
  return {
    kind: spec.kind,
    source: spec.source,
    // BI-6 part 2: after steps a row is no longer a set of source rows: no value drill.
    // BI-10: a drilled tile shows the next level of its hierarchy, and clicks on that.
    dimension: (own ? (spec.def?.steps?.length ? undefined : level ?? spec.resolved?.dimension ?? spec.def?.dimension) : spec.x) || undefined,
    breakdown: (own ? spec.resolved?.breakdown ?? spec.def?.breakdown : spec.series) || undefined,
    y: spec.y,
  };
}

/**
 * A Tableau/Metabase-style dashboard — a drag/resize tile canvas, multiple
 * dashboards. Edit mode arranges the layout (saved to the lakehouse); View
 * mode is a clean presentation. Built-in, manual, and AI-built cards.
 *
 * The open board arrives as a prop from the `/dashboards/[id]` route segment
 * rather than being read from the URL here: `/dashboards` now resolves to a
 * board, so this canvas no longer has a URL without one.
 */
export function DashboardPage({ boardId }: { boardId: string }) {
  const router = useRouter();
  const board = boardId || "default";
  const isDefault = board === "default";

  const { resolvedTheme } = useTheme();
  const themeDark = resolvedTheme === "dark";
  const [data, setData] = React.useState<Payload | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [edit, setEdit] = React.useState(false);
  const [layout, setLayout] = React.useState<LayoutMap>({});
  // BI-18 part C: the phone layout as held here while an editor arranges it.
  const [phone, setPhone] = React.useState<PhoneTile[] | undefined>(undefined);
  const [phoneEdit, setPhoneEdit] = React.useState(false);
  const [lineDialog, setLineDialog] = React.useState<
    { mode: "add"; kind: "heading" | "section" } | { mode: "edit"; id: string; kind: string } | null
  >(null);
  // Filters are temporary and live in the address (`?f=`); the board's saved
  // default applies while the address carries none. Nothing is written to the
  // board until an editor presses "Save as default".
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { hasPermission, user } = useAuth();
  const fParam = searchParams.get(FILTER_PARAM);
  // The dashboard's grain switch lives in the address beside the filters.
  const gChoice: GrainChoice = readGrainParam(searchParams.get(GRAIN_PARAM));
  // BI-11: the parameter values a viewer chose live in the address too (`?p=`).
  const pParam = searchParams.get(PARAMS_PARAM);
  const pValues = React.useMemo(() => readParamsParam(pParam), [pParam]);
  const [paramsOpen, setParamsOpen] = React.useState(false);
  const [savingDefault, setSavingDefault] = React.useState(false);
  const [editing, setEditing] = React.useState<{ id: string; def: ChartDef } | null>(null);
  // Tile delete is one click away, so it asks first.
  const [removing, setRemoving] = React.useState<{ id: string; title: string } | null>(null);
  const [removeBusy, setRemoveBusy] = React.useState(false);
  const [renameOpen, setRenameOpen] = React.useState(false);
  const [themeOpen, setThemeOpen] = React.useState(false);
  // BI-4 part B: the dashboard's own freshness rule, saved through the board write path.
  const [freshOpen, setFreshOpen] = React.useState(false);
  const [savingFresh, setSavingFresh] = React.useState(false);
  const [freshError, setFreshError] = React.useState<string | null>(null);
  // BI-17: a note being pinned on a clicked point (edit mode), and the sentence when the save was refused.
  const [noteFor, setNoteFor] = React.useState<{ tileId: string; point: string } | null>(null);
  const [savingNote, setSavingNote] = React.useState(false);
  const [noteError, setNoteError] = React.useState<string | null>(null);
  const [savingTheme, setSavingTheme] = React.useState(false);
  const [themeError, setThemeError] = React.useState<string | null>(null);
  // Folders only group the title switcher here. Managing them and filing
  // a dashboard are on the dashboard list (`dashboard-list-page.tsx`).
  const foldersState = useService((signal) => dashboardService.listFolders(signal), []);
  const folders = React.useMemo(() => labelledFolders(foldersState.data ?? [], user?.id), [foldersState.data, user?.id]);
  const [shareOpen, setShareOpen] = React.useState(false);
  const [commentsOpen, setCommentsOpen] = React.useState(false);
  const [historyOpen, setHistoryOpen] = React.useState(false);
  // BI-18 part C, part 2: favourites, and "recent" noted once per dashboard opened.
  const shortcuts = useService((signal) => dashboardService.shortcuts(signal), []);
  const reloadShortcuts = shortcuts.reload;
  const isFavourite = shortcuts.status === "success" && shortcuts.data.favourites.some((f) => f.kind === "board" && f.id === board);
  React.useEffect(() => { void dashboardService.touchRecent("board", board).catch(() => undefined); }, [board]);
  const [newChartOpen, setNewChartOpen] = React.useState(false);
  const [fullscreen, setFullscreen] = React.useState(false);
  // Auto-refresh starts from the board's saved interval; a viewer's change is
  // this session's only (null = follow the saved one) and is never written
  // back unless an editor presses "Save as dashboard default" (BI-18·B).
  const [sessionSec, setSessionSec] = React.useState<number | null>(null);
  const [savingRefresh, setSavingRefresh] = React.useState(false);
  // The dark choice of the full screen is remembered per browser; read after
  // mount so the server-rendered page and the first client render agree.
  const [fsDark, setFsDark] = React.useState(false);
  React.useEffect(() => { setFsDark(readFullscreenDark()); }, []);
  const appDark = themeDark || (fullscreen && fsDark);
  // BI-17 part A (D6): the dashboard's own theme and font win over the console's.
  const look = dashboardLook(data ?? undefined, appDark, data?.reporting?.fonts);
  const dark = look.dark;
  // Drill / cross-filter: menu on data-point click + a modal of raw rows.
  const [drill, setDrill] = React.useState<(DrillTarget & { builtin: boolean; tileId?: string }) | null>(null);
  const [records, setRecords] = React.useState<RecordsRequest | null>(null);
  // Tiles whose saved click destination the viewer set aside for the drill menu (this session only).
  const [menuTiles, setMenuTiles] = React.useState<ReadonlySet<string>>(new Set());
  // BI-14: the chart whose "Where does this come from" panel is open.
  const [lineageChart, setLineageChart] = React.useState<{ id: string; title: string } | null>(null);
  const [tileDialog, setTileDialog] = React.useState<{ kind: "data" | "expand"; id: string } | null>(null);
  // BI-19 part C: a chart's ⋯ > "Alert me…".
  const [alertChart, setAlertChart] = React.useState<AlertChart | null>(null);
  // Only the newest load may write state. Creating a dashboard fires a
  // reload of the board being left and then navigates to the new one; the
  // old board's response (slower: it runs the built-in tiles) used to land
  // last and paint its charts under the new dashboard's name.
  const loadSeq = React.useRef(0);
  // BI-5 (plan D5): the run in flight, so Stop can end its queries on the engine and abort the request.
  const runRef = React.useRef<{ runId: string; controller: AbortController } | null>(null);
  const [stopped, setStopped] = React.useState(false);
  // `fresh` asks the server to compute instead of using what it remembers (BI-4 part B, feature
  // decision 6): "1" for every tile (Refresh), or a chart id for "Refresh this tile". The automatic
  // refresh and every other reload follow the rule and pass nothing.
  const load = React.useCallback(async (fresh?: string): Promise<Payload | undefined> => {
    const seq = ++loadSeq.current;
    const run = { runId: newRunId(), controller: new AbortController() };
    runRef.current = run;
    setLoading(true); setError(null); setStopped(false);
    try {
      const q = new URLSearchParams({ board });
      // A parameter that is not a filter list is ignored: the saved default shows.
      const fromAddress = decodeFilters(fParam);
      if (fromAddress) q.set("filters", JSON.stringify(fromAddress));
      const grainAsked = grainQuery(gChoice);
      if (grainAsked) q.set("grain", grainAsked);
      const paramsAsked = paramsQuery(pValues);
      if (paramsAsked) q.set("params", paramsAsked);
      if (fresh) q.set("fresh", fresh);
      const res = await apiFetch(`/api/dashboard?${q.toString()}`, { cache: "no-store", headers: { [RUN_ID_HEADER]: run.runId }, signal: run.controller.signal });
      const json = (await res.json()) as Payload;
      if (seq !== loadSeq.current) return undefined;
      if (!res.ok) throw new Error((json as { error?: string }).error ?? "Failed to load dashboard");
      // BI-11: a chart's parameter slots arrive as `{ param }` objects and are held as `{name}` text.
      setData({ ...json, charts: (json.charts ?? []).map((c) => (c.def ? { ...c, def: normalizeDef(c.def) } : c)) });
      setLayout(json.layout ?? {});
      setPhone(json.phoneLayout);
      return json;
    } catch (e) {
      // Stop is an answer, not an error: the tiles that were already on screen stay, and the page says "Stopped".
      if (run.controller.signal.aborted) {
        if (seq === loadSeq.current) setStopped(true);
        return undefined;
      }
      if (seq === loadSeq.current) setError(e instanceof Error ? e.message : String(e));
      return undefined;
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [board, fParam, gChoice, pValues]);

  /**
   * Stop on a dashboard that is still loading (BI-5 decision 4): ask the server to end this
   * run's queries on the engine, then abort the request. The cancel call is best effort (the
   * route lands with T4 of the BI-5 plan; until then it answers 404): the abort always happens.
   */
  const stopLoading = React.useCallback(() => {
    const run = runRef.current;
    if (!run) return;
    void queryService.cancel(run.runId).catch(() => undefined);
    run.controller.abort();
  }, []);

  // Switching dashboard starts in view mode; the switcher navigates without
  // `f`, so the new board opens on its own default.
  React.useEffect(() => { setEdit(false); setPhoneEdit(false); }, [board]);
  React.useEffect(() => { if (!edit) setPhoneEdit(false); }, [edit]);
  // `/dashboards` meneruskan ke board terakhir, jadi membukanya di sinilah
  // yang menentukan "terakhir" — bukan klik di daftar, karena kanvas juga
  // dicapai lewat switcher, tautan bersama dan Copilot.
  React.useEffect(() => { rememberLastBoard(board); }, [board]);
  React.useEffect(() => { void load(); }, [load]);
  // Charts added or removed elsewhere (Copilot, board menus); folders are
  // reloaded too, since the same event covers filing a dashboard.
  const reloadFolders = foldersState.reload;
  useDashboardsChanged(React.useCallback(() => { void load(); reloadFolders(); }, [load, reloadFolders]));
  // Periodic refresh for presenting; pauses while the tab is hidden.
  const savedSec = listedInterval(data?.refreshSeconds);
  const autoSec = effectiveRefresh(data?.refreshSeconds, sessionSec);
  const reload = React.useCallback(async () => { await load(); }, [load]);
  useAutoRefresh(autoSec * 1000, reload);
  // A different board starts from its own saved interval.
  React.useEffect(() => { setSessionSec(null); }, [board]);
  // Full screen: the page's own overlay is always applied; the browser's
  // real full screen is requested on top of it where it is allowed, and the
  // viewer is told when it is not. Esc leaves the browser's full screen by
  // itself (a `fullscreenchange`) and the overlay's on the keydown below.
  const nativeFs = React.useRef(false);
  const leaveFs = React.useCallback(() => {
    setFullscreen(false);
    if (nativeFs.current) { nativeFs.current = false; void leaveFullscreen(document); }
  }, []);
  const toggleFullscreen = React.useCallback(() => {
    if (fullscreen) { leaveFs(); return; }
    setFullscreen(true);
    // Called from the click that asked for it, which the browser requires.
    void enterFullscreen(document).then((outcome) => {
      nativeFs.current = outcome === "native";
      if (outcome === "in-page") notifyInfo("Showing the dashboard in the page", "The browser did not allow full screen here.");
    });
  }, [fullscreen, leaveFs]);
  React.useEffect(() => {
    if (!fullscreen) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") leaveFs(); };
    const onChange = () => {
      // The browser left its full screen (Esc, F11): the overlay goes too.
      if (!document.fullscreenElement && nativeFs.current) leaveFs();
    };
    window.addEventListener("keydown", onKey);
    document.addEventListener("fullscreenchange", onChange);
    return () => {
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("fullscreenchange", onChange);
    };
  }, [fullscreen, leaveFs]);
  // Leaving the page (or the board) while in full screen leaves it too.
  React.useEffect(() => () => {
    if (nativeFs.current) { nativeFs.current = false; void leaveFullscreen(document); }
  }, []);
  const toggleFsDark = () => setFsDark((on) => { writeFullscreenDark(!on); return !on; });

  // Placeholders the pre-BI-18 bar stored (a column with no values) filter
  // nothing; dropped here so they neither draw a chip nor block the column.
  const defaultFilters = React.useMemo(
    () => (data?.defaultFilters ? dropInertFilters(data.defaultFilters) : NO_FILTERS),
    [data?.defaultFilters],
  );
  // The data on screen was loaded with these; before the first answer the
  // address (or nothing) is all there is.
  const filters = React.useMemo(
    () => enforceRequired(decodeFilters(fParam) ?? defaultFilters, defaultFilters),
    [fParam, defaultFilters],
  );
  const dirty = decodeFilters(fParam) !== null && filtersToParam(filters, defaultFilters) !== null;

  const writeAddress = React.useCallback((param: string | null) => {
    const q = new URLSearchParams(window.location.search);
    if (param === null) q.delete(FILTER_PARAM); else q.set(FILTER_PARAM, param);
    const qs = q.toString();
    // replace, not push: every click would otherwise add a history entry.
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  }, [router, pathname]);

  const applyFilters = React.useCallback((next: FilterDef[]) => {
    // Required columns (BI-18 round two) cannot be dropped: an emptied or
    // removed one comes back as the saved default's filter.
    writeAddress(filtersToParam(enforceRequired(next, defaultFilters), defaultFilters));
  }, [writeAddress, defaultFilters]);

  // A link that omits a required column loaded its tiles without it, before
  // the saved default was known. Put the column in the address, which reloads.
  React.useEffect(() => {
    const fromAddress = decodeFilters(fParam);
    if (!fromAddress || !data) return;
    const enforced = enforceRequired(fromAddress, defaultFilters);
    if (!filtersEqual(enforced, fromAddress)) writeAddress(filtersToParam(enforced, defaultFilters));
  }, [fParam, data, defaultFilters, writeAddress]);


  // BI-9: the grain switch. Only a chart whose own grain is a truncation
  // follows it; the control is offered only when there is one, and offers
  // hour and minute only when every such chart is on a timestamp.
  const grainedCharts = (data?.charts ?? []).filter((c) => isGrain(c.def?.grain) && isTruncation(c.def.grain as Grain));
  const grainChoices = switchChoices(
    grainedCharts.map((c) => {
      const cell = data?.results[c.id];
      return hasRows(cell) ? (cell.grainColumn as ColumnKind | undefined) : undefined;
    }),
  );
  const shownGrainValue = shownGrain(gChoice, data?.grain);
  const writeGrain = React.useCallback((choice: GrainChoice) => {
    const qs = withGrainParam(window.location.search, choice);
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  }, [router, pathname]);
  // BI-9 review fix (SHOULD-FIX) R4: one "Save as default" and one Reset for
  // the whole row. The filters and the grouping travel in one
  // `PUT /api/dashboard/boards` body (the route applies both) and only what
  // differs from the saved default is sent; Reset drops both address parameters
  // in a single navigation.
  const grainDirty = gChoice !== null && differsFromSaved(shownGrainValue, data?.grain);
  // BI-11: the parameters join the same row, the same Save as default and Reset.
  const paramControls = data?.params ?? NO_CONTROLS;
  const paramsDirty = pValues !== null && paramsDiffer(paramControls, data?.defaultParams);
  const rowDirty = dirty || grainDirty || paramsDirty;
  const changeParam = React.useCallback((name: string, value: ParamValue) => {
    const qs = withParamsParam(window.location.search, withValue(pValues, data?.defaultParams ?? {}, name, value));
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  }, [router, pathname, pValues, data?.defaultParams]);
  const resetRow = React.useCallback(() => {
    const q = new URLSearchParams(window.location.search);
    q.delete(FILTER_PARAM);
    q.delete(GRAIN_PARAM);
    q.delete(PARAMS_PARAM);
    const qs = q.toString();
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  }, [router, pathname]);
  const saveRowDefault = React.useCallback(async () => {
    setSavingDefault(true);
    try {
      const body: Record<string, unknown> = { id: board };
      if (dirty) body.filters = normalizeFilters(filters);
      if (grainDirty) body.grain = savedGrainBody(shownGrainValue);
      if (paramsDirty) body.params = savedParamsBody(paramControls);
      const res = await apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not save the defaults");
      }
      // The saved defaults now equal the state; dropping both parameters reloads onto them.
      if (grainDirty) setData((prev) => (prev ? { ...prev, grain: savedGrainBody(shownGrainValue) } : prev));
      resetRow();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingDefault(false);
    }
  }, [board, dirty, grainDirty, paramsDirty, paramControls, filters, shownGrainValue, resetRow]);
  // BI-11: an editor ticks a parameter in the dialog and its control joins (or leaves) this
  // dashboard's row. Written at once through the board write path; the values already saved stay.
  const toggleShownParam = React.useCallback(async (name: string, on: boolean) => {
    const current: BoardParam[] = paramControls.map((c) => ({ name: c.param.name, value: data?.defaultParams?.[c.param.name] }));
    const next = on ? [...current, { name }] : current.filter((p) => p.name !== name);
    try {
      const res = await apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: board, params: next }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not change the parameters shown");
      }
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [board, paramControls, data?.defaultParams, load]);
  // BI-18·B: an editor's choice becomes the dashboard's own default. Written
  // through the board write path (`dashboard:write`); the session choice is
  // dropped once it is the saved one.
  const saveRefreshDefault = React.useCallback(async () => {
    setSavingRefresh(true);
    try {
      const res = await apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: board, refreshSeconds: autoSec }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not save the auto-refresh");
      }
      setData((prev) => (prev ? { ...prev, refreshSeconds: autoSec } : prev));
      setSessionSec(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingRefresh(false);
    }
  }, [board, autoSec]);

  // BI-17 part A (D6): the theme and font are written through the board write path
  // (`dashboard:write`); a font the administrator does not allow comes back as the server's sentence.
  const saveThemeAndFont = React.useCallback(async (choice: { theme: DashboardTheme; font: string }) => {
    setSavingTheme(true);
    setThemeError(null);
    try {
      const res = await apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: board, theme: choice.theme, font: choice.font }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not save the theme and font");
      }
      setData((prev) => (prev ? { ...prev, theme: choice.theme === "auto" ? undefined : choice.theme, font: choice.font || undefined } : prev));
      setThemeOpen(false);
    } catch (e) {
      setThemeError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingTheme(false);
    }
  }, [board]);

  const saveFreshness = React.useCallback(async (rule: CacheRule) => {
    setSavingFresh(true);
    setFreshError(null);
    try {
      const res = await apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: board, cache: rule }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not save the freshness rule");
      }
      setFreshOpen(false);
      void load();
    } catch (e) {
      setFreshError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingFresh(false);
    }
  }, [board, load]);

  // BI-17: pin a note on a point. The chart is written whole through its own write route
  // (`dashboard:write`), with the note added to the stored definition.
  const saveNote = React.useCallback(async (text: string) => {
    const target = noteFor && data?.charts.find((c) => c.id === noteFor.tileId);
    if (!noteFor || !target?.def) return;
    setSavingNote(true);
    setNoteError(null);
    try {
      const res = await apiFetch("/api/dashboard/specs", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ ...withNote(target.def as { annotations?: never[] }, noteFor.point, text), id: noteFor.tileId, board }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not save the note");
      }
      setNoteFor(null);
      await load();
    } catch (e) {
      setNoteError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingNote(false);
    }
  }, [noteFor, data?.charts, board, load]);

  // Cross-filter: toggle a value in a column → filters EVERY tile with that column.
  const crossFilter = React.useCallback((column: string, value: string) => {
    applyFilters(toggleValue(filters, column, value));
    setDrill(null);
  }, [applyFilters, filters]);
  // BI-9: a bucket of a grouped chart sets one date range on its column.
  const crossFilterBucket = React.useCallback((column: string, grain: Grain, value: string) => {
    const range = bucketRangeFilter(column, grain, value);
    if (range) applyFilters(toggleBucketFilter(filters, range));
    setDrill(null);
  }, [applyFilters, filters]);

  // BI-10: a bin of a binned chart sets one numeric range on its column.
  const crossFilterBin = React.useCallback((column: string, lower: number, bin: { width: number; last: boolean }) => {
    applyFilters(toggleBinFilter(filters, binRangeFilter(column, lower, bin.width, bin.last)));
    setDrill(null);
  }, [applyFilters, filters]);

  // Drill-down: the rows behind the clicked value (or behind a whole tile),
  // for the filters in force so the list agrees with the number clicked. A
  // built-in tile ignores dashboard filters, so its rows are not narrowed.
  const openRecords = React.useCallback((request: Omit<RecordsRequest, "filters">, builtin: boolean, extra: FilterDef[] = NO_FILTERS, tileId?: string) => {
    setDrill(null);
    // BI-18 part C: the list of a tile takes the filters the tile is given.
    const given = tileId ? filtersOnTab(filters, tabOf(data?.tabs ?? NO_TABS, layout, tileId)) : filters;
    setRecords({ ...request, filters: builtin ? NO_FILTERS : [...given, ...extra], params: paramsQuery(data?.appliedParams ?? null) ?? undefined });
  }, [filters, data?.appliedParams, data?.tabs, layout]);

  // No "jump to the newest user dashboard" from the built-in board here.
  // That effect dated from when `/dashboards` was this page; it now is the
  // resolver (`dashboard-resolver.tsx`), which sends you to the board you
  // last had open. Opening Main from the list remembered "default", this
  // effect replaced the URL with `/dashboards?board=<newest>`, the resolver
  // sent you back to Main, and the page reloaded forever. Where to land is
  // the resolver's decision alone; Main is a board you can choose to open.

  // Save layout (debounced). The built-in board saves too: the backend
  // accepts a layout — and only a layout — for id "default".
  const saveTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const persistLayout = React.useCallback((next: LayoutMap) => {
    setLayout(next);
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      // BI-18 part C review fix (SHOULD-FIX) R19: a refused save is said, not dropped.
      dashboardService.saveBoardLayout(board, next).catch((e) => notifyFailure("The layout was not saved", writeError(e)));
    }, 600);
  }, [board]);

  async function removeChart(id: string) {
    setRemoveBusy(true);
    try {
      await dashboardService.deleteChart(id);
      setRemoving(null);
      void load();
    } catch (e) {
      notifyFailure("The chart was not deleted", writeError(e));
    } finally {
      setRemoveBusy(false);
    }
  }
  // BI-18 part C review fix (SHOULD-FIX) R19: every write below goes through
  // the service, which checks the answer, and says the server's sentence when
  // it is refused.
  async function duplicateDashboard() {
    try {
      const copy = await dashboardService.duplicateBoard(board);
      notifyDashboardsChanged();
      if (copy.id) router.push(`/dashboards/${copy.id}`);
    } catch (e) {
      notifyFailure("The dashboard was not duplicated", writeError(e));
    }
  }
  async function createDashboard() {
    try {
      const created = await dashboardService.createBoard({ name: "New dashboard" });
      notifyDashboardsChanged();
      if (created.id) router.push(`/dashboards/${created.id}`);
    } catch (e) {
      notifyFailure("The dashboard was not created", writeError(e));
    }
  }
  async function deleteDashboard() {
    if (isDefault) return;
    try {
      await dashboardService.deleteBoard(board);
    } catch (e) {
      notifyFailure("The dashboard was not deleted", writeError(e));
      return;
    }
    notifySuccess("Dashboard moved to the trash", "Restore it from Trash on the dashboard list for 30 days.");
    notifyDashboardsChanged();
    // Lupakan sebelum pergi, kalau tidak `/dashboards` menuntun balik ke sini.
    forgetLastBoard(board);
    router.push("/dashboards/browse");
  }
  async function renameDashboard(name: string) {
    if (isDefault) return;
    try {
      await dashboardService.updateBoard({ id: board, name });
    } catch (e) {
      notifyFailure("The dashboard was not renamed", writeError(e));
      return;
    }
    setRenameOpen(false);
    notifyDashboardsChanged();
  }
  // Export PDF through the browser's print dialog; print CSS stacks the tiles.
  function exportPdf() {
    setEdit(false);
    const prev = document.title;
    document.title = (dashName || "dashboard").replace(/[^\w\s-]/g, "").trim();
    setTimeout(() => { window.print(); document.title = prev; }, 200);
  }

  const boards = data?.boards ?? [{ id: "default", name: "Main" }];
  const dashName = boards.find((b) => b.id === board)?.name ?? "Dashboards";
  const ownerId = boards.find((b) => b.id === board)?.ownerId;
  // Stable empty fallbacks: a fresh `[]` each render re-ran the Copilot
  // context effect, which re-rendered this page, forever, while loading.
  const kpis = data?.kpis ?? NO_KPIS;
  const charts = data?.charts ?? NO_CHARTS;
  // BI-10: a viewer's own, temporary drill down a hierarchy; nothing is saved.
  const tileDrills = useTileDrills(charts, filters, !edit);
  const results = data?.results ?? NO_RESULTS;

  // ── BI-18 part C: tabs, section tiles and the phone layout ────────────────
  const tabs = data?.tabs ?? NO_TABS;
  const selTab = selectedTab(tabs, searchParams.get(TAB_PARAM));
  const chartIds = React.useMemo(() => charts.map((c) => c.id), [charts]);
  // BI-18 part C review fix (BLOCKER) R13: the layout AS SHOWN, every tile with a
  // box. Tab operations (move, delete a tab, add a block) work from this, never
  // from the stored map alone, which lacks a chart that was never placed.
  const shownLayout = React.useMemo(
    () => placeUnboxed(charts.map((c) => ({ id: c.id, span: c.span, kind: c.kind })), layout, tabs),
    [charts, layout, tabs],
  );
  // BI-19 B2: the dashboard's own level decides what is offered; the server
  // checks every write against the same level.
  const { canEdit: canWrite, canManage } = offerFor(data?.access?.level, hasPermission("dashboard:write"));
  const selectTab = React.useCallback((id: string) => {
    const qs = withTabParam(window.location.search, tabs, id);
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  }, [router, pathname, tabs]);
  async function putBoard(body: Record<string, unknown>) {
    const res = await apiFetch("/api/dashboard/boards", {
      method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: board, ...body }),
    });
    if (!res.ok) {
      const j = (await res.json().catch(() => null)) as { error?: string } | null;
      throw new Error(j?.error ?? "Could not save the dashboard");
    }
  }
  // One write for the tabs, the tiles that name them and the ties of the saved
  // filters, so the server never sees a tile on a tab that is gone.
  const saveTabEdit = async (next: TabEdit) => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    try {
      const body: Record<string, unknown> = { tabs: next.tabs, layout: next.layout };
      if (JSON.stringify(next.filters) !== JSON.stringify(defaultFilters)) {
        body.filters = normalizeFilters(next.filters);
      }
      await putBoard(body);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const currentEdit = (): TabEdit => ({ tabs, layout: shownLayout, filters: defaultFilters });
  const tabEditing = canWrite && !isDefault && edit
    ? {
        onAdd: (title: string) => {
          const next = addTab(currentEdit(), title);
          if (!next) { notifyFailure("A dashboard has at most 20 tabs"); return; }
          void saveTabEdit(next).then(() => selectTab(next.tabs[next.tabs.length - 1].id));
        },
        onRename: (id: string, title: string) => void saveTabEdit({ ...currentEdit(), tabs: renameTab(tabs, id, title) }),
        onMove: (id: string, by: -1 | 1) => void saveTabEdit({ ...currentEdit(), tabs: moveTab(tabs, id, by) }),
        onDelete: (id: string, moveTiles: boolean) => {
          const next = deleteTab(currentEdit(), chartIds, id, moveTiles);
          if ("error" in next) { notifyFailure(next.error); return; }
          if (selTab === id && next.tabs.length > 0) selectTab(next.tabs[0].id);
          void saveTabEdit(next);
        },
        tileCount: (id: string) => tilesOnTab(tabs, shownLayout, chartIds, id).length,
      }
    : undefined;
  // The phone layout: this tab's tiles replace their entries, other tabs' entries stay.
  const phoneTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const persistPhone = (forTab: PhoneTile[]) => {
    const mine = new Set(forTab.map((p) => p.id));
    const onTab = new Set(tilesOnTab(tabs, layout, chartIds, selTab ?? "").concat(selTab ? [] : chartIds));
    const next = [...forTab, ...(phone ?? []).filter((p) => !mine.has(p.id) && !onTab.has(p.id))];
    setPhone(next);
    if (phoneTimer.current) clearTimeout(phoneTimer.current);
    phoneTimer.current = setTimeout(() => {
      void putBoard({ phoneLayout: next }).catch((e) => setError(e instanceof Error ? e.message : String(e)));
    }, 600);
  };
  const resetPhone = async () => {
    try { await putBoard({ phoneLayout: [] }); setPhone(undefined); } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  // A heading, a rule or a section header, at the bottom of the tab being edited.
  const addSectionTile = async (kind: "heading" | "divider" | "section", text: string) => {
    try {
      const res = await apiFetch("/api/dashboard/specs", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ title: kind === "divider" ? "Divider" : text, kind, board, span: 2, ...(kind === "divider" ? {} : { text }) }),
      });
      const j = (await res.json().catch(() => null)) as { chart?: { id: string }; error?: string } | null;
      if (!res.ok || !j?.chart) throw new Error(j?.error ?? "Could not add the tile");
      // BI-18 part C review fix (SHOULD-FIX) R15: at the bottom of the tab as shown,
      // where the charts with no stored box are drawn too, like a new chart.
      const y = bottomOfTab(tabs, shownLayout, chartIds, selTab);
      const box = { x: 0, y, w: 12, h: defaultTileHeight(kind), ...(selTab ? { tab: selTab } : {}) };
      await putBoard({ layout: { ...shownLayout, [j.chart.id]: box } });
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const editSectionText = async (id: string, kind: string, text: string) => {
    try {
      const res = await apiFetch("/api/dashboard/specs", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id, title: text, kind, text, board, span: 2 }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not change the text");
      }
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  // A chart made while another tab is open lands on that tab, not on the first.
  const loadAndPlace = async () => {
    const known = new Set(chartIds);
    const json = await load();
    if (!json || !canWrite || tabs.length < 2 || !selTab || selTab === tabs[0].id) return;
    const added = (json.charts ?? []).filter((c) => !known.has(c.id) && !(c.id in (json.layout ?? {})));
    if (added.length === 0) return;
    const onTab = tilesOnTab(tabs, json.layout ?? {}, (json.charts ?? []).map((c) => c.id), selTab);
    let y = onTab.reduce((m, id) => Math.max(m, (json.layout?.[id]?.y ?? 0) + (json.layout?.[id]?.h ?? 0)), 0);
    const next: LayoutMap = { ...(json.layout ?? {}) };
    for (const c of added) { next[c.id] = { x: 0, y, w: c.span === 2 ? 12 : 6, h: 6, tab: selTab }; y += 6; }
    try { await putBoard({ layout: next }); await load(); } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };

  // Tell Copilot what is on this dashboard — the built-in one included.
  const {
    messages: messagesFromCopilot,
    setPageContext, setMode: setCopilotMode, setExpanded: setCopilotExpanded,
  } = useCopilot();
  // AI-2 (plan 20): a `set_board_view_filters` step in the most recent
  // assistant answer applies when its `board` matches the board on screen —
  // mirror what it set into the page address (`?f=`) with the same helper a
  // chip change uses. A step for another board is ignored here; the step
  // card carries the link instead.
  const chatViewFilters = React.useMemo(
    () => extractViewFiltersFromChat(messagesFromCopilot, board),
    [messagesFromCopilot, board],
  );
  // Re-applying the same filters (a re-render of the same message) loops back
  // through `writeAddress` and reloads the page; key on the filter JSON so a
  // single apply is one navigation.
  const chatViewFilterKey = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (chatViewFilters === null) return;
    const key = JSON.stringify(chatViewFilters);
    if (chatViewFilterKey.current === key) return;
    chatViewFilterKey.current = key;
    writeAddress(filtersToParam(chatViewFilters, defaultFilters));
  }, [chatViewFilters, defaultFilters, writeAddress]);
  // AI-2 (plan 20) — the page context carries the grain (shown, saved) and
  // the active filters as compact JSON so the model's table context lists
  // them in machine-readable form too. Each is bounded by its own budget;
  // `summarizeFilterJson` returns `null` over budget so the chip text stays
  // as the single source of truth.
  const grainText = React.useMemo(
    () => summarizeGrain(shownGrainValue, data?.grain),
    [shownGrainValue, data?.grain],
  );
  const filterJson = React.useMemo(
    () => summarizeFilterJson(filters, { maxChars: 4000 }),
    [filters],
  );
  React.useEffect(() => {
    // The numbers on screen, not just tile titles (plan §6): KPIs and tiles
    // with their first rows, and the filters in force. Built from data this
    // page already loaded; nothing extra is queried.
    const tiles = summarizeTiles(
      [
        ...kpis.map((k) => ({ id: k.id, title: k.title, kind: "kpi", source: "builtin" })),
        ...charts,
      ],
      results,
    );
    const filterLine = filterJson
      ? `Active filters: ${summarizeFilters(filters)} (raw: ${filterJson}).`
      : `Active filters: ${summarizeFilters(filters)}.`;
    setPageContext({
      key: "dashboard-view",
      title: `Working on "${dashName}"`,
      hint: "Create a chart, edit a tile, or ask about this dashboard.",
      suggest: {
        ask: ["Explain the charts on this dashboard", "Which category leads here?"],
        build: ["Add a KPI of total visitors", "Add a table of top countries", "Add a pie of visitors by region"],
      },
      system:
        `The user is viewing the dashboard "${dashName}" (board id: ${board}). ` +
        `${grainText}. ` +
        `${filterLine} ` +
        `Parameters shown (a formula reads one as {name}; list_parameters has them all): ${summarizeParams(paramControls)}. ` +
        `Tabs: ${summarizeTabs(tabs, (id) => tabOf(tabs, layout, id), chartIds, selTab)}. ` +
        `Tiles and the data they currently show (built-in tiles are not editable):\n${tiles || "none yet"}\n` +
        `When creating a chart use board="${board}". To change a tile you created, use update_chart with its id. ` +
        `You can also explain what the charts show.`,
    });
    return () => setPageContext(null);
  }, [board, dashName, charts, kpis, results, filters, grainText, filterJson, paramControls, tabs, layout, chartIds, selTab, setPageContext]);

  // Carry out a chart's saved click destination (BI-18·B) for the clicked
  // value. The server did not check that the target exists, so the console
  // says so; a URL is checked against the rule again before it is followed.
  const followClick = (click: ChartClick, value: string, grain?: Grain) => {
    if (click.kind === "dashboard") {
      if (!boards.some((b) => b.id === click.board)) {
        notifyFailure("That dashboard no longer exists", "Choose another one in the chart's settings.");
        return;
      }
      // BI-9: a bucket reaches another dashboard as its date range.
      if (grain) {
        const range = bucketRangeFilter(click.column, grain, value);
        if (!range) {
          notifyFailure("Cannot open the dashboard from this bucket", "Only a day, week, month, quarter or year can set a date range.");
          return;
        }
        router.push(dashboardRangeDestination(click.board, range));
        return;
      }
      router.push(dashboardDestination(click.board, click.column, value));
    } else if (click.kind === "query") {
      router.push(queryDestination(click.id));
    } else {
      const dest = urlDestination(click.url, value);
      if (!dest) {
        notifyFailure("This chart's link is not allowed", "Only https, http or a path in the console can be opened.");
      } else if (dest.external) {
        window.open(dest.href, "_blank", "noopener,noreferrer");
      } else {
        router.push(dest.href);
      }
    }
  };

  // The chart's click, mapped to the column and stored value it stands for
  // (lib/chart-click.ts); a click that stands for none does nothing. A saved
  // destination replaces the drill menu unless the viewer set it aside.
  const onChartClick = (spec: ChartCard, clickSpec: ClickSpec): ChartClickHandler => (hit, pos, ctx) => {
    const target = drillTarget(clickSpec, hit, ctx);
    if (!target) return;
    // BI-9: a part of the date (a weekday) spans many days: no rows to list,
    // no range to filter. The click says so instead of doing nothing silently.
    if (target.grain && !isTruncation(target.grain)) {
      notifyInfo("Nothing to open from this value", PART_NO_CLICK_REASON);
      return;
    }
    const saved = spec.def?.click;
    if (saved && !menuTiles.has(spec.id)) {
      followClick(saved, target.value, target.grain);
      return;
    }
    const bin = target.bin ? { width: target.bin.width, last: isLastBin(Number(target.value), target.bin.width, target.bin.max) } : undefined;
    // BI-13: a mark of one of the chart's other questions lists that question's own rows and
    // filters the dashboard by that question's dimension; the server needs the chart and the question's index.
    const layer = ctx.layer;
    setDrill({
      tileId: spec.id, name: target.value, label: target.label, grain: target.grain, bin,
      column: layer ? layer.dimension : target.column,
      mart: layer ? layer.mart : spec.mart, sqlSource: layer ? layer.sqlSource : spec.sqlSource,
      chart: layer ? spec.id : ownFilterChart(spec), layer: layer?.index,
      x: pos.x, y: pos.y, builtin: spec.source === "builtin",
    });
  };
  const toggleMenuTile = (id: string) => setMenuTiles((prev) => {
    const next = new Set(prev);
    if (!next.delete(id)) next.add(id);
    return next;
  });

  // The sort a viewer chose on a raw table, by tile, so its export follows the tile.
  const tableSorts = React.useRef<Record<string, { column: string; dir: "asc" | "desc" }>>({});
  // BI-1: what the page sent to read the dashboard, sent again with a download, so the file holds what is on screen.
  const readState = (): ReadState => ({ filters, params: paramsQuery(data?.appliedParams ?? null), grain: grainQuery(gChoice) });
  const canDownload = data?.canDownload !== false;
  const [pdfOpen, setPdfOpen] = React.useState(false);
  const [subsOpen, setSubsOpen] = React.useState(false);
  const [dataFor, setDataFor] = React.useState<{ id: string; title: string } | null>(null);
  async function downloadPng(id: string) {
    try {
      const fileName = await downloadChartPng(id, readState());
      notifySuccess("Image downloaded", fileName);
    } catch (err) {
      notifyFailure("The image could not be made", err instanceof Error ? err.message : String(err));
    }
  }

  async function downloadFile() {
    try {
      const fileName = await downloadDashboardFile(board);
      notifySuccess("Dashboard file downloaded", fileName);
    } catch (err) {
      notifyFailure("The dashboard file could not be made", err instanceof Error ? err.message : String(err));
    }
  }

  async function downloadData(id: string, choice: DataChoice) {
    const done = await downloadChartData(id, choice, readState(), tableSorts.current[id]);
    const note = exportNotice(done);
    if (done.cut) notifyInfo(note.message, note.description); else notifySuccess(note.message);
  }

  // Build the tiles for the grid.
  // "Move to <tab>" for a tile, in edit mode of a dashboard with several tabs.
  const moveItems = (id: string): TileMenuItem[] => {
    if (!edit || !canWrite || tabs.length < 2) return [];
    const here = tabOf(tabs, layout, id);
    return tabs.filter((t) => t.id !== here).map((t, i) => ({
      label: `Move to ${t.title}`, icon: <ArrowRightLeft />, separatorBefore: i === 0,
      onSelect: () => persistLayout(moveTileToTab(tabs, shownLayout, chartIds, id, t.id)),
    }));
  };
  const items: GridItem[] = charts.map((spec) => {
    if (isSectionKind(spec.kind)) {
      return {
        id: spec.id, kind: spec.kind, span: spec.span, title: spec.title, body: null,
        menu: edit && canWrite ? [
          ...(spec.kind !== "divider" ? [{ label: "Edit text", icon: <Pencil />, onSelect: () => setLineDialog({ mode: "edit", id: spec.id, kind: spec.kind }) }] : []),
          ...moveItems(spec.id),
          { label: "Delete", icon: <Trash2 />, destructive: true, separatorBefore: true, onSelect: () => setRemoving({ id: spec.id, title: spec.title }) },
        ] : undefined,
      };
    }
    const cell = data?.results[spec.id];
    const badge = SOURCE_BADGE[spec.source];
    const drilled = tileDrills.drills[spec.id];
    const level = drilled ? tileDrills.currentLevel(spec) : undefined;
    const clickSpec = clickSpecOf(spec, level);
    const drillable = !edit && canDrill(clickSpec);
    const own = spec.source !== "builtin";
    const downloadKind = downloadMenuKind({ own, hasRows: hasRows(cell) && cell.rows.length > 0 }, canDownload);
    const write = chartWriteItems({
      canWrite, own, hasDef: Boolean(spec.def),
      onEdit: () => setEditing({ id: spec.id, def: spec.def as ChartDef }),
      onDelete: () => setRemoving({ id: spec.id, title: spec.title }),
    });
    const menu: TileMenuItem[] = [
      { label: "Expand", icon: <Maximize2 />, onSelect: () => setTileDialog({ kind: "expand", id: spec.id }) },
      { label: "View data", icon: <Table2 />, onSelect: () => setTileDialog({ kind: "data", id: spec.id }) },
      // BI-4 part B (feature decision 6): this tile is computed again and replaces what is remembered; the others stay.
      ...(hasRows(cell) && cell.computedAt ? [{ label: "Refresh this tile", icon: <RefreshCw />, onSelect: () => void load(spec.id) }] : []),
      // The drill menu stays reachable on a chart that opens somewhere else.
      ...(spec.def?.click && drillable
        ? [{ label: menuTiles.has(spec.id) ? "Click opens its destination" : "Click opens the drill menu", icon: <MousePointerClick />, onSelect: () => toggleMenuTile(spec.id) }]
        : []),
      // A KPI, gauge or table has no mark to click: its rows are one step away here.
      ...((offersTileRecords(spec.kind) || spec.def?.steps?.length) && (spec.mart || spec.sqlSource)
        ? [{ label: "View records", icon: <Table2 />, onSelect: () => openRecords({ title: spec.title, mart: spec.mart, sqlSource: spec.sqlSource, chart: ownFilterChart(spec) }, spec.source === "builtin", NO_FILTERS, spec.id) }]
        : []),
      // BI-14 (decision 1, Q6): everyone who sees a stored chart gets the item; the panel says what a person without `lineage:read` may do.
      ...(own ? [{ label: "Where does this come from", icon: <GitFork />, onSelect: () => setLineageChart({ id: spec.id, title: spec.title }) }] : []),
      // BI-19 part C (C3): everyone who can view a stored chart may alert on it
      // for themselves; the server decides who may send it elsewhere.
      ...(own && !edit ? [{ label: "Alert me…", icon: <Bell />, onSelect: () => setAlertChart({ id: spec.id, title: spec.title, kind: spec.kind, measures: hasRows(cell) ? cell.columns.filter((c) => c !== spec.def?.dimension) : [] }) }] : []),
      // BI-9: a chart grouped by a part of the date (a weekday) has no rows to
      // list or range to filter; the menu says why instead of leaving the click dead.
      ...(hasRows(cell) && isGrain(cell.grain) && !isTruncation(cell.grain)
        ? [{ label: "Why clicking does nothing", icon: <MousePointerClick />, onSelect: () => notifyInfo("Nothing to open from this value", PART_NO_CLICK_REASON) }]
        : []),
      // BI-1: a chart of the board downloads its whole result from the server, as
      // Excel or CSV, formatted or raw. A built-in tile has no stored definition to
      // read and keeps the rows on screen as CSV.
      ...(downloadKind === "data"
        ? [
            { label: "Download data…", icon: <Download />, onSelect: () => setDataFor({ id: spec.id, title: spec.title }) },
            // BI-19 part D (plan D13): the chart as a picture, drawn by the same renderer as the PDF, so only where there is one.
            ...(data?.pdfSupported === true
              ? [{ label: "Download PNG", icon: <ImageDown />, onSelect: () => void downloadPng(spec.id) }]
              : []),
          ]
        : downloadKind === "csv"
          ? [{ label: "Download CSV", icon: <Download />, onSelect: () => hasRows(cell) && downloadRowsCsv(spec.title, cell) }]
          : []),
      ...write.edit,
      ...moveItems(spec.id),
      ...write.remove,
    ];
    return {
      id: spec.id,
      span: spec.span,
      title: spec.title,
      subtitle: spec.subtitle,
      hint: (
        <>
          {/* BI-6: the chart's own filters, listed and not removable here. */}
          {spec.def?.filters?.length ? <OwnFiltersMarker filters={spec.def.filters} /> : null}
          {hasRows(cell) && cell.filtersSkipped?.length ? <SkippedFiltersMarker skipped={cell.filtersSkipped} /> : null}
          {/* A pivot says its own cut-off in its footer (the cell cap, not the latest buckets). */}
          {hasRows(cell) && (cell.grainSkipped || (cell.truncated && spec.kind !== "pivot"))
            ? <GrainMarkers skipped={cell.grainSkipped} truncated={cell.truncated && spec.kind !== "pivot"} limit={typeof spec.def?.limit === "number" ? spec.def.limit : undefined} /> : null}
          {drillable ? (
        <Tooltip>
          <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
            <MousePointerClick className="size-3.5" aria-label="Clickable chart" />
          </TooltipTrigger>
          <TooltipContent>
            {spec.source === "builtin"
              ? "Click a value in the chart to see its records"
              : spec.def?.click && !menuTiles.has(spec.id)
                ? `Click a value in the chart to ${clickVerb(spec.def.click, boards)}`
                : "Click a value in the chart to filter the dashboard or see its records"}
          </TooltipContent>
        </Tooltip>
          ) : null}
        </>
      ),
      badge: (
        <div className="flex items-center gap-1.5">
          {badge ? (
            <span className={cn("inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium", badge.cls)}>
              {spec.source === "ai" ? <Sparkles className="size-2.5" /> : null}{badge.label}
            </span>
          ) : null}
          {/* The source table is detail for whoever arranges the board. */}
          {edit ? <span className="hidden rounded-full border px-2 py-0.5 font-mono text-[10px] text-muted-foreground sm:inline">{spec.sqlSource ? "SQL source" : spec.mart}</span> : null}
        </div>
      ),
      menuLabel: spec.sqlSource ? "Source: SQL source" : spec.mart ? `Source: ${spec.mart}` : undefined,
      menu,
      body: drilled ? (
        <div className="flex h-full min-h-0 flex-col">
          <DrillTrail steps={drilled.steps} onClimb={(index) => tileDrills.climb(spec, index)} />
          <div className="min-h-0 flex-1">
            <TileBody spec={{ ...spec, x: level ?? spec.x, def: { ...spec.def, dimension: level } }} cell={drilled.cell} dark={dark} loading={loading || drilled.loading}
              onDataClick={drillable ? onChartClick(spec, clickSpec) : undefined} />
          </div>
        </div>
      ) : (
        <TileBody spec={spec} cell={cell} dark={dark} loading={loading} paging={{ filters: filtersOnTab(filters, tabOf(tabs, layout, spec.id)) }} onTableSort={(sort) => { tableSorts.current[spec.id] = sort; }}
          onDataClick={drillable ? onChartClick(spec, clickSpec) : undefined} />
      ),
    };
  });

  const dialogSpec = tileDialog ? charts.find((c) => c.id === tileDialog.id) : undefined;
  const drillChart = drill?.tileId ? charts.find((c) => c.id === drill.tileId) : undefined;
  const drillOffer = drillChart ? tileDrills.offer(drillChart) : null;
  const filterFields = data?.filterFields ?? NO_FIELDS;
  const showFilterBar = filterFields.length > 0 || filters.length > 0;

  // `dark` on the container themes the dashboard alone (the tokens and the
  // `dark:` variants key off an ancestor with the class); the rest of the
  // console, and anything portalled out of this element, keeps its theme.
  return (
    <ReportingProvider reporting={data?.reporting}>
    <div className={cn("flex flex-col gap-4", fullscreen && "fixed inset-0 z-40 overflow-auto bg-background p-4 sm:p-6", look.className || (fullscreen && fsDark && "dark"), look.className && !fullscreen && "rounded-lg p-4")} style={look.style}>
      <PageHeader
        title={
          <BoardSwitcher
            boards={boards}
            activeId={board}
            activeName={dashName}
            onSelect={(id) => router.push(`/dashboards/${id}`)}
            onCreate={() => void createDashboard()}
            folders={folders}
            badge={ownerId ? <PrivateBadge /> : undefined}
          />
        }
        actions={
          <span data-print-hide className="contents">
            {fullscreen ? (
              <Button variant="outline" size="sm" aria-pressed={fsDark} onClick={toggleFsDark}>
                <Moon className="size-4" /> Dark
              </Button>
            ) : null}
            <EditLayoutButton canWrite={canWrite} edit={edit} onToggle={() => setEdit((e) => !e)} />
            {!isDefault ? (
              <FavouriteButton
                on={isFavourite}
                name={dashName}
                onToggle={() => {
                  void dashboardService.setFavourite("board", board, !isFavourite).then(reloadShortcuts).catch(() => undefined);
                }}
              />
            ) : null}
            {loading ? (
              <Button size="sm" variant="outline" onClick={stopLoading}>
                <Square className="size-4" aria-hidden /> Stop
              </Button>
            ) : null}
            <DashboardActionsMenu
              isDefault={isDefault}
              canWrite={canWrite}
              canManage={canManage}
              isPrivate={Boolean(ownerId)}
              onHistory={() => setHistoryOpen(true)}
              onTheme={canWrite ? () => { setThemeError(null); setThemeOpen(true); } : undefined}
              onFreshness={canWrite ? () => { setFreshError(null); setFreshOpen(true); } : undefined}
              loading={loading}
              fullscreen={fullscreen}
              autoSec={String(autoSec)}
              savedSec={String(savedSec)}
              canSaveRefresh={refreshIsSaveable({ mayWrite: canWrite, saved: savedSec, effective: autoSec })}
              savingRefresh={savingRefresh}
              onRefresh={() => void load("1")}
              onToggleFullscreen={toggleFullscreen}
              onAutoSec={(v) => setSessionSec(Number(v))}
              onSaveRefresh={() => void saveRefreshDefault()}
              onRename={() => setRenameOpen(true)}
              onShare={() => setShareOpen(true)}
              onExportPdf={exportPdf}
              onDownloadPdf={() => setPdfOpen(true)}
              onExportFile={() => void downloadFile()}
              onSubscriptions={() => setSubsOpen(true)}
              pdfSupported={data?.pdfSupported === true}
              canDownload={canDownload}
              onDuplicate={() => void duplicateDashboard()}
              onDelete={() => void deleteDashboard()}
            />
            {/* BI-19 part E: anyone who can see the dashboard can read and write comments; the server checks. */}
            {!isDefault ? (
              <Button size="sm" variant="outline" onClick={() => setCommentsOpen(true)}>
                <MessageSquare className="size-4" /> Comments
              </Button>
            ) : null}
            {hasPermission("dashboard:write") ? (
              <Button size="sm" variant="outline" onClick={() => setParamsOpen(true)}>
                <SlidersHorizontal className="size-4" /> Parameters
              </Button>
            ) : null}
            <NewChartButton canWrite={canWrite} onClick={() => setNewChartOpen(true)} />
          </span>
        }
      />

      {error ? <div className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">{readableErrorMessage(error)}</div> : null}
      {data?.storeError ? <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-2 text-xs text-amber-600 dark:text-amber-400">Could not load saved charts: {data.storeError}</div> : null}

      {showFilterBar || grainedCharts.length > 0 || paramControls.length > 0 ? (
        <FilterRow count={filters.filter(isActiveFilter).length + (pValues ? Object.keys(pValues).length : 0)}>
          {grainedCharts.length > 0 ? (
            <GrainSwitch
              choices={grainChoices}
              value={shownGrainValue}
              onChange={(next) => writeGrain(next === "" ? "own" : next)}
            />
          ) : null}
          <ParamControls controls={paramControls} onChange={changeParam} />
          <FilterBar
            board={board}
            fields={filterFields}
            filters={filters}
            savedFilters={defaultFilters}
            onChange={applyFilters}
            dirty={rowDirty}
            canSaveDefault={!isDefault && canWrite}
            tabs={tabs}
            saving={savingDefault}
            onSaveDefault={() => void saveRowDefault()}
            onReset={resetRow}
          />
        </FilterRow>
      ) : null}

      {/* KPI row (builtin dashboard) */}
      {kpis.length ? (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          {kpis.map((k) => {
            const cell = data?.results[k.id];
            const v = hasRows(cell) ? Number(cell.rows[0]?.v ?? 0) : null;
            return (
              <div key={k.id} className="rounded-xl border bg-card p-4 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
                <p className="text-xs font-medium text-muted-foreground">{k.title}</p>
                {loading && v === null ? <div className="mt-2 h-7 w-24 animate-pulse rounded bg-muted" /> : <p className="mt-1 text-2xl font-semibold tabular-nums text-foreground">{v === null ? "—" : fmtInt(v)}</p>}
                {k.caption ? <p className="mt-1 text-[11px] text-muted-foreground">{k.caption}</p> : null}
              </div>
            );
          })}
        </div>
      ) : null}

      {/* Canvas */}
      {stopped ? (
        <p role="status" className="text-sm text-muted-foreground">{STOPPED_SENTENCE}. Press Refresh to load the dashboard again.</p>
      ) : null}
      {loading && charts.length === 0 ? (
        <DashboardTilesSkeleton />
      ) : !loading && charts.length === 0 && !stopped ? (
        <Empty className="border border-dashed">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ChartColumn />
            </EmptyMedia>
            <EmptyTitle>No charts yet</EmptyTitle>
            <EmptyDescription>
              Build one yourself, or ask AI Copilot — for example “a bar chart of foreign visitors by region”.
            </EmptyDescription>
          </EmptyHeader>
          <EmptyContent className="flex-row justify-center gap-2">
            <NewChartButton canWrite={canWrite} onClick={() => setNewChartOpen(true)} />
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setCopilotMode("build");
                setCopilotExpanded(true);
              }}
            >
              <Sparkles className="size-4" /> Ask Copilot
            </Button>
          </EmptyContent>
        </Empty>
      ) : (
        <>
          {edit ? (
            <div className="flex flex-wrap items-center gap-2 rounded-lg border border-dashed border-primary/30 bg-primary/5 px-3 py-1.5 text-xs">
              <Move className="size-3.5 text-primary" />
              {/* BI-18 part C review fix (SHOULD-FIX) R2: the muted colour is the hint's alone; on the bar it was inherited by the buttons and made them look disabled. */}
              <span className="min-w-0 flex-1 text-muted-foreground">
                {phoneEdit
                  ? <>Phone layout: <span className="font-medium text-foreground">move tiles up or down</span> and change their height. Saved automatically.</>
                  : <>Edit mode: <span className="font-medium text-foreground">drag the header</span> to move, <span className="font-medium text-foreground">drag the bottom-right corner</span> to resize. Saved automatically.</>}
              </span>
              {canWrite && !isDefault && !phoneEdit ? (
                <DropdownMenu>
                  <DropdownMenuTrigger render={<Button variant="outline" size="sm" />}><Plus className="size-3.5" /> Add</DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-40">
                    <DropdownMenuItem onClick={() => setLineDialog({ mode: "add", kind: "heading" })}><Heading className="size-4" /> Heading</DropdownMenuItem>
                    <DropdownMenuItem onClick={() => setLineDialog({ mode: "add", kind: "section" })}><Rows3 className="size-4" /> Section</DropdownMenuItem>
                    <DropdownMenuItem onClick={() => void addSectionTile("divider", "")}><Minus className="size-4" /> Divider</DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              ) : null}
              {canWrite ? (
                <Button variant={phoneEdit ? "default" : "outline"} size="sm" aria-pressed={phoneEdit} onClick={() => setPhoneEdit((v) => !v)}>
                  <Smartphone className="size-3.5" /> {phoneEdit ? "Done with phone layout" : "Edit phone layout"}
                </Button>
              ) : null}
              {phoneEdit && phone?.length ? <Button variant="ghost" size="sm" onClick={() => void resetPhone()}>Use the computed order</Button> : null}
            </div>
          ) : null}
          <TabBar tabs={tabs} selected={selTab} onSelect={selectTab} editing={tabEditing} />
          <DashboardGrid
            items={items} layout={shownLayout} editable={edit && !phoneEdit} onLayoutChange={persistLayout}
            tabs={tabs} tab={selTab} phone={phone} phoneEdit={edit && phoneEdit} onPhoneChange={persistPhone}
            foldKey={board}
          />
          {tabs.length > 0 && selTab && tilesOnTab(tabs, layout, chartIds, selTab).length === 0 ? (
            // BI-18 part C review fix (SHOULD-FIX) R3: the page's own empty state.
            <Empty className="border border-dashed">
              <EmptyHeader>
                <EmptyTitle>No tiles on this tab</EmptyTitle>
                {edit ? <EmptyDescription>Use “Move to” in a tile&apos;s menu on another tab.</EmptyDescription> : null}
              </EmptyHeader>
            </Empty>
          ) : null}
        </>
      )}

      {lineDialog ? (
        <LineDialog
          title={lineDialog.mode === "add" ? (lineDialog.kind === "section" ? "New section" : "New heading") : "Edit text"}
          initial={lineDialog.mode === "edit" ? (charts.find((c) => c.id === lineDialog.id)?.text ?? charts.find((c) => c.id === lineDialog.id)?.title ?? "") : ""}
          confirm={lineDialog.mode === "add" ? "Add" : "Save"}
          label="Heading text"
          maxLength={SECTION_TEXT_MAX}
          onClose={() => setLineDialog(null)}
          onSave={(text) => {
            const d = lineDialog;
            setLineDialog(null);
            if (d.mode === "add") void addSectionTile(d.kind, text); else void editSectionText(d.id, d.kind, text);
          }}
        />
      ) : null}
      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(o) => { if (!o) setRemoving(null); }}
        title="Delete chart?"
        description={`"${removing?.title ?? ""}" goes to the trash. You can restore it for 30 days.`}
        confirmLabel="Delete chart"
        destructive
        confirming={removeBusy}
        onConfirm={() => { if (removing) void removeChart(removing.id); }}
      />

      {paramsOpen ? (
        <ParamDialog
          open
          onOpenChange={setParamsOpen}
          canWrite={hasPermission("dashboard:write")}
          shown={paramControls.map((c) => c.param.name)}
          onToggleShown={(name, on) => void toggleShownParam(name, on)}
          onChanged={() => void load()}
        />
      ) : null}
      {editing ? (
        <ChartBuilder hideTrigger open={!!editing} onOpenChange={(o) => { if (!o) setEditing(null); }}
          editId={editing.id} initial={editing.def} board={board} boards={boards} paramValues={data?.appliedParams}
          onSaved={() => { setEditing(null); void load(); }} />
      ) : null}
      {newChartOpen ? (
        <ChartBuilder hideTrigger open onOpenChange={setNewChartOpen}
          board={board} boards={boards} paramValues={data?.appliedParams}
          onSaved={() => { setNewChartOpen(false); void loadAndPlace(); }} />
      ) : null}

      {drill ? (
        <DrillMenu
          drill={drill}
          onClose={() => setDrill(null)}
          onFilter={
            drill.builtin ? undefined
              : drill.bin
                ? () => crossFilterBin(drill.column, Number(drill.name), drill.bin as { width: number; last: boolean })
              : drill.grain && isGrain(drill.grain)
                ? (bucketActions(drill.grain).filter ? () => crossFilterBucket(drill.column, drill.grain as Grain, drill.name) : undefined)
                : () => crossFilter(drill.column, drill.name)
          }
          onAddNote={edit && canWrite && !drill.builtin && drill.tileId && drillChart && NOTE_KINDS.includes(drillChart.kind) && drill.layer === undefined
            ? () => { setNoteError(null); setNoteFor({ tileId: drill.tileId as string, point: drill.label ?? drill.name }); setDrill(null); }
            : undefined}
          onRecords={() => openRecords({ title: drill.label ?? drill.name, mart: drill.mart, sqlSource: drill.sqlSource, column: drill.column, value: drill.name, grain: drill.grain, binWidth: drill.bin?.width, chart: drill.chart, layer: drill.layer }, drill.builtin, drillChart && drill.layer === undefined ? tileDrills.fixed(drillChart) : NO_FILTERS, drill.tileId)}
          drillDown={drillOffer && drill.layer === undefined ? ("reason" in drillOffer ? drillOffer : { label: `Drill down to ${drillOffer.to}`, onSelect: () => { if (drillChart) tileDrills.down(drillChart, drillOffer.hierarchy, drill.name, drill.label); setDrill(null); } }) : undefined}
        />
      ) : null}
      <RecordsDialog request={records} onClose={() => setRecords(null)} />
      {noteFor ? <NoteDialog point={noteFor.point} saving={savingNote} error={noteError} onSave={(t) => void saveNote(t)} onClose={() => setNoteFor(null)} /> : null}

      {tileDialog?.kind === "data" && dialogSpec ? (
        <TileDataDialog title={dialogSpec.title} cell={data?.results[dialogSpec.id]} onClose={() => setTileDialog(null)} canDownload={canDownload} />
      ) : null}
      {alertChart ? (
        <ChartAlertDialog
          open
          onOpenChange={(open) => { if (!open) setAlertChart(null); }}
          chart={alertChart}
          filters={normalizeFilters(filters)}
          userEmail={user?.email ?? null}
          canWriteAlerts={hasPermission("alert:write")}
        />
      ) : null}
      {tileDialog?.kind === "expand" && dialogSpec ? (
        <TileExpandDialog spec={dialogSpec} cell={data?.results[dialogSpec.id]} dark={dark} onClose={() => setTileDialog(null)} />
      ) : null}

      {historyOpen ? (
        <VersionHistoryDialog
          boardId={board}
          canWrite={canWrite}
          onOpenChange={setHistoryOpen}
          onReverted={() => { notifyDashboardsChanged(); void load(); }}
        />
      ) : null}
      {!isDefault ? <CommentsDialog board={board} dashName={dashName} open={commentsOpen} onOpenChange={setCommentsOpen} /> : null}
      {!isDefault ? <ShareDialog board={board} dashName={dashName} open={shareOpen && canManage} onOpenChange={setShareOpen} /> : null}
      {themeOpen ? (
        <ThemeFontDialog open={themeOpen} onOpenChange={setThemeOpen} theme={data?.theme} font={data?.font}
          allowedFonts={data?.reporting?.fonts ?? SHIPPED_FONT_IDS} saving={savingTheme} error={themeError} onSave={(c) => void saveThemeAndFont(c)} />
      ) : null}
      {freshOpen ? (
        <FreshnessDialog open={freshOpen} onOpenChange={setFreshOpen} rule={data?.boardCacheRule} effective={data?.cacheRule}
          saving={savingFresh} error={freshError} onSave={(r) => void saveFreshness(r)} />
      ) : null}
      {renameOpen ? (
        <RenameDashboardDialog open={renameOpen} onOpenChange={setRenameOpen} currentName={dashName} onSave={(n) => void renameDashboard(n)} />
      ) : null}
      {subsOpen ? (
        <SubscriptionsDialog
          open={subsOpen}
          onOpenChange={setSubsOpen}
          board={board}
          dashName={dashName}
          charts={(data?.charts ?? []).filter((c) => c.def && !isStaticKind(c.kind)).map((c) => ({ id: c.id, title: c.title, grouped: !isSingleValueChart(c.kind, c.def) }))}
          readState={readState()}
          reportZone={data?.reporting?.timeZone ?? "UTC"}
        />
      ) : null}
      {pdfOpen ? (
        <DownloadPdfDialog
          open={pdfOpen}
          onOpenChange={setPdfOpen}
          onDownload={async (choice: PdfChoice) => {
            const fileName = await downloadDashboardPdf(board, choice, readState());
            notifySuccess("PDF downloaded", fileName);
          }}
        />
      ) : null}
      {dataFor ? (
        <DownloadDataDialog
          open
          onOpenChange={(next) => { if (!next) setDataFor(null); }}
          title={dataFor.title}
          onDownload={(choice) => downloadData(dataFor.id, choice)}
        />
      ) : null}
      <ChartLineagePanel chart={lineageChart} onOpenChange={(next) => { if (!next) setLineageChart(null); }} />
    </div>
    </ReportingProvider>
  );
}
