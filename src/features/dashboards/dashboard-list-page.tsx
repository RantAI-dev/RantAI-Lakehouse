"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { LayoutGrid, Rows3 } from "lucide-react"

import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { useIsMobile } from "@/hooks/use-mobile"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useUrlTab } from "@/hooks/use-url-tab"
import { useAuth } from "@/features/auth/auth-provider"
import { folderChoices, labelledFolders, manageableFolders } from "@/lib/dashboard-organise"
import { buildFolderTree, flattenFolders, folderPath } from "@/lib/folder-tree"
import { notifyError, notifyFailure, notifyInfo, notifySuccess, withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { dashboardReportService, dashboardService, dashboardTemplateService } from "@/services"
import type { Board, DashboardTemplate } from "@/services"
import { BoardFormDialog } from "./board-form-dialog"
import { notifyDashboardsChanged } from "./dashboard-events"
import { ImportDashboardDialog } from "./import-dashboard-dialog"
import { ManageFoldersDialog, MoveBoardDialog } from "./folder-dialogs"
import {
  boardItem, filterItems, firstDirection, reportItem, sortItems, templateItem,
  type KindFilter, type FolderFilter, type LibraryItem, type SortColumn, type SortState,
} from "./library-items"
import { FolderPicker, NewMenu, PageMenu, type FolderChoice, type ItemActions } from "./library-menus"
import { BulkBar, favouriteItems, pickable, trashItems } from "./library-bulk"
import { EmptyFolder, EmptyWorkspace, NoMatch, NoneOfKind } from "./library-empty"
import { LibraryGallery } from "./library-gallery"
import { EASE, FOCUS, PageFrame, ScrollRegion, SearchField, Segmented } from "./library-parts"
import { LibraryTable, type FolderLookup } from "./library-table"
import { ItemPicture, pictureSource } from "./library-picture"
import { RecentRow } from "./library-recent"
import { MoveItemsDialog } from "./move-items-dialog"
import { UseTemplateDialog } from "./template-dialogs"
import { forgetLastBoard } from "./last-board"
import { TrashDialog } from "./trash-dialog"

/**
 * The type filter, kept in `?tab=` so a link can open one kind (the report
 * page's "Reports" link is `?tab=reports`). It replaces the three tabs of
 * `BI-18` part D. Templates are for a holder of `dashboard:write` (BI-18 part
 * D, T4), so a reader's filter has no Templates.
 */
type TabValue = "all" | "dashboards" | "reports" | "templates"
const READER_TABS: readonly TabValue[] = ["all", "dashboards", "reports"]
const WRITER_TABS: readonly TabValue[] = ["all", "dashboards", "reports", "templates"]
const TAB_LABEL: Record<TabValue, string> = { all: "All", dashboards: "Dashboards", reports: "Reports", templates: "Templates" }
const TAB_KIND: Record<TabValue, KindFilter> = { all: "all", dashboards: "dashboard", reports: "report", templates: "template" }

/**
 * The Dashboards page (`/dashboards/browse`, `BI-59`): find, make and organise
 * the dashboards, reports and templates of the workspace.
 *
 * Reached on purpose, not as the way in: `/dashboards` forwards to the last
 * board opened. This page is for managing the list.
 *
 * The header holds the one primary action (New), the page's ⋯ menu and a
 * large search; nothing else (spec `BI-59` T1). The page is a frame that ends
 * above the assistant bar, so the bar never covers a row (T5).
 *
 * Folders belong here, because folders organise dashboards and this is where
 * dashboards are managed: the list shows each item's folder, the page menu
 * opens the folder manager, "Move to folder…" is in each item's ⋯ menu, and a
 * new dashboard can be filed on creation.
 *
 * The page reads every board at once: `/api/dashboard/boards` is not paged
 * and a workspace holds few dashboards, so search and filters run here.
 *
 * The time column is **Updated**, never Created: `console.bi_board` is a
 * `ReplacingMergeTree(created_at)`, so `created_at` is the version column
 * rewritten on every save and has always meant "last changed".
 */
/**
 * Table or Gallery (decision P2): Table is the default, and the choice is
 * kept per browser. A new key rather than the old page's `dashboards:list-view`,
 * whose default was the gallery, so everyone starts on the table the owner
 * chose. Read after mount so the server's and the browser's first render
 * agree, and wrapped in `try` because `localStorage` throws in a private
 * window or with storage blocked: the page then works and only forgets.
 */
type ViewMode = "table" | "gallery"
const VIEW_STORAGE_KEY = "dashboards:browse-view"

function useViewMode(): readonly [ViewMode, (next: ViewMode) => void] {
  const [view, setView] = React.useState<ViewMode>("table")
  React.useEffect(() => {
    try {
      const saved = window.localStorage.getItem(VIEW_STORAGE_KEY)
      if (saved === "gallery" || saved === "table") setView(saved)
    } catch {
      /* the default stays */
    }
  }, [])
  const change = React.useCallback((next: ViewMode) => {
    setView(next)
    try {
      window.localStorage.setItem(VIEW_STORAGE_KEY, next)
    } catch {
      /* the choice holds for this visit, it is only not remembered */
    }
  }, [])
  return [view, change] as const
}

/** The Table / Gallery switch: two icon buttons with `aria-pressed`, in one soft group like the type filter. */
function ViewSwitch({ value, onChange }: { readonly value: ViewMode; readonly onChange: (next: ViewMode) => void }) {
  const options = [
    { value: "table" as const, label: "Table view", icon: Rows3 },
    { value: "gallery" as const, label: "Gallery view", icon: LayoutGrid },
  ]
  return (
    <div role="group" aria-label="View" className="inline-flex rounded-lg bg-muted p-0.5">
      {options.map((o) => {
        const on = o.value === value
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={on}
            aria-label={o.label}
            title={o.label}
            onClick={() => onChange(o.value)}
            className={cn(
              "grid size-8 place-items-center rounded-md",
              EASE, FOCUS,
              on ? "bg-background text-foreground shadow-sm dark:bg-input/40" : "text-muted-foreground hover:text-foreground",
            )}
          >
            <o.icon aria-hidden className="size-4" />
          </button>
        )
      })}
    </div>
  )
}

/** What the list says when it has no row: why, in words that fit the filter (T5's designed states). */
function ListEmpty({ kind, query, folderFilter, folderPathOf, isPersonal, canWrite, anyOfKind, onShowAll }: {
  readonly kind: KindFilter
  readonly query: string
  readonly folderFilter: FolderFilter
  readonly folderPathOf: (id: string) => string
  readonly isPersonal: (id: string) => boolean
  readonly canWrite: boolean
  /** Whether anything of this kind exists at all, before the search and folder. */
  readonly anyOfKind: boolean
  readonly onShowAll: () => void
}) {
  if (query.trim()) return <NoMatch query={query.trim()} />
  if (folderFilter !== null && anyOfKind) {
    return (
      <EmptyFolder
        path={folderFilter ? folderPathOf(folderFilter) : "No folder"}
        personal={Boolean(folderFilter) && isPersonal(folderFilter)}
        onShowAll={onShowAll}
      />
    )
  }
  return <NoneOfKind kind={kind} canWrite={canWrite} />
}

export function DashboardListPage() {
  const router = useRouter()
  const state = useService((s) => dashboardService.listBoards(s), [])
  // Folders are listed to everyone who can see dashboards; filing needs
  // `dashboard:write`, which the API enforces (policy.rs).
  const foldersState = useService((s) => dashboardService.listFolders(s), [])
  const reportsState = useService((s) => dashboardReportService.listReports(s), [])
  const auth = useAuth()
  const me = auth.user?.id
  const myName = auth.user?.name
  // The caller's own personal space is called "Personal space"; the folder
  // manager never offers one, and a dashboard is filed only in an ordinary
  // folder or the caller's own space (BI-18 part C, part 2).
  const folders = React.useMemo(
    () => labelledFolders(foldersState.data ?? [], me),
    [foldersState.data, me],
  )
  const fileable = React.useMemo(() => folderChoices(folders, me), [folders, me])
  const reloadFolders = foldersState.reload
  const canFile = auth.hasPermission("dashboard:write")
  const tabValues = canFile ? WRITER_TABS : READER_TABS
  const [tab, setTabRaw] = useUrlTab<TabValue>("tab", tabValues)
  // The personal space is created on first use: a writer with none yet opens it
  // once, then the folder list is read again.
  const spaceAsked = React.useRef(false)
  React.useEffect(() => {
    if (!canFile || !me || foldersState.status !== "success" || spaceAsked.current) return
    spaceAsked.current = true
    if (foldersState.data.some((f) => f.ownerId === me)) return
    void dashboardService
      .ensurePersonalSpace()
      .then(() => reloadFolders())
      .catch(() => undefined)
  }, [canFile, me, foldersState.status, foldersState.data, reloadFolders])
  // Templates are for a holder of `dashboard:write` (BI-18 part D, T4). A
  // reader asks for nothing.
  // The answer names the permission it was read for: `useService` shows the
  // previous answer for one render after `canFile` changes, and a reader's
  // empty stand-in must not pass for a writer's list.
  const templatesState = useService(
    async (s) => ({
      forWriter: canFile,
      list: canFile ? await dashboardTemplateService.listTemplates(s) : { templates: [], truncated: false },
    }),
    [canFile],
  )
  const templatesReady = templatesState.status === "error" || (templatesState.status === "success" && templatesState.data.forWriter === canFile)
  const templates = React.useMemo(
    () => (templatesState.status === "success" ? templatesState.data.list.templates : templatesState.status === "error" ? null : []),
    [templatesState.status, templatesState.data],
  )
  // The pictures (decision P3): every stored chart's board, kind and width,
  // read once for the whole page; no dashboard is read and no query runs.
  // When it fails the pictures fall back to glyphs and the list still works.
  const outlinesState = useService((s) => dashboardService.listChartOutlines(s), [])
  const pictures = React.useMemo(
    () => pictureSource(outlinesState.status === "success" ? outlinesState.data : outlinesState.status === "error" ? null : []),
    [outlinesState.status, outlinesState.data],
  )
  // Favourites and recent items (best effort: the list works without them).
  const shortcuts = useService((s) => dashboardService.shortcuts(s), [])
  const favouriteIds = React.useMemo(
    () => new Set((shortcuts.status === "success" ? shortcuts.data.favourites : []).filter((f) => f.kind === "board").map((f) => f.id)),
    [shortcuts],
  )
  const reloadShortcuts = shortcuts.reload
  const [query, setQueryRaw] = React.useState("")
  const [folderFilter, setFolderFilterRaw] = React.useState<FolderFilter>(null)
  // BI-59 T5: the ticked rows, by key. A change of search, type, folder or
  // view clears them, so a bulk action never reaches a row that is not shown.
  const [selected, setSelected] = React.useState<ReadonlySet<string>>(new Set())
  const [bulkTrash, setBulkTrash] = React.useState<readonly LibraryItem[] | null>(null)
  const [bulkBusy, setBulkBusy] = React.useState(false)
  const clearSelection = React.useCallback(() => setSelected(new Set()), [])
  const setQuery = (next: string) => {
    setQueryRaw(next)
    clearSelection()
  }
  const setFolderFilter = (next: FolderFilter) => {
    setFolderFilterRaw(next)
    clearSelection()
  }
  const [sort, setSort] = React.useState<SortState>({ column: "updated", dir: "desc" })
  const [view, setViewRaw] = useViewMode()
  const phone = useIsMobile()
  const setView = (next: ViewMode) => {
    setViewRaw(next)
    clearSelection()
  }
  const [trashOpen, setTrashOpen] = React.useState(false)
  const [foldersOpen, setFoldersOpen] = React.useState(false)
  const [importOpen, setImportOpen] = React.useState(false)
  const [moving, setMoving] = React.useState<Board | null>(null)
  const [movingItems, setMovingItems] = React.useState<readonly LibraryItem[] | null>(null)
  const [using, setUsing] = React.useState<DashboardTemplate | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [renaming, setRenaming] = React.useState<Board | null>(null)
  const [deleting, setDeleting] = React.useState<Board | null>(null)
  const [deletingReport, setDeletingReport] = React.useState<LibraryItem | null>(null)
  const [deletingTemplate, setDeletingTemplate] = React.useState<LibraryItem | null>(null)

  const reload = state.reload
  const reloadReports = reportsState.reload
  const reloadTemplates = templatesState.reload
  const createAction = useServiceAction(
    withNotify(
      { success: "Dashboard created", error: "Could not create dashboard" },
      // Filing is a second call: the create endpoint takes no folder. If
      // it fails the dashboard still exists, at the top level, and the
      // error says so rather than reporting the whole create as failed.
      async (signal, input: { name: string; description: string; folderId: string }) => {
        const created = await dashboardService.createBoard(
          { name: input.name, description: input.description },
          signal,
        )
        if (input.folderId && created?.id) {
          await dashboardService.moveBoard(created.id, input.folderId, signal)
        }
        return created
      },
    ),
  )
  const updateAction = useServiceAction(
    withNotify(
      { success: "Dashboard updated", error: "Could not update dashboard" },
      (signal, input: { id: string; name?: string; description?: string }) =>
        dashboardService.updateBoard(input, signal),
    ),
  )
  const duplicateAction = useServiceAction(
    withNotify(
      { success: "Dashboard duplicated", error: "Could not duplicate dashboard" },
      (signal, id: string) => dashboardService.duplicateBoard(id, signal),
    ),
  )
  const deleteAction = useServiceAction(
    withNotify(
      { success: "Moved to the trash", error: "Could not delete dashboard" },
      (signal, id: string) => dashboardService.deleteBoard(id, signal),
    ),
  )
  const deleteReportAction = useServiceAction(
    withNotify(
      { success: "Moved to the trash", error: "Could not delete the report" },
      (signal, id: string) => dashboardReportService.deleteReport(id, signal),
    ),
  )
  const deleteTemplateAction = useServiceAction(
    withNotify(
      { success: "Template deleted", error: "Could not delete the template" },
      (signal, id: string) => dashboardTemplateService.deleteTemplate(id, signal),
    ),
  )

  const toggleFavourite = React.useCallback(
    (item: LibraryItem) => {
      void (async () => {
        try {
          await dashboardService.setFavourite("board", item.id, !favouriteIds.has(item.id))
        } catch (err) {
          notifyError("Could not save the favourite", err)
        }
        reloadShortcuts()
      })()
    },
    [favouriteIds, reloadShortcuts],
  )
  const actions = React.useMemo<ItemActions>(
    () => ({
      isFavourite: (item) => item.kind === "dashboard" && favouriteIds.has(item.id),
      onToggleFavourite: toggleFavourite,
      onRename: canFile ? (item) => item.board && setRenaming(item.board) : undefined,
      onMove: canFile
        ? (item) => (item.kind === "dashboard" && item.board ? setMoving(item.board) : setMovingItems([item]))
        : undefined,
      onTrash: canFile
        ? (item) => (item.kind === "dashboard" && item.board ? setDeleting(item.board) : setDeletingReport(item))
        : undefined,
      onDuplicate: canFile
        ? (item) => {
            void (async () => {
              const created = await duplicateAction.run(item.id)
              notifyDashboardsChanged()
              reload()
              if (created?.id) router.push(`/dashboards/${created.id}`)
            })()
          }
        : undefined,
      onUseTemplate: canFile ? (item) => item.template && setUsing(item.template) : undefined,
      onDeleteTemplate: canFile ? setDeletingTemplate : undefined,
    }),
    [canFile, duplicateAction, favouriteIds, reload, router, toggleFavourite],
  )

  const pathOf = React.useCallback((folderId: string) => (folderId ? folderPath(folders, folderId) : ""), [folders])
  const folderLookup = React.useMemo<FolderLookup>(
    () => ({ pathOf, isPersonal: (id) => Boolean(folders.find((f) => f.id === id)?.ownerId) }),
    [folders, pathOf],
  )

  const items = React.useMemo<LibraryItem[]>(
    () => [
      ...(state.data ?? []).map(boardItem),
      ...(reportsState.data?.reports ?? []).map(reportItem),
      ...(templates ?? []).map(templateItem),
    ],
    [state.data, reportsState.data, templates],
  )
  const kind = TAB_KIND[tab]
  const rows = React.useMemo(
    () => sortItems(filterItems(items, { kind, folder: folderFilter, query }, pathOf), sort, pathOf),
    [items, kind, folderFilter, query, sort, pathOf],
  )
  const folderChoicesList = React.useMemo<FolderChoice[]>(
    () =>
      flattenFolders(buildFolderTree(folders, []).folders).map(({ folder }) => ({
        id: folder.id,
        path: folderPath(folders, folder.id),
        personal: Boolean(folder.ownerId),
        count: items.filter((i) => i.kind !== "template" && i.folderId === folder.id).length,
      })),
    [folders, items],
  )
  // A folder filter that names a folder no longer listed (it was deleted
  // from the folder manager) falls back to every folder.
  React.useEffect(() => {
    if (folderFilter && foldersState.status === "success" && !folders.some((f) => f.id === folderFilter)) setFolderFilterRaw(null)
  }, [folderFilter, folders, foldersState.status])

  const sortBy = (column: SortColumn) =>
    setSort((s) => (s.column === column ? { column, dir: s.dir === "asc" ? "desc" : "asc" } : { column, dir: firstDirection(column) }))

  // The list waits for the three reads, so rows do not jump in as each one
  // lands; a failed read of reports or templates is said under the list and
  // leaves the dashboards usable.
  // Until the session is known the templates read is a stand-in (`canFile` is
  // still false), so the list also waits for it.
  const loading = auth.status === "loading" || state.status === "loading" || reportsState.status === "loading" || !templatesReady || outlinesState.status === "loading"
  const tabs = tabValues.map((v) => ({ value: v, label: TAB_LABEL[v] }))
  const setTab = (next: TabValue) => {
    setTabRaw(next)
    clearSelection()
  }
  const ticked = rows.filter((r) => selected.has(r.key) && pickable(r))
  const selection = canFile
    ? {
        selected,
        onToggle: (item: LibraryItem) =>
          setSelected((prev) => {
            const next = new Set(prev)
            if (!next.delete(item.key)) next.add(item.key)
            return next
          }),
        onToggleAll: (list: readonly LibraryItem[], on: boolean) => setSelected(on ? new Set(list.map((i) => i.key)) : new Set()),
      }
    : undefined
  const isFavourite = actions.isFavourite
  async function favouriteTicked() {
    setBulkBusy(true)
    const out = await favouriteItems(ticked, isFavourite)
    setBulkBusy(false)
    reloadShortcuts()
    clearSelection()
    const skipped = out.skipped > 0 ? `${out.skipped.toLocaleString("en-US")} ${out.skipped === 1 ? "report was" : "reports were"} skipped: reports cannot be favourites.` : undefined
    if (out.refused.length > 0) notifyFailure(`${out.refused.length.toLocaleString("en-US")} could not be added to favourites`, out.refused.map((r) => `${r.name}: ${r.reason}`).join(" "))
    else notifySuccess(out.done === 1 ? "1 dashboard added to favourites" : `${out.done.toLocaleString("en-US")} dashboards added to favourites`, skipped)
  }
  async function trashTicked(list: readonly LibraryItem[]) {
    setBulkBusy(true)
    const out = await trashItems(list)
    setBulkBusy(false)
    setBulkTrash(null)
    clearSelection()
    if (out.done > 0) {
      notifyDashboardsChanged()
      reload()
      reloadReports()
      notifySuccess(`${out.done.toLocaleString("en-US")} moved to the trash`, "Restore them from the trash for 30 days.")
    }
    if (out.refused.length > 0) notifyFailure(`${out.refused.length.toLocaleString("en-US")} could not be moved to the trash`, out.refused.map((r) => `${r.name}: ${r.reason}`).join(" "))
  }
  // The workspace holds nothing but the built-in Main: the list says how to start.
  const emptyWorkspace = !loading && state.status === "success" && items.every((i) => i.builtin)
  const boardsById = React.useMemo(
    () => new Map(items.filter((i) => i.kind === "dashboard").map((i) => [i.id, i])),
    [items],
  )
  const recent = shortcuts.status === "success" ? shortcuts.data.recent : []

  return (
    <PageFrame>
      <header className="grid shrink-0 gap-4">
        <div className="flex items-center justify-between gap-3">
          <h1 className="text-xl font-semibold tracking-tight">Dashboards</h1>
          <div className="flex items-center gap-2">
            <PageMenu
              onFolders={canFile ? () => setFoldersOpen(true) : undefined}
              onTrash={canFile ? () => setTrashOpen(true) : undefined}
            />
            {/* BI-18 part C review fix (SHOULD-FIX) R27: without `dashboard:write` there is no New. */}
            {canFile ? (
              <NewMenu
                templates={templates}
                renderPicture={(t) => (
                  <ItemPicture item={templateItem(t)} charts={null} size="sm" className="h-10 w-16 shrink-0" />
                )}
                onDashboard={() => setCreateOpen(true)}
                onTemplate={setUsing}
                onImport={() => setImportOpen(true)}
              />
            ) : null}
          </div>
        </div>
        <SearchField value={query} onChange={setQuery} className="max-w-2xl" />
      </header>

      <ScrollRegion className="-mx-2 mt-5 px-2 pb-6">
        <div className="grid grid-cols-[minmax(0,1fr)] gap-7">
          {!query.trim() && state.status === "success" && !loading ? (
            <RecentRow recent={recent} boards={boardsById} source={pictures} />
          ) : null}
          <section aria-label="Dashboards, reports and templates" className="grid grid-cols-[minmax(0,1fr)] gap-3">
            {/* The controls row; while rows are ticked it becomes the bulk bar, in the same place. */}
            <div className="flex min-h-9 flex-wrap items-center justify-between gap-2">
              {ticked.length > 0 ? (
                <BulkBar
                  count={ticked.length}
                  canFavourite={ticked.some((i) => i.kind === "dashboard")}
                  busy={bulkBusy}
                  onClear={clearSelection}
                  onMove={() => setMovingItems(ticked)}
                  onFavourite={() => void favouriteTicked()}
                  onTrash={() => setBulkTrash(ticked)}
                />
              ) : (
                <>
                  {/* Smaller and full width on a phone, so the four choices fit one line. */}
                  <Segmented<TabValue> label="Type" size={phone ? "sm" : "default"} value={tab} onChange={setTab} options={tabs} className={cn(phone && "w-full")} />
                  <div className={cn("flex items-center gap-2", phone && "w-full justify-between")}>
                    <FolderPicker
                      value={folderFilter}
                      onChange={setFolderFilter}
                      choices={folderChoicesList}
                      noFolderCount={items.filter((i) => i.kind !== "template" && !i.folderId).length}
                    />
                    <ViewSwitch value={view} onChange={setView} />
                  </div>
                </>
              )}
            </div>
            {emptyWorkspace && !query.trim() && folderFilter === null && kind !== "report" && kind !== "template" ? (
              <EmptyWorkspace canWrite={canFile} onNewDashboard={() => setCreateOpen(true)} />
            ) : null}

            {state.status === "error" ? <ErrorState error={state.error} onRetry={reload} /> : null}
            {state.status !== "error" && loading ? <LoadingSkeleton /> : null}
            {state.status === "success" && !loading ? (
              rows.length === 0 ? (
                <ListEmpty
                  kind={kind}
                  query={query}
                  folderFilter={folderFilter}
                  folderPathOf={pathOf}
                  isPersonal={folderLookup.isPersonal}
                  canWrite={canFile}
                  anyOfKind={items.some((i) => kind === "all" || i.kind === kind)}
                  onShowAll={() => setFolderFilter(null)}
                />
              ) : view === "gallery" ? (
                <LibraryGallery
                  items={rows}
                  folders={folders}
                  showEmpty={!query.trim() && folderFilter === null && (kind === "all" || kind === "dashboard")}
                  actions={actions}
                  myName={myName}
                  pictures={pictures}
                  onOpenTemplate={(item) => item.template && setUsing(item.template)}
                  emptyFolder={(id) => <EmptyFolder compact path={pathOf(id)} personal={folderLookup.isPersonal(id)} />}
                />
              ) : (
                <LibraryTable
                  items={rows}
                  sort={sort}
                  onSort={sortBy}
                  actions={actions}
                  folders={folderLookup}
                  myName={myName}
                  onOpenTemplate={(item) => item.template && setUsing(item.template)}
                  pictures={pictures}
                  selection={selection}
                  phone={phone}
                />
              )
            ) : null}
            {reportsState.status === "error" ? (
              <p className="text-xs text-destructive">
                Reports could not be listed: {reportsState.error.message}{" "}
                <button type="button" className="underline underline-offset-4" onClick={reloadReports}>Try again</button>
              </p>
            ) : null}
            {templatesState.status === "error" ? (
              <p className="text-xs text-destructive">
                Templates could not be listed: {templatesState.error.message}{" "}
                <button type="button" className="underline underline-offset-4" onClick={reloadTemplates}>Try again</button>
              </p>
            ) : null}
            {reportsState.data?.truncated ? (
              <p className="text-xs text-muted-foreground">Showing the 1,000 most recently changed reports. Search to find an older one.</p>
            ) : null}
            {templatesState.data?.list.truncated ? (
              <p className="text-xs text-muted-foreground">Showing the 500 newest templates.</p>
            ) : null}
          </section>
        </div>
      </ScrollRegion>

      <BoardFormDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        title="New dashboard"
        description="Give it a name now; you can add tiles after it opens."
        submitLabel="Create"
        initialName=""
        initialDescription=""
        folders={canFile ? fileable : undefined}
        busy={createAction.status === "pending"}
        onSubmit={({ name, description, folderId }) => {
          void (async () => {
            const created = await createAction.run({ name, description, folderId })
            setCreateOpen(false)
            notifyDashboardsChanged()
            reload()
            if (created?.id) router.push(`/dashboards/${created.id}`)
          })()
        }}
      />

      <BoardFormDialog
        open={renaming !== null}
        onOpenChange={(open) => !open && setRenaming(null)}
        title="Rename dashboard"
        description="Change how this dashboard is named and described."
        submitLabel="Save"
        initialName={renaming?.name ?? ""}
        initialDescription={renaming?.description ?? ""}
        busy={updateAction.status === "pending"}
        onSubmit={({ name, description }) => {
          const target = renaming
          if (!target) return
          void (async () => {
            await updateAction.run({ id: target.id, name, description })
            setRenaming(null)
            notifyDashboardsChanged()
            reload()
          })()
        }}
      />

      {canFile ? (
        <ImportDashboardDialog
          open={importOpen}
          onOpenChange={setImportOpen}
          folders={fileable}
          onImported={() => {
            notifyDashboardsChanged()
            reload()
          }}
        />
      ) : null}
      {canFile ? (
        <ManageFoldersDialog
          open={foldersOpen}
          onOpenChange={setFoldersOpen}
          folders={manageableFolders(folders)}
          onChanged={() => {
            notifyDashboardsChanged()
            reloadFolders()
            reload()
          }}
        />
      ) : null}
      {canFile && moving ? (
        <MoveBoardDialog
          open
          onOpenChange={(open) => !open && setMoving(null)}
          folders={fileable}
          boardId={moving.id}
          currentFolderId={moving.folderId ?? ""}
          onMoved={() => {
            notifyDashboardsChanged()
            reload()
          }}
        />
      ) : null}
      {canFile ? (
        <UseTemplateDialog
          template={using}
          folders={fileable}
          onOpenChange={(open) => !open && setUsing(null)}
          onStarted={(started) => {
            setUsing(null)
            notifyDashboardsChanged()
            if (started.leftOut.length > 0) {
              notifyInfo(
                `${started.leftOut.length} chart(s) could not be copied`,
                started.leftOut.map((c) => `${c.title}: ${c.reason}`).join(" "),
              )
            }
            router.push(`/dashboards/${encodeURIComponent(started.board.id)}`)
          }}
        />
      ) : null}

      {canFile && movingItems ? (
        <MoveItemsDialog
          items={movingItems}
          folders={fileable}
          onOpenChange={(open) => !open && setMovingItems(null)}
          onDone={(outcome) => {
            clearSelection()
            if (outcome.moved > 0) {
              notifyDashboardsChanged()
              reload()
              reloadReports()
              notifySuccess(outcome.moved === 1 ? "Moved" : `${outcome.moved.toLocaleString("en-US")} moved`)
            }
          }}
        />
      ) : null}

      {trashOpen ? (
        <TrashDialog
          canWrite={canFile}
          onOpenChange={setTrashOpen}
          onRestored={() => {
            notifyDashboardsChanged()
            reload()
            reloadReports()
            reloadShortcuts()
          }}
        />
      ) : null}

      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={`Delete ${deleting?.name ?? "dashboard"}?`}
        description="The dashboard and its tiles go to the trash. You can restore them for 30 days."
        impact={
          deleting?.publicToken
            ? "This dashboard has a public link. Anyone using that link will lose access."
            : undefined
        }
        confirmLabel="Delete"
        destructive
        confirming={deleteAction.status === "pending"}
        onConfirm={() => {
          const target = deleting
          if (!target) return
          void (async () => {
            await deleteAction.run(target.id)
            setDeleting(null)
            // If this board was the last one opened, `/dashboards` must not
            // lead back to something that is gone.
            forgetLastBoard(target.id)
            notifyDashboardsChanged()
            reload()
          })()
        }}
      />
      <ConfirmActionDialog
        open={bulkTrash !== null}
        onOpenChange={(open) => !open && setBulkTrash(null)}
        title={`Move ${(bulkTrash?.length ?? 0).toLocaleString("en-US")} ${bulkTrash?.length === 1 ? "item" : "items"} to the trash?`}
        description="Dashboards go with their tiles, and reports keep their charts on their dashboards. You can restore them for 30 days."
        impact={(() => {
          const linked = (bulkTrash ?? []).filter((i) => i.publicLink).map((i) => i.name)
          if (linked.length === 0) return undefined
          return `${linked.join(", ")} ${linked.length === 1 ? "has a public link" : "have public links"}. Anyone using ${linked.length === 1 ? "that link" : "those links"} will lose access.`
        })()}
        confirmLabel="Move to trash"
        destructive
        confirming={bulkBusy}
        onConfirm={() => {
          const list = bulkTrash
          if (list) void trashTicked(list)
        }}
      />

      <ConfirmActionDialog
        open={deletingReport !== null}
        onOpenChange={(open) => !open && setDeletingReport(null)}
        title={`Delete ${deletingReport?.name ?? "report"}?`}
        description="The report goes to the trash. You can restore it for 30 days. Its charts stay on their dashboards."
        confirmLabel="Delete"
        destructive
        confirming={deleteReportAction.status === "pending"}
        onConfirm={() => {
          const target = deletingReport
          if (!target) return
          void (async () => {
            await deleteReportAction.run(target.id)
            setDeletingReport(null)
            reloadReports()
          })()
        }}
      />

      <ConfirmActionDialog
        open={deletingTemplate !== null}
        onOpenChange={(open) => !open && setDeletingTemplate(null)}
        title={`Delete the template ${deletingTemplate?.name ?? ""}?`}
        description="The template is deleted for good. Dashboards already made from it are not changed."
        confirmLabel="Delete"
        destructive
        confirming={deleteTemplateAction.status === "pending"}
        onConfirm={() => {
          const target = deletingTemplate
          if (!target) return
          void (async () => {
            await deleteTemplateAction.run(target.id)
            setDeletingTemplate(null)
            reloadTemplates()
          })()
        }}
      />
    </PageFrame>
  )
}
