"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { FileText, LayoutGrid, LayoutTemplate } from "lucide-react"

import { DataTable } from "@/components/data-table/data-table"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { LineTabsList, type LineTab } from "@/components/patterns/line-tabs"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useUrlTab } from "@/hooks/use-url-tab"
import { useAuth } from "@/features/auth/auth-provider"
import { filterDataClientSide } from "@/lib/data-table"
import { folderChoices, labelledFolders, manageableFolders } from "@/lib/dashboard-organise"
import { folderPath } from "@/lib/folder-tree"
import { notifyError, notifyInfo, withNotify } from "@/lib/notify"
import { dashboardService, dashboardTemplateService } from "@/services"
import type { Board, DashboardTemplate } from "@/services"
import { BoardFormDialog } from "./board-form-dialog"
import { notifyDashboardsChanged } from "./dashboard-events"
import { getDashboardColumns, type BoardRowActions } from "./dashboard-list-columns"
import { ImportDashboardDialog } from "./import-dashboard-dialog"
import { ManageFoldersDialog, MoveBoardDialog } from "./folder-dialogs"
import { NewMenu, PageMenu } from "./library-menus"
import { PageFrame, ScrollRegion, SearchField } from "./library-parts"
import { ReportList } from "./report-list"
import { UseTemplateDialog } from "./template-dialogs"
import { TemplateList } from "./template-list"
import { forgetLastBoard } from "./last-board"
import { TrashDialog } from "./trash-dialog"

/**
 * The tabs of the Dashboards page (`BI-18` part D): the dashboards, the
 * reports and, for a holder of `dashboard:write`, the templates. The open one
 * is in `?tab=`, so a report's "Reports" link lands on it.
 */
type ListTab = "dashboards" | "reports" | "templates"
const READER_TABS: readonly ListTab[] = ["dashboards", "reports"]
/** BI-18 part D (T6, T4): templates are for those who may save and use them. */
const WRITER_TABS: readonly ListTab[] = ["dashboards", "reports", "templates"]
const TAB_STRIP: Record<ListTab, LineTab> = {
  dashboards: { value: "dashboards", label: "Dashboards", icon: LayoutGrid },
  reports: { value: "reports", label: "Reports", icon: FileText },
  templates: { value: "templates", label: "Templates", icon: LayoutTemplate },
}

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
export function DashboardListPage() {
  const router = useRouter()
  const state = useService((s) => dashboardService.listBoards(s), [])
  // Folders are listed to everyone who can see dashboards; filing needs
  // `dashboard:write`, which the API enforces (policy.rs).
  const foldersState = useService((s) => dashboardService.listFolders(s), [])
  const auth = useAuth()
  const me = auth.user?.id
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
  const [tab, setTab] = useUrlTab<ListTab>("tab", tabValues)
  const listTabs = tabValues.map((t) => TAB_STRIP[t])
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
  // Templates are for a holder of `dashboard:write` (BI-18 part D, T4); the
  // New menu offers them. A reader asks for nothing.
  const templatesState = useService(
    (s) => (canFile ? dashboardTemplateService.listTemplates(s) : Promise.resolve({ templates: [], truncated: false })),
    [canFile],
  )
  const templates = templatesState.status === "success" ? templatesState.data.templates : templatesState.status === "error" ? null : []
  // Favourites (best effort: the list works without them).
  const shortcuts = useService((s) => dashboardService.shortcuts(s), [])
  const favouriteIds = React.useMemo(
    () => new Set((shortcuts.status === "success" ? shortcuts.data.favourites : []).filter((f) => f.kind === "board").map((f) => f.id)),
    [shortcuts],
  )
  const reloadShortcuts = shortcuts.reload
  const [query, setQuery] = React.useState("")
  const [trashOpen, setTrashOpen] = React.useState(false)
  const [foldersOpen, setFoldersOpen] = React.useState(false)
  const [importOpen, setImportOpen] = React.useState(false)
  const [moving, setMoving] = React.useState<Board | null>(null)
  const [using, setUsing] = React.useState<DashboardTemplate | null>(null)
  const [createOpen, setCreateOpen] = React.useState(false)
  const [renaming, setRenaming] = React.useState<Board | null>(null)
  const [deleting, setDeleting] = React.useState<Board | null>(null)

  const reload = state.reload
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

  const actions = React.useMemo<BoardRowActions>(
    () => ({
      onRename: canFile ? setRenaming : undefined,
      onDelete: canFile ? setDeleting : undefined,
      onMove: canFile ? setMoving : undefined,
      isFavourite: (b) => favouriteIds.has(b.id),
      onToggleFavourite: (b) => {
        void (async () => {
          try {
            await dashboardService.setFavourite("board", b.id, !favouriteIds.has(b.id))
          } catch (err) {
            notifyError("Could not save the favourite", err)
          }
          reloadShortcuts()
        })()
      },
      onDuplicate: canFile
        ? (b) => {
            void (async () => {
              const created = await duplicateAction.run(b.id)
              notifyDashboardsChanged()
              reload()
              if (created?.id) router.push(`/dashboards/${created.id}`)
            })()
          }
        : undefined,
    }),
    [canFile, duplicateAction, favouriteIds, reload, reloadShortcuts, router],
  )
  const folderPathOf = React.useCallback(
    (b: Board) => (b.folderId ? folderPath(folders, b.folderId) : ""),
    [folders],
  )
  const columns = React.useMemo(
    () => getDashboardColumns(actions, folderPathOf),
    [actions, folderPathOf],
  )

  const boards = React.useMemo(() => state.data ?? [], [state.data])
  const filtered = React.useMemo(
    () =>
      filterDataClientSide(boards, {
        search: query,
        // The folder path is searchable, so typing a folder's name
        // narrows the list to what is filed in it.
        searchFields: [
          (b) => b.name,
          (b) => b.description ?? "",
          (b) => b.createdBy ?? "",
          folderPathOf,
        ],
        filters: [],
        joinOperator: "and",
      }),
    [boards, folderPathOf, query],
  )

  const { table } = useDataTable({
    data: filtered,
    columns,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/dashboards/browse",
    initialState: { columnPinning: { right: ["actions"] } },
    getRowId: (row) => row.id,
  })

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
        <Tabs value={tab} onValueChange={(v) => setTab(v as ListTab)} className="gap-4">
          <LineTabsList tabs={listTabs} active={tab} />
          <TabsContent value="dashboards" className="flex flex-col gap-4">
            {state.status === "loading" ? <LoadingSkeleton /> : null}
            {state.status === "error" ? <ErrorState error={state.error} onRetry={reload} /> : null}
            {state.status === "success" && boards.length === 0 ? (
              <EmptyState
                title="No dashboards yet"
                description="Create a dashboard to collect charts, KPIs and tables on one canvas."
              />
            ) : null}
            {state.status === "success" && boards.length > 0 ? (
              filtered.length === 0 ? (
                <EmptyState title="No matching dashboards" description="No dashboard matches the search." />
              ) : (
                <div className="rounded-md border">
                  <DataTable table={table} />
                </div>
              )
            ) : null}
          </TabsContent>
          <TabsContent value="reports">
            {/* BI-18 part D (T5): reports live in the same folders, under the same access. */}
            <ReportList folders={folders} canWrite={canFile} query={query} />
          </TabsContent>
          {canFile ? (
            <TabsContent value="templates">
              <TemplateList folders={fileable} query={query} />
            </TabsContent>
          ) : null}
        </Tabs>
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

      {trashOpen ? (
        <TrashDialog
          canWrite={canFile}
          onOpenChange={setTrashOpen}
          onRestored={() => {
            notifyDashboardsChanged()
            reload()
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
    </PageFrame>
  )
}
