"use client";

import * as React from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useTheme } from "next-themes";
import { ChartColumn, Download, Eye, Maximize2, MousePointerClick, Move, Pencil, Sparkles, Table2, Trash2 } from "lucide-react";
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { PageHeader } from "@/components/patterns/page-header";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@rantai/design-system/ui/empty";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { summarizeFilters, summarizeTiles } from "@/lib/page-context-summary";
import { FILTER_PARAM, decodeFilters, filtersToParam, normalizeFilters, toggleValue } from "@/lib/dashboard-filter-state";
import type { ChartRenderSpec, ChartSource } from "@/lib/dashboard-specs";
import type { LayoutMap, FilterDef, FilterField } from "@/services/clients/bi-store";
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
import { FilterBar } from "./filters/filter-bar";
import { SkippedFiltersMarker } from "./filters/skipped-marker";
import { DashboardGrid, type GridItem, type TileMenuItem } from "./dashboard-grid";
import { DashboardTilesSkeleton } from "./dashboard-skeleton";
import { DrillMenu, RecordsDialog, fetchRecords, type DrillTarget, type RecordsState } from "./drill";
import { forgetLastBoard, rememberLastBoard } from "./last-board";
import { ShareDialog } from "./share-dialog";
import { TileBody } from "./tile-body";
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

/**
 * The column a click on this tile's data drills into, if any: category
 * charts only. Built-in tiles have no stored definition, but their `x` is a
 * real mart column — except on time series, whose axis is a derived period.
 */
function drillColumn(spec: ChartCard): string | undefined {
  // Nodes, rings, distributions and days are not one category value to drill
  // into, and neither is a map (points have no category, and a region click
  // would drill into a name that was matched, not stored).
  if (["geomap", "pointmap", "geoheat", "table", "kpi", "gauge", "text", "sankey", "sunburst", "boxplot", "calendar"].includes(spec.kind)) return undefined;
  if (spec.source !== "builtin") return spec.def?.dimension || undefined;
  return ["line", "area"].includes(spec.kind) ? undefined : spec.x || undefined;
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
  const dark = resolvedTheme === "dark";
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
  const [autoSec, setAutoSec] = React.useState("0");
  // Drill / cross-filter: menu on data-point click + a modal of raw rows.
  const [drill, setDrill] = React.useState<(DrillTarget & { builtin: boolean; sqlSource: boolean }) | null>(null);
  const [records, setRecords] = React.useState<RecordsState | null>(null);
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
  }, [board, fParam]);

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
  useAutoRefresh(Number(autoSec) * 1000, load);
  // Esc exits fullscreen.
  React.useEffect(() => {
    if (!fullscreen) return;
    const h = (e: KeyboardEvent) => { if (e.key === "Escape") setFullscreen(false); };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [fullscreen]);

  const defaultFilters = data?.defaultFilters ?? NO_FILTERS;
  // The data on screen was loaded with these; before the first answer the
  // address (or nothing) is all there is.
  const filters = React.useMemo(
    () => decodeFilters(fParam) ?? data?.defaultFilters ?? NO_FILTERS,
    [fParam, data?.defaultFilters],
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
    writeAddress(filtersToParam(next, defaultFilters));
  }, [writeAddress, defaultFilters]);

  const resetFilters = React.useCallback(() => writeAddress(null), [writeAddress]);

  const saveDefault = React.useCallback(async () => {
    setSavingDefault(true);
    try {
      const res = await apiFetch("/api/dashboard/boards", {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ id: board, filters: normalizeFilters(filters) }),
      });
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error ?? "Could not save the default filters");
      }
      // The saved list now equals the state; dropping `f` reloads onto it.
      writeAddress(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingDefault(false);
    }
  }, [board, filters, writeAddress]);

  // Cross-filter: toggle a value in a column → filters EVERY tile with that column.
  const crossFilter = React.useCallback((column: string, value: string) => {
    applyFilters(toggleValue(filters, column, value));
    setDrill(null);
  }, [applyFilters, filters]);

  // Drill-down: show the raw Gold rows behind the clicked value.
  const openRecords = React.useCallback(async (mart: string, column: string, value: string) => {
    setDrill(null);
    setRecords({ columns: [], rows: [], value, loading: true });
    const rows = await fetchRecords(mart, column, value);
    setRecords({ ...rows, value, loading: false });
  }, []);

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

  // Build the tiles for the grid.
  const items: GridItem[] = charts.map((spec) => {
    const cell = data?.results[spec.id];
    const badge = SOURCE_BADGE[spec.source];
    const dim = edit ? undefined : drillColumn(spec);
    const drillable = !!dim;
    const own = spec.source !== "builtin";
    const menu: TileMenuItem[] = [
      { label: "Expand", icon: <Maximize2 />, onSelect: () => setTileDialog({ kind: "expand", id: spec.id }) },
      { label: "View data", icon: <Table2 />, onSelect: () => setTileDialog({ kind: "data", id: spec.id }) },
      ...(hasRows(cell) && cell.rows.length
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
          {drillable ? (
        <Tooltip>
          <TooltipTrigger render={<span className="inline-flex shrink-0 text-muted-foreground/70" />}>
            <MousePointerClick className="size-3.5" aria-label="Clickable chart" />
          </TooltipTrigger>
          <TooltipContent>
            {spec.source === "builtin"
              ? "Click a bar or slice in the chart to see its records"
              : "Click a bar or slice in the chart to filter the dashboard or see its records"}
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
        <TileBody spec={spec} cell={cell} dark={dark} loading={loading}
          onDataClick={dim ? (name, pos) => setDrill({ name, column: dim, mart: spec.mart, x: pos.x, y: pos.y, builtin: spec.source === "builtin", sqlSource: !!spec.sqlSource }) : undefined} />
      ),
    };
  });

  const dialogSpec = tileDialog ? charts.find((c) => c.id === tileDialog.id) : undefined;
  const filterFields = data?.filterFields ?? NO_FIELDS;
  const showFilterBar = filterFields.length > 0 || filters.length > 0;

  return (
    <div className={cn("flex flex-col gap-4", fullscreen && "fixed inset-0 z-40 overflow-auto bg-background p-4 sm:p-6")}>
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
            <Button variant={edit ? "default" : "outline"} size="sm" onClick={() => setEdit((e) => !e)}>
              {edit ? <Eye className="size-4" /> : <Pencil className="size-4" />}{edit ? "Done" : "Edit layout"}
            </Button>
            <DashboardActionsMenu
              isDefault={isDefault}
              loading={loading}
              fullscreen={fullscreen}
              autoSec={autoSec}
              onRefresh={() => void load()}
              onToggleFullscreen={() => setFullscreen((f) => !f)}
              onAutoSec={setAutoSec}
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

      {showFilterBar ? (
        <div data-print-hide className="flex flex-wrap items-center gap-3 rounded-lg border border-border bg-card/50 px-3 py-2">
          <FilterBar
            board={board}
            fields={filterFields}
            filters={filters}
            onChange={applyFilters}
            dirty={dirty}
            canSaveDefault={!isDefault && hasPermission("dashboard:write")}
            saving={savingDefault}
            onSaveDefault={() => void saveDefault()}
            onReset={resetFilters}
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
          onFilter={drill.builtin ? undefined : () => crossFilter(drill.column, drill.name)}
          // Records drill-down over a SQL source is not supported yet (the API
          // answers `supported: false`); offer only the cross-filter there.
          onRecords={drill.sqlSource ? undefined : () => void openRecords(drill.mart, drill.column, drill.name)}
        />
      ) : null}
      <RecordsDialog records={records} onClose={() => setRecords(null)} />

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
  );
}
