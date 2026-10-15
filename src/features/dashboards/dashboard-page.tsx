"use client";

import * as React from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useTheme } from "next-themes";
import { ChartColumn, Download, Eye, Maximize2, MousePointerClick, Move, Moon, Pencil, Sparkles, Table2, Trash2 } from "lucide-react";
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { PageHeader } from "@/components/patterns/page-header";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@rantai/design-system/ui/empty";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { dashboardDestination, queryDestination, urlDestination } from "@/lib/click-destination";
import { bucketActions, bucketRangeFilter, dashboardRangeDestination, toggleBucketFilter } from "@/lib/bucket-click";
import { GRAIN_PARAM, differsFromSaved, grainQuery, readGrainParam, savedGrainBody, shownGrain, withGrainParam, type GrainChoice } from "@/lib/grain-switch";
import { PART_NO_CLICK_REASON, isGrain, isTruncation, switchChoices, type ColumnKind, type Grain, type ReportingContext } from "@/lib/time-grain";
import { effectiveRefresh, canSaveRefresh as refreshIsSaveable, listedInterval } from "@/lib/dashboard-refresh";
import { enterFullscreen, leaveFullscreen, readFullscreenDark, writeFullscreenDark } from "@/lib/dashboard-fullscreen";
import { notifyFailure, notifyInfo, notifySuccess } from "@/lib/notify";
import { summarizeFilters, summarizeTiles } from "@/lib/page-context-summary";
import { FILTER_PARAM, decodeFilters, dropInertFilters, enforceRequired, filtersEqual, filtersToParam, normalizeFilters, toggleValue } from "@/lib/dashboard-filter-state";
import { canDrill, drillTarget, offersTileRecords, type ChartClickHandler, type ClickSpec } from "@/lib/chart-click";
import type { ChartRenderSpec, ChartSource } from "@/lib/dashboard-specs";
import type { ChartClick, LayoutMap, FilterDef, FilterField } from "@/services/clients/bi-store";
import { useAuth } from "@/features/auth/auth-provider";
import { useCopilot } from "@/features/copilot/use-copilot";
import { useService } from "@/hooks/use-service";
import { dashboardService } from "@/services";
import { apiFetch } from "@/services/http";
import { useAutoRefresh } from "./auto-refresh";
import { BoardSwitcher } from "./board-switcher";
import { ChartBuilder, type ChartDef } from "./chart-builder";
import { fmtInt } from "./chart-option";
import { DashboardActionsMenu, RenameDashboardDialog } from "./dashboard-actions";
import { notifyDashboardsChanged, useDashboardsChanged } from "./dashboard-events";
import { GrainMarkers } from "./grain-marker";
import { GrainSwitch } from "./grain-switch";
import { ReportingProvider } from "./reporting-context";
import { FilterBar } from "./filters/filter-bar";
import { SkippedFiltersMarker } from "./filters/skipped-marker";
import { DashboardGrid, type GridItem, type TileMenuItem } from "./dashboard-grid";
import { DashboardTilesSkeleton } from "./dashboard-skeleton";
import { DrillMenu, RecordsDialog, type DrillTarget } from "./drill";
import type { RecordsRequest } from "./records";
import { forgetLastBoard, rememberLastBoard } from "./last-board";
import { ShareDialog } from "./share-dialog";
import { TileBody } from "./tile-body";
import { downloadTableExport, exportNotice } from "./table-export";
import { TileDataDialog, TileExpandDialog, downloadRowsCsv, hasRows, type Cell } from "./tile-dialogs";

type KpiMeta = { id: string; title: string; caption?: string; format: string };
type BoardOpt = { id: string; name: string; folderId?: string | null };
type ChartCard = ChartRenderSpec & { board?: string; def?: ChartDef };
type Payload = {
  board: string; layout: LayoutMap;
  /** The filters in force for this response (the address's, else the default). */
  filters: FilterDef[];
  /** The board's saved default, which `filters` is not while the address overrides it. */
  defaultFilters?: FilterDef[];
  /** The board's saved auto-refresh in seconds; 0 when none (BI-18·B). */
  refreshSeconds?: number;
  /** The grain the board saved ("" when none), and the one this response used (BI-9). */
  grain?: string; appliedGrain?: string | null;
  /** The zone and first weekday the buckets were cut with (BI-9). */
  reporting?: ReportingContext;
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
function clickSpecOf(spec: ChartCard): ClickSpec {
  const own = spec.source !== "builtin";
  return {
    kind: spec.kind,
    source: spec.source,
    dimension: (own ? spec.def?.dimension : spec.x) || undefined,
    breakdown: (own ? spec.def?.breakdown : spec.series) || undefined,
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
  // Filters are temporary and live in the address (`?f=`); the board's saved
  // default applies while the address carries none. Nothing is written to the
  // board until an editor presses "Save as default".
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { hasPermission } = useAuth();
  const fParam = searchParams.get(FILTER_PARAM);
  // The dashboard's grain switch lives in the address beside the filters.
  const gChoice: GrainChoice = readGrainParam(searchParams.get(GRAIN_PARAM));
  const [savingDefault, setSavingDefault] = React.useState(false);
  const [editing, setEditing] = React.useState<{ id: string; def: ChartDef } | null>(null);
  // Tile delete is one click away, so it asks first.
  const [removing, setRemoving] = React.useState<{ id: string; title: string } | null>(null);
  const [removeBusy, setRemoveBusy] = React.useState(false);
  const [renameOpen, setRenameOpen] = React.useState(false);
  // Folders only group the title switcher here. Managing them and filing
  // a dashboard are on the dashboard list (`dashboard-list-page.tsx`).
  const foldersState = useService((signal) => dashboardService.listFolders(signal), []);
  const folders = foldersState.data ?? [];
  const [shareOpen, setShareOpen] = React.useState(false);
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
  const dark = themeDark || (fullscreen && fsDark);
  // Drill / cross-filter: menu on data-point click + a modal of raw rows.
  const [drill, setDrill] = React.useState<(DrillTarget & { builtin: boolean }) | null>(null);
  const [records, setRecords] = React.useState<RecordsRequest | null>(null);
  // Tiles whose saved click destination the viewer set aside for the drill menu (this session only).
  const [menuTiles, setMenuTiles] = React.useState<ReadonlySet<string>>(new Set());
  const [tileDialog, setTileDialog] = React.useState<{ kind: "data" | "expand"; id: string } | null>(null);
  // Only the newest load may write state. Creating a dashboard fires a
  // reload of the board being left and then navigates to the new one; the
  // old board's response (slower: it runs the built-in tiles) used to land
  // last and paint its charts under the new dashboard's name.
  const loadSeq = React.useRef(0);
  const load = React.useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoading(true); setError(null);
    try {
      const q = new URLSearchParams({ board });
      // A parameter that is not a filter list is ignored: the saved default shows.
      const fromAddress = decodeFilters(fParam);
      if (fromAddress) q.set("filters", JSON.stringify(fromAddress));
      const grainAsked = grainQuery(gChoice);
      if (grainAsked) q.set("grain", grainAsked);
      const res = await apiFetch(`/api/dashboard?${q.toString()}`, { cache: "no-store" });
      const json = (await res.json()) as Payload;
      if (seq !== loadSeq.current) return;
      if (!res.ok) throw new Error((json as { error?: string }).error ?? "Failed to load dashboard");
      setData(json);
      setLayout(json.layout ?? {});
    } catch (e) {
      if (seq === loadSeq.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [board, fParam, gChoice]);

  // Switching dashboard starts in view mode; the switcher navigates without
  // `f`, so the new board opens on its own default.
  React.useEffect(() => { setEdit(false); }, [board]);
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
  useAutoRefresh(autoSec * 1000, load);
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
  const rowDirty = dirty || grainDirty;
  const resetRow = React.useCallback(() => {
    const q = new URLSearchParams(window.location.search);
    q.delete(FILTER_PARAM);
    q.delete(GRAIN_PARAM);
    const qs = q.toString();
    router.replace(qs ? `${pathname}?${qs}` : pathname, { scroll: false });
  }, [router, pathname]);
  const saveRowDefault = React.useCallback(async () => {
    setSavingDefault(true);
    try {
      const body: Record<string, unknown> = { id: board };
      if (dirty) body.filters = normalizeFilters(filters);
      if (grainDirty) body.grain = savedGrainBody(shownGrainValue);
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
  }, [board, dirty, grainDirty, filters, shownGrainValue, resetRow]);
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

  // Drill-down: the rows behind the clicked value (or behind a whole tile),
  // for the filters in force so the list agrees with the number clicked. A
  // built-in tile ignores dashboard filters, so its rows are not narrowed.
  const openRecords = React.useCallback((request: Omit<RecordsRequest, "filters">, builtin: boolean) => {
    setDrill(null);
    setRecords({ ...request, filters: builtin ? NO_FILTERS : filters });
  }, [filters]);

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
      void apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: board, layout: next }),
      });
    }, 600);
  }, [board]);

  async function removeChart(id: string) {
    setRemoveBusy(true);
    try {
      await apiFetch(`/api/dashboard/specs?id=${encodeURIComponent(id)}`, { method: "DELETE" });
      setRemoving(null);
      void load();
    } finally {
      setRemoveBusy(false);
    }
  }
  async function boardRequest(method: "POST" | "PUT", body: Record<string, unknown>) {
    const res = await apiFetch("/api/dashboard/boards", {
      method, headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
    });
    return (await res.json()) as { board?: { id: string } };
  }
  async function duplicateDashboard() {
    const json = await boardRequest("POST", { duplicate: board });
    notifyDashboardsChanged();
    if (json.board?.id) router.push(`/dashboards/${json.board.id}`);
  }
  async function createDashboard() {
    const json = await boardRequest("POST", { name: "New dashboard" });
    notifyDashboardsChanged();
    if (json.board?.id) router.push(`/dashboards/${json.board.id}`);
  }
  async function deleteDashboard() {
    if (isDefault) return;
    await apiFetch(`/api/dashboard/boards?id=${encodeURIComponent(board)}`, { method: "DELETE" });
    notifyDashboardsChanged();
    // Lupakan sebelum pergi, kalau tidak `/dashboards` menuntun balik ke sini.
    forgetLastBoard(board);
    router.push("/dashboards/browse");
  }
  async function renameDashboard(name: string) {
    if (isDefault) return;
    await boardRequest("PUT", { id: board, name });
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
  // Stable empty fallbacks: a fresh `[]` each render re-ran the Copilot
  // context effect, which re-rendered this page, forever, while loading.
  const kpis = data?.kpis ?? NO_KPIS;
  const charts = data?.charts ?? NO_CHARTS;
  const results = data?.results ?? NO_RESULTS;

  // Tell Copilot what is on this dashboard — the built-in one included.
  const { setPageContext, setMode: setCopilotMode, setExpanded: setCopilotExpanded } = useCopilot();
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
        `Active filters: ${summarizeFilters(filters)}. ` +
        `Tiles and the data they currently show (built-in tiles are not editable):\n${tiles || "none yet"}\n` +
        `When creating a chart use board="${board}". To change a tile you created, use update_chart with its id. ` +
        `You can also explain what the charts show.`,
    });
    return () => setPageContext(null);
  }, [board, dashName, charts, kpis, results, filters, setPageContext]);

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
    setDrill({ name: target.value, label: target.label, grain: target.grain, column: target.column, mart: spec.mart, sqlSource: spec.sqlSource, x: pos.x, y: pos.y, builtin: spec.source === "builtin" });
  };
  const toggleMenuTile = (id: string) => setMenuTiles((prev) => {
    const next = new Set(prev);
    if (!next.delete(id)) next.add(id);
    return next;
  });

  // The sort a viewer chose on a raw table, by tile, so its export follows the tile.
  const tableSorts = React.useRef<Record<string, { column: string; dir: "asc" | "desc" }>>({});
  async function exportRawTable(id: string) {
    try {
      const done = await downloadTableExport(id, filters, tableSorts.current[id]);
      const note = exportNotice(done);
      if (done.cut) notifyInfo(note.message, note.description); else notifySuccess(note.message);
    } catch (e) {
      notifyFailure("The export failed", e instanceof Error ? e.message : undefined);
    }
  }

  // Build the tiles for the grid.
  const items: GridItem[] = charts.map((spec) => {
    const cell = data?.results[spec.id];
    const badge = SOURCE_BADGE[spec.source];
    const clickSpec = clickSpecOf(spec);
    const drillable = !edit && canDrill(clickSpec);
    const own = spec.source !== "builtin";
    const menu: TileMenuItem[] = [
      { label: "Expand", icon: <Maximize2 />, onSelect: () => setTileDialog({ kind: "expand", id: spec.id }) },
      { label: "View data", icon: <Table2 />, onSelect: () => setTileDialog({ kind: "data", id: spec.id }) },
      // The drill menu stays reachable on a chart that opens somewhere else.
      ...(spec.def?.click && drillable
        ? [{ label: menuTiles.has(spec.id) ? "Click opens its destination" : "Click opens the drill menu", icon: <MousePointerClick />, onSelect: () => toggleMenuTile(spec.id) }]
        : []),
      // A KPI, gauge or table has no mark to click: its rows are one step away here.
      ...(offersTileRecords(spec.kind) && (spec.mart || spec.sqlSource)
        ? [{ label: "View records", icon: <Table2 />, onSelect: () => openRecords({ title: spec.title, mart: spec.mart, sqlSource: spec.sqlSource }, spec.source === "builtin") }]
        : []),
      // BI-9: a chart grouped by a part of the date (a weekday) has no rows to
      // list or range to filter; the menu says why instead of leaving the click dead.
      ...(hasRows(cell) && isGrain(cell.grain) && !isTruncation(cell.grain)
        ? [{ label: "Why clicking does nothing", icon: <MousePointerClick />, onSelect: () => notifyInfo("Nothing to open from this value", PART_NO_CLICK_REASON) }]
        : []),
      // BI-16A T8: a raw table exports its whole result (every row, the tile's
      // filters and sort), from the server; other tiles keep the rows on screen.
      ...(hasRows(cell) && cell.rows.length && own && spec.kind === "table" && spec.def?.tableMode === "rows"
        ? [{ label: "Download CSV (all rows)", icon: <Download />, onSelect: () => void exportRawTable(spec.id) }]
        : hasRows(cell) && cell.rows.length
          ? [{ label: "Download CSV", icon: <Download />, onSelect: () => downloadRowsCsv(spec.title, cell) }]
          : []),
      ...(own && spec.def
        ? [{ label: "Edit chart", icon: <Pencil />, separatorBefore: true, onSelect: () => setEditing({ id: spec.id, def: spec.def as ChartDef }) }]
        : []),
      ...(own
        ? [{ label: "Delete chart", icon: <Trash2 />, destructive: true, separatorBefore: !spec.def, onSelect: () => setRemoving({ id: spec.id, title: spec.title }) }]
        : []),
    ];
    return {
      id: spec.id,
      span: spec.span,
      title: spec.title,
      subtitle: spec.subtitle,
      hint: (
        <>
          {hasRows(cell) && cell.filtersSkipped?.length ? <SkippedFiltersMarker skipped={cell.filtersSkipped} /> : null}
          {/* A pivot says its own cut-off in its footer (the cell cap, not the latest buckets). */}
          {hasRows(cell) && (cell.grainSkipped || (cell.truncated && spec.kind !== "pivot"))
            ? <GrainMarkers skipped={cell.grainSkipped} truncated={cell.truncated && spec.kind !== "pivot"} limit={spec.def?.limit} /> : null}
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
      body: (
        <TileBody spec={spec} cell={cell} dark={dark} loading={loading} paging={{ filters }} onTableSort={(sort) => { tableSorts.current[spec.id] = sort; }}
          onDataClick={drillable ? onChartClick(spec, clickSpec) : undefined} />
      ),
    };
  });

  const dialogSpec = tileDialog ? charts.find((c) => c.id === tileDialog.id) : undefined;
  const filterFields = data?.filterFields ?? NO_FIELDS;
  const showFilterBar = filterFields.length > 0 || filters.length > 0;

  // `dark` on the container themes the dashboard alone (the tokens and the
  // `dark:` variants key off an ancestor with the class); the rest of the
  // console, and anything portalled out of this element, keeps its theme.
  return (
    <ReportingProvider reporting={data?.reporting}>
    <div className={cn("flex flex-col gap-4", fullscreen && "fixed inset-0 z-40 overflow-auto bg-background p-4 sm:p-6", fullscreen && fsDark && "dark")}>
      <PageHeader
        title={
          <BoardSwitcher
            boards={boards}
            activeId={board}
            activeName={dashName}
            onSelect={(id) => router.push(`/dashboards/${id}`)}
            onCreate={() => void createDashboard()}
            folders={folders}
          />

        }
        actions={
          <span data-print-hide className="contents">
            {fullscreen ? (
              <Button variant="outline" size="sm" aria-pressed={fsDark} onClick={toggleFsDark}>
                <Moon className="size-4" /> Dark
              </Button>
            ) : null}
            <Button variant={edit ? "default" : "outline"} size="sm" onClick={() => setEdit((e) => !e)}>
              {edit ? <Eye className="size-4" /> : <Pencil className="size-4" />}{edit ? "Done" : "Edit layout"}
            </Button>
            <DashboardActionsMenu
              isDefault={isDefault}
              loading={loading}
              fullscreen={fullscreen}
              autoSec={String(autoSec)}
              savedSec={String(savedSec)}
              canSaveRefresh={refreshIsSaveable({ mayWrite: hasPermission("dashboard:write"), saved: savedSec, effective: autoSec })}
              savingRefresh={savingRefresh}
              onRefresh={() => void load()}
              onToggleFullscreen={toggleFullscreen}
              onAutoSec={(v) => setSessionSec(Number(v))}
              onSaveRefresh={() => void saveRefreshDefault()}
              onRename={() => setRenameOpen(true)}
              onShare={() => setShareOpen(true)}
              onExportPdf={exportPdf}
              onDuplicate={() => void duplicateDashboard()}
              onDelete={() => void deleteDashboard()}
            />
            <Button size="sm" onClick={() => setNewChartOpen(true)}>
              <ChartColumn className="size-4" /> New chart
            </Button>
          </span>
        }
      />

      {error ? <div className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">{error}</div> : null}
      {data?.storeError ? <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-2 text-xs text-amber-600 dark:text-amber-400">Could not load saved charts: {data.storeError}</div> : null}

      {showFilterBar || grainedCharts.length > 0 ? (
        <div data-print-hide className="flex flex-wrap items-center gap-3 rounded-lg border border-border bg-card/50 px-3 py-2">
          {grainedCharts.length > 0 ? (
            <GrainSwitch
              choices={grainChoices}
              value={shownGrainValue}
              onChange={(next) => writeGrain(next === "" ? "own" : next)}
            />
          ) : null}
          <FilterBar
            board={board}
            fields={filterFields}
            filters={filters}
            savedFilters={defaultFilters}
            onChange={applyFilters}
            dirty={rowDirty}
            canSaveDefault={!isDefault && hasPermission("dashboard:write")}
            saving={savingDefault}
            onSaveDefault={() => void saveRowDefault()}
            onReset={resetRow}
          />
        </div>
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
      {loading && charts.length === 0 ? (
        <DashboardTilesSkeleton />
      ) : !loading && charts.length === 0 ? (
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
            <Button size="sm" onClick={() => setNewChartOpen(true)}>
              <ChartColumn className="size-4" /> New chart
            </Button>
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
            <div className="flex items-center gap-2 rounded-lg border border-dashed border-primary/30 bg-primary/5 px-3 py-1.5 text-xs text-muted-foreground">
              <Move className="size-3.5 text-primary" />
              Edit mode: <span className="font-medium text-foreground">drag the header</span> to move, <span className="font-medium text-foreground">drag the bottom-right corner</span> to resize. Saved automatically.
            </div>
          ) : null}
          <DashboardGrid items={items} layout={layout} editable={edit} onLayoutChange={persistLayout} />
        </>
      )}

      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(o) => { if (!o) setRemoving(null); }}
        title="Delete chart?"
        description={`"${removing?.title ?? ""}" will be removed from this dashboard.`}
        confirmLabel="Delete chart"
        destructive
        confirming={removeBusy}
        onConfirm={() => { if (removing) void removeChart(removing.id); }}
      />

      {editing ? (
        <ChartBuilder hideTrigger open={!!editing} onOpenChange={(o) => { if (!o) setEditing(null); }}
          editId={editing.id} initial={editing.def} board={board} boards={boards}
          onSaved={() => { setEditing(null); void load(); }} />
      ) : null}
      {newChartOpen ? (
        <ChartBuilder hideTrigger open onOpenChange={setNewChartOpen}
          board={board} boards={boards}
          onSaved={() => { setNewChartOpen(false); void load(); }} />
      ) : null}

      {drill ? (
        <DrillMenu
          drill={drill}
          onClose={() => setDrill(null)}
          onFilter={
            drill.builtin ? undefined
              : drill.grain && isGrain(drill.grain)
                ? (bucketActions(drill.grain).filter ? () => crossFilterBucket(drill.column, drill.grain as Grain, drill.name) : undefined)
                : () => crossFilter(drill.column, drill.name)
          }
          onRecords={() => openRecords({ title: drill.label ?? drill.name, mart: drill.mart, sqlSource: drill.sqlSource, column: drill.column, value: drill.name, grain: drill.grain }, drill.builtin)}
        />
      ) : null}
      <RecordsDialog request={records} onClose={() => setRecords(null)} />

      {tileDialog?.kind === "data" && dialogSpec ? (
        <TileDataDialog title={dialogSpec.title} cell={data?.results[dialogSpec.id]} onClose={() => setTileDialog(null)} />
      ) : null}
      {tileDialog?.kind === "expand" && dialogSpec ? (
        <TileExpandDialog spec={dialogSpec} cell={data?.results[dialogSpec.id]} dark={dark} onClose={() => setTileDialog(null)} />
      ) : null}

      {!isDefault ? <ShareDialog board={board} dashName={dashName} open={shareOpen} onOpenChange={setShareOpen} /> : null}
      {renameOpen ? (
        <RenameDashboardDialog open={renameOpen} onOpenChange={setRenameOpen} currentName={dashName} onSave={(n) => void renameDashboard(n)} />
      ) : null}
    </div>
    </ReportingProvider>
  );
}
