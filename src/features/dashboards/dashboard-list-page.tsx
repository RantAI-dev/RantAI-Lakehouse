"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { ChevronDown, Folder, FolderCog, LayoutGrid, PlusIcon, Rows3 } from "lucide-react"

import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useAuth } from "@/features/auth/auth-provider"
import { filterDataClientSide } from "@/lib/data-table"
import {
  buildFolderTree,
  flattenFolders,
  folderPath,
  groupBoardsByFolder,
  type FolderLike,
} from "@/lib/folder-tree"
import { formatRelativeTime } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { dashboardService } from "@/services"
import type { Board } from "@/services/clients/bi-store"
import { notifyDashboardsChanged } from "./dashboard-events"
import {
  BoardActionsMenu,
  getDashboardColumns,
  ShareBadges,
  type BoardRowActions,
} from "./dashboard-list-columns"
import { ManageFoldersDialog, MoveBoardDialog } from "./folder-dialogs"
import { forgetLastBoard } from "./last-board"

/** Select value for "no folder" (base-ui needs a non-empty value). */
const NO_FOLDER = "__none__"

type ViewMode = "gallery" | "table"
const VIEW_STORAGE_KEY = "dashboards:list-view"

/** Dialog buat/ubah nama + deskripsi sebuah dashboard. */
function BoardFormDialog({
  open,
  onOpenChange,
  title,
  description,
  submitLabel,
  initialName,
  initialDescription,
  folders,
  busy,
  onSubmit,
}: {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly title: string
  readonly description: string
  readonly submitLabel: string
  readonly initialName: string
  readonly initialDescription: string
  /** Given when creating: offers a folder to file the new dashboard in. */
  readonly folders?: FolderLike[]
  readonly busy: boolean
  readonly onSubmit: (values: { name: string; description: string; folderId: string }) => void
}) {
  const [name, setName] = React.useState(initialName)
  const [desc, setDesc] = React.useState(initialDescription)
  const [folderId, setFolderId] = React.useState(NO_FOLDER)
  const folderOptions = flattenFolders(buildFolderTree(folders ?? [], []).folders)

  // Reset tiap kali dibuka: dialog ini dipakai ulang untuk baris yang
  // berbeda, jadi state lama tidak boleh bocor ke board berikutnya.
  React.useEffect(() => {
    if (open) {
      setName(initialName)
      setDesc(initialDescription)
      setFolderId(NO_FOLDER)
    }
  }, [open, initialName, initialDescription])

  const clean = name.trim()
  const canSubmit = clean.length > 0

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="board-name">Name</Label>
            <Input
              id="board-name"
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Quarterly visitors"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="board-description">Description</Label>
            <Input
              id="board-description"
              value={desc}
              onChange={(e) => setDesc(e.target.value)}
              placeholder="What this dashboard answers"
            />
            <p className="text-xs text-muted-foreground">
              Optional. Shown in the dashboard list so others know what it is for.
            </p>
          </div>
          {folderOptions.length > 0 ? (
            <div className="flex flex-col gap-1.5">
              <Label>Folder</Label>
              <Select value={folderId} onValueChange={(v) => setFolderId(v ?? NO_FOLDER)}>
                <SelectTrigger className="w-full">
                  <SelectValue>
                    {folderId === NO_FOLDER ? "No folder" : folderPath(folders ?? [], folderId)}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_FOLDER}>No folder</SelectItem>
                  {folderOptions.map(({ folder }) => (
                    <SelectItem key={folder.id} value={folder.id}>
                      {folderPath(folders ?? [], folder.id)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          ) : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={!canSubmit || busy}
            onClick={() =>
              onSubmit({
                name: clean,
                description: desc.trim(),
                folderId: folderId === NO_FOLDER ? "" : folderId,
              })
            }
          >
            {busy ? "Working…" : submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/**
 * Kartu satu dashboard untuk tampilan galeri.
 *
 * The whole card opens the dashboard (a stretched link on the name), and
 * the ⋯ menu sits above that link so the card offers the same actions as a
 * table row. A menu button cannot be nested inside a link, hence the
 * overlay instead of wrapping the card in one.
 */
function BoardCard({
  board,
  actions,
}: {
  readonly board: Board
  readonly actions: BoardRowActions
}) {
  return (
    <div
      className={cn(
        "group relative flex flex-col gap-3 rounded-xl border bg-card p-4 text-left",
        "shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] transition-colors hover:border-primary/40",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <Link
          href={`/dashboards/${board.id}`}
          className="min-w-0 font-medium outline-none after:absolute after:inset-0 after:rounded-xl group-hover:underline focus-visible:after:ring-2 focus-visible:after:ring-ring/50"
        >
          {board.name}
        </Link>
        <div className="relative z-10 -mt-1.5 -mr-2 flex shrink-0 items-center gap-1">
          <ShareBadges board={board} />
          <BoardActionsMenu board={board} actions={actions} />
        </div>
      </div>
      <p className="line-clamp-2 min-h-8 text-sm text-muted-foreground">
        {board.description || "No description."}
      </p>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span className="tabular-nums">
          {board.chartCount ?? 0} {board.chartCount === 1 ? "tile" : "tiles"}
        </span>
        {board.builtin ? (
          <span>Built-in</span>
        ) : (
          <>
            {board.createdBy ? <span>{board.createdBy}</span> : null}
            {board.updatedAt ? (
              <span>Updated {formatRelativeTime(board.updatedAt)}</span>
            ) : null}
          </>
        )}
      </div>
    </div>
  )
}

/**
 * Halaman DAFTAR dashboard (`/dashboards/browse`) — galeri kartu atau tabel.
 *
 * Dituju dengan sengaja, bukan pintu masuk: `/dashboards` adalah penerus
 * yang membawa ke board terakhir. Halaman ini untuk MENGELOLA daftarnya.
 *
 * Folders belong here, because folders organise dashboards and this is
 * where dashboards are managed: the gallery is sectioned by folder, the
 * table has a Folder column, "Folders" opens the folder manager, "Move to
 * folder…" is in each dashboard's ⋯ menu, and a new dashboard can be filed
 * on creation. They used to live on a dashboard's canvas, the page for
 * charts, where they read as part of managing charts and were invisible
 * here (QA feedback). The canvas keeps only the title switcher, grouped by
 * folder, for moving between dashboards.
 *
 * Kanvas satu dashboard ada di `/dashboards/[id]`. Halaman ini sengaja
 * memuat seluruh board sekaligus: endpoint `/api/dashboard/boards` memang
 * tidak berhalaman, dan jumlah dashboard di satu workspace kecil. Karena
 * itu pencarian/filter dikerjakan di sisi klien.
 *
 * Kolom waktunya dilabeli **Updated**, bukan Created. `console.bi_board`
 * adalah `ReplacingMergeTree(created_at)` sehingga `created_at` adalah
 * kolom versi yang ditulis ulang setiap board disimpan — isinya "terakhir
 * diubah" sejak awal. Menyebutnya "Created" adalah klaim yang salah.
 */
export function DashboardListPage() {
  const router = useRouter()
  const state = useService((s) => dashboardService.listBoards(s), [])
  // Folders are listed to everyone who can see dashboards; filing needs
  // `dashboard:write`, which the API enforces (policy.rs).
  const foldersState = useService((s) => dashboardService.listFolders(s), [])
  const folders = React.useMemo(() => foldersState.data ?? [], [foldersState.data])
  const reloadFolders = foldersState.reload
  const canFile = useAuth().hasPermission("dashboard:write")
  const [view, setView] = React.useState<ViewMode>("gallery")
  const [collapsed, setCollapsed] = React.useState<Set<string>>(new Set())
  const [foldersOpen, setFoldersOpen] = React.useState(false)
  const [moving, setMoving] = React.useState<Board | null>(null)

  // Preferensi tampilan dibaca setelah mount supaya markup server dan
  // klien identik pada render pertama. Dibungkus `try` dengan alasan yang
  // sama seperti `last-board.ts`: di mode privat atau saat kuota penuh,
  // localStorage melempar, dan preferensi tata letak tidak layak
  // menjatuhkan halamannya.
  React.useEffect(() => {
    try {
      const saved = window.localStorage.getItem(VIEW_STORAGE_KEY)
      if (saved === "gallery" || saved === "table") setView(saved)
    } catch {
      /* biarkan galeri sebagai bawaan */
    }
  }, [])
  function changeView(next: ViewMode) {
    setView(next)
    try {
      window.localStorage.setItem(VIEW_STORAGE_KEY, next)
    } catch {
      /* pilihan tetap berlaku untuk sesi ini, hanya tidak diingat */
    }
  }

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
      { success: "Dashboard deleted", error: "Could not delete dashboard" },
      (signal, id: string) => dashboardService.deleteBoard(id, signal),
    ),
  )

  const actions = React.useMemo<BoardRowActions>(
    () => ({
      onRename: setRenaming,
      onDelete: setDeleting,
      onMove: canFile ? setMoving : undefined,
      onDuplicate: (b) => {
        void (async () => {
          const created = await duplicateAction.run(b.id)
          notifyDashboardsChanged()
          reload()
          if (created?.id) router.push(`/dashboards/${created.id}`)
        })()
      },
    }),
    [canFile, duplicateAction, reload, router],
  )
  const folderPathOf = React.useCallback(
    (b: Board) => (b.folderId ? folderPath(folders, b.folderId) : ""),
    [folders],
  )
  const columns = React.useMemo(
    () => getDashboardColumns(actions, folderPathOf),
    [actions, folderPathOf],
  )

  const tableUrlState = useTableUrlState()
  const boards = React.useMemo(() => state.data ?? [], [state.data])
  const filtered = React.useMemo(
    () =>
      filterDataClientSide(boards, {
        search: tableUrlState.search,
        // The folder path is searchable, so typing a folder's name
        // narrows the list to what is filed in it.
        searchFields: [
          (b) => b.name,
          (b) => b.description ?? "",
          (b) => b.createdBy ?? "",
          folderPathOf,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [boards, folderPathOf, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator],
  )

  const { table } = useDataTable({
    data: filtered,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/dashboards/browse",
    initialState: { columnPinning: { right: ["actions"] } },
    getRowId: (row) => row.id,
  })

  // Galeri membaca baris dari tabel, bukan dari `filtered` langsung. Toolbar
  // di atasnya memegang satu kontrol urutan; kalau galeri memakai array
  // mentahnya, kontrol itu berubah jadi tombol mati begitu tampilan diganti.
  //
  // Tanpa `useMemo`: `getRowModel()` sudah di-cache TanStack terhadap state
  // tabel, sehingga membungkusnya lagi hanya menambah daftar dependensi yang
  // tidak bisa dipercaya.
  const visible = table.getRowModel().rows.map((r) => r.original)
  // Empty folders are shown only when nothing narrows the list: under a
  // search or filter an empty section is noise.
  const narrowed = Boolean(tableUrlState.search) || tableUrlState.filters.length > 0
  const sections = groupBoardsByFolder(folders, visible, !narrowed)
  const toggleSection = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (!next.delete(id)) next.add(id)
      return next
    })

  const newButton = (
    <Button size="sm" onClick={() => setCreateOpen(true)}>
      <PlusIcon data-icon="inline-start" />
      New dashboard
    </Button>
  )

  // Tidak ada primitive ToggleGroup di repo ini, jadi dua tombol ikon dengan
  // `aria-pressed` — sama jelasnya bagi screen reader tanpa menambah primitive
  // baru hanya untuk satu halaman.
  const viewToggle = (
    <div role="group" aria-label="Layout" className="flex items-center gap-1">
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant={view === "gallery" ? "secondary" : "ghost"}
              size="icon-sm"
              aria-label="Gallery view"
              aria-pressed={view === "gallery"}
              onClick={() => changeView("gallery")}
            />
          }
        >
          <LayoutGrid />
        </TooltipTrigger>
        <TooltipContent>Gallery view</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant={view === "table" ? "secondary" : "ghost"}
              size="icon-sm"
              aria-label="Table view"
              aria-pressed={view === "table"}
              onClick={() => changeView("table")}
            />
          }
        >
          <Rows3 />
        </TooltipTrigger>
        <TooltipContent>Table view</TooltipContent>
      </Tooltip>
    </div>
  )

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Dashboards"
        description="Every dashboard in this workspace. Open one to edit its tiles, or create a new one."
      />

      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={reload} /> : null}

      {state.status === "success" && boards.length === 0 ? (
        <EmptyState
          title="No dashboards yet"
          description="Create a dashboard to collect charts, KPIs and tables on one canvas."
          action={newButton}
        />
      ) : null}

      {state.status === "success" && boards.length > 0 ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar
            table={table}
            onRefresh={reload}
            trailing={
              <div className="flex items-center gap-2">
                {viewToggle}
                {canFile ? (
                  <Button variant="outline" size="sm" onClick={() => setFoldersOpen(true)}>
                    <FolderCog data-icon="inline-start" />
                    Folders
                  </Button>
                ) : null}
                {newButton}
              </div>
            }
          >
            <DataTableSearch placeholder="Search dashboards..." />
          </DataTableAdvancedToolbar>

          {visible.length === 0 ? (
            <EmptyState
              title="No matching dashboards"
              description="No dashboard matches the current search or filters."
            />
          ) : view === "gallery" ? (
            folders.length === 0 ? (
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {visible.map((b) => (
                  <BoardCard key={b.id} board={b} actions={actions} />
                ))}
              </div>
            ) : (
              <div className="flex flex-col gap-5">
                {sections.map((section) => {
                  const open = !collapsed.has(section.folderId)
                  return (
                    <section key={section.folderId || "none"} className="flex flex-col gap-2.5">
                      <button
                        type="button"
                        aria-expanded={open}
                        onClick={() => toggleSection(section.folderId)}
                        className="flex w-fit items-center gap-2 rounded-md px-1 py-0.5 text-sm font-medium text-foreground transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                      >
                        <ChevronDown
                          className={cn("size-4 text-muted-foreground transition-transform", !open && "-rotate-90")}
                          aria-hidden
                        />
                        {section.folderId ? (
                          <Folder className="size-4 text-muted-foreground" aria-hidden />
                        ) : null}
                        {section.path || "No folder"}
                        <span className="text-xs font-normal text-muted-foreground tabular-nums">
                          {section.boards.length}
                        </span>
                      </button>
                      {open ? (
                        section.boards.length === 0 ? (
                          <p className="px-1 text-xs text-muted-foreground">
                            No dashboards in this folder.
                          </p>
                        ) : (
                          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                            {section.boards.map((b) => (
                              <BoardCard key={b.id} board={b} actions={actions} />
                            ))}
                          </div>
                        )
                      ) : null}
                    </section>
                  )
                })}
              </div>
            )
          ) : (
            <div className="rounded-md border">
              <DataTable table={table} />
            </div>
          )}
        </div>
      ) : null}

      <BoardFormDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        title="New dashboard"
        description="Give it a name now; you can add tiles after it opens."
        submitLabel="Create"
        initialName=""
        initialDescription=""
        folders={canFile ? folders : undefined}
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
        <ManageFoldersDialog
          open={foldersOpen}
          onOpenChange={setFoldersOpen}
          folders={folders}
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
          folders={folders}
          boardId={moving.id}
          currentFolderId={moving.folderId ?? ""}
          onMoved={() => {
            notifyDashboardsChanged()
            reload()
          }}
        />
      ) : null}

      <ConfirmActionDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={`Delete ${deleting?.name ?? "dashboard"}?`}
        description="This deletes the dashboard and every tile on it."
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
            // Kalau board ini yang terakhir dibuka, `/dashboards` tidak boleh
            // menuntun balik ke sesuatu yang sudah tidak ada.
            forgetLastBoard(target.id)
            notifyDashboardsChanged()
            reload()
          })()
        }}
      />
    </div>
  )
}
