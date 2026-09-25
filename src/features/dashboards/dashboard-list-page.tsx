"use client"

import * as React from "react"
import Link from "next/link"
import { useRouter } from "next/navigation"
import { LayoutGrid, PlusIcon, Rows3 } from "lucide-react"

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
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatRelativeTime } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { dashboardService } from "@/services"
import type { Board } from "@/services/clients/bi-store"
import { notifyDashboardsChanged } from "./dashboard-events"
import { getDashboardColumns, ShareBadges } from "./dashboard-list-columns"
import { forgetLastBoard } from "./last-board"

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
  readonly busy: boolean
  readonly onSubmit: (values: { name: string; description: string }) => void
}) {
  const [name, setName] = React.useState(initialName)
  const [desc, setDesc] = React.useState(initialDescription)

  // Reset tiap kali dibuka: dialog ini dipakai ulang untuk baris yang
  // berbeda, jadi state lama tidak boleh bocor ke board berikutnya.
  React.useEffect(() => {
    if (open) {
      setName(initialName)
      setDesc(initialDescription)
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
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            disabled={!canSubmit || busy}
            onClick={() => onSubmit({ name: clean, description: desc.trim() })}
          >
            {busy ? "Working…" : submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Kartu satu dashboard untuk tampilan galeri. */
function BoardCard({ board }: { readonly board: Board }) {
  return (
    <Link
      href={`/dashboards/${board.id}`}
      className={cn(
        "group flex flex-col gap-3 rounded-xl border bg-card p-4 text-left",
        "shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)] transition-colors hover:border-primary/40",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <span className="min-w-0 font-medium group-hover:underline">{board.name}</span>
        <ShareBadges board={board} />
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
    </Link>
  )
}

/**
 * Halaman DAFTAR dashboard (`/dashboards/browse`) — galeri kartu atau tabel.
 *
 * Dituju dengan sengaja, bukan pintu masuk: `/dashboards` adalah penerus
 * yang membawa ke board terakhir. Halaman ini untuk MENGELOLA daftarnya.
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
  const [view, setView] = React.useState<ViewMode>("gallery")

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
      (signal, input: { name: string; description: string }) =>
        dashboardService.createBoard(input, signal),
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

  const columns = React.useMemo(
    () =>
      getDashboardColumns({
        onRename: setRenaming,
        onDelete: setDeleting,
        onDuplicate: (b) => {
          void (async () => {
            const created = await duplicateAction.run(b.id)
            notifyDashboardsChanged()
            reload()
            if (created?.id) router.push(`/dashboards/${created.id}`)
          })()
        },
      }),
    [duplicateAction, reload, router],
  )

  const tableUrlState = useTableUrlState()
  const boards = React.useMemo(() => state.data ?? [], [state.data])
  const filtered = React.useMemo(
    () =>
      filterDataClientSide(boards, {
        search: tableUrlState.search,
        searchFields: [(b) => b.name, (b) => b.description ?? "", (b) => b.createdBy ?? ""],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [boards, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator],
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
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {visible.map((b) => (
                <BoardCard key={b.id} board={b} />
              ))}
            </div>
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
        busy={createAction.status === "pending"}
        onSubmit={({ name, description }) => {
          void (async () => {
            const created = await createAction.run({ name, description })
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
