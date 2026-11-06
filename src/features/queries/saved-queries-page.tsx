"use client"

import * as React from "react"
import Link from "next/link"
import { Trash2Icon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSkeleton } from "@/components/data-table/data-table-skeleton"
import { CodeBlock } from "@/components/patterns/code-block"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useDataTable } from "@/hooks/use-data-table"
import { useDebouncedCallback } from "@/hooks/use-debounced-callback"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { formatRelativeTime } from "@/lib/format"
import { readableErrorMessage } from "@/lib/notify-message"
import { queryService } from "@/services"
import type { SavedQuery } from "@/services/contracts/queries"
import { QueryStudioTabs } from "./query-studio-tabs"
import { SavedQueryAccessPanel } from "./saved-query-access-panel"
import { SavedQueryFolderTree } from "./saved-query-folder-tree"
import { SavedQueryHistoryDialog } from "./saved-query-history-dialog"
import { SavedQueryMoveDialog } from "./saved-query-move-dialog"
import { SavedQueryRenameDialog } from "./saved-query-rename-dialog"
import { SavedQueryTrashDialog } from "./saved-query-trash-dialog"
import { getSavedQueryColumns, TagPills } from "./saved-query-columns"

/**
 * The level the caller holds on the trash.
 *
 * The server's trash holds only the queries the caller may edit — the same
 * set that may delete and restore one (plan 55 D5, decision 6) — so the
 * console shows a Restore for what the server sent and does not filter the
 * list again (C2).
 */
const TRASH_LEVEL = "edit"

function SavedQueryDrawerContent({ query }: { readonly query: SavedQuery }) {
  return (
    <>
      <CodeBlock>{query.sql}</CodeBlock>
      <MetadataList
        items={[
          { label: "Owner", value: query.owner },
          { label: "Updated", value: formatRelativeTime(query.updatedAt) },
          { label: "Tags", value: <TagPills tags={query.tags} /> },
        ]}
      />
      <Button
        size="sm"
        render={<Link href={`/query-studio?saved=${encodeURIComponent(query.id)}`} />}
      >
        Open in Studio
      </Button>
    </>
  )
}

export function SavedQueriesPage() {
  // The search is the server's to apply (P9): the console may not see every
  // row the server would send, so it asks for the text and shows the answer
  // as it comes (C1).
  const [typed, setTyped] = React.useState("")
  const [q, setQ] = React.useState("")
  const [folder, setFolder] = React.useState<string | undefined>(undefined)
  const pushSearch = useDebouncedCallback((next: string) => setQ(next), 300)

  const state = useService(
    (signal) => queryService.listSavedFiltered({ q: q || undefined, folder }, signal),
    [q, folder],
  )
  const folders = useService((signal) => queryService.listFolders(signal), [])
  // `GET /api/query/scheduling` is a static capability probe: no safe
  // per-principal execution path exists before WS7's policy-obligations
  // engine, so this always resolves `supported: false`. Rendered as plain
  // text, not a schedule control — there is nothing for a control to
  // submit to.
  const scheduling = useService((s) => queryService.getSchedulingCapability(s), [])
  const [selected, setSelected] = React.useState<SavedQuery | null>(null)
  const [renaming, setRenaming] = React.useState<SavedQuery | null>(null)
  const [moving, setMoving] = React.useState<SavedQuery | null>(null)
  const [historyFor, setHistoryFor] = React.useState<SavedQuery | null>(null)
  const [deleting, setDeleting] = React.useState<SavedQuery | null>(null)
  const [sharing, setSharing] = React.useState<SavedQuery | null>(null)
  const [trashOpen, setTrashOpen] = React.useState(false)

  const duplicateAct = useServiceAction((signal, id: string) =>
    queryService.duplicateSaved(id, signal),
  )
  const renameAct = useServiceAction((signal, id: string, title: string) =>
    queryService.updateSaved(id, { title }, signal),
  )
  // A move writes the folder the query lands in: `null` is the shared
  // space, an id is that folder. The server refuses a move below manage,
  // and says so in its own sentence.
  const moveAct = useServiceAction((signal, id: string, folderId: string | null) =>
    queryService.updateSaved(id, { folderId }, signal),
  )
  const deleteAct = useServiceAction((signal, id: string) =>
    queryService.deleteSaved(id, signal).then(() => true),
  )
  // The action runners and the reload never change identity, so the columns
  // below are built once and not on every render.
  const { run: runDuplicate } = duplicateAct
  const { run: runMove, reset: resetMove } = moveAct
  const { run: runDelete } = deleteAct
  const { reload } = state

  const columns = React.useMemo(
    () =>
      getSavedQueryColumns({
        onInspect: (query) => setSelected(query),
        onDuplicate: (query) => {
          void runDuplicate(query.id).then((done) => {
            if (done) reload()
          })
        },
        onRename: (query) => setRenaming(query),
        onMove: (query) => {
          resetMove()
          setMoving(query)
        },
        onHistory: (query) => setHistoryFor(query),
        onDelete: (query) => setDeleting(query),
        onShare: (query) => setSharing(query),
      }),
    [runDuplicate, reload, resetMove],
  )

  const rawData = React.useMemo(() => state.data ?? [], [state.data])
  // A search text or a folder narrows what the server sends, so an empty
  // answer then means "nothing matches", not "nothing saved".
  const narrowed = q !== "" || folder !== undefined

  const tableUrlState = useTableUrlState()
  // The column filters of the advanced toolbar still filter the rows on
  // screen; the text search does not, because that one is the server's.
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(rawData, {
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [rawData, tableUrlState.filters, tableUrlState.joinOperator],
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/query-studio/saved",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  // Each refusal is shown where it happened: the rename and the delete in
  // their own dialog, the duplicate in the banner under the list. A
  // sentence said twice is a sentence nobody reads (C6).
  const errorText = duplicateAct.error
    ? readableErrorMessage(duplicateAct.error.message)
    : null

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Saved Queries"
        description="Reusable SQL in a personal and a shared space, with owners and tags."
        actions={
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setTrashOpen(true)}
            >
              <Trash2Icon data-icon="inline-start" />
              Trash
            </Button>
            <Button size="sm" render={<Link href="/query-studio" />}>
              New query
            </Button>
          </>
        }
      />
      <QueryStudioTabs />
      {scheduling.status === "success" && scheduling.data.supported === false ? (
        <p className="text-sm text-muted-foreground">
          Scheduled queries: not supported yet — {scheduling.data.reason}.
        </p>
      ) : null}
      <div className="flex flex-col gap-4 lg:flex-row">
        <aside className="w-full shrink-0 lg:w-64">
          <Input
            type="search"
            value={typed}
            onChange={(event) => {
              setTyped(event.target.value)
              pushSearch(event.target.value)
            }}
            placeholder="Search saved queries"
            aria-label="Search saved queries"
          />
          <div className="mt-3">
            <SavedQueryFolderTree
              folders={folders.data ?? []}
              selected={folder}
              onSelect={setFolder}
              onChanged={() => {
                folders.reload()
                state.reload()
              }}
            />
          </div>
        </aside>
        <div className="min-w-0 flex-1 space-y-4">
          {errorText ? (
            <div
              role="alert"
              className="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive"
            >
              {errorText}
            </div>
          ) : null}
          {state.status === "loading" ? (
            <DataTableSkeleton columnCount={5} rowCount={6} filterCount={1} />
          ) : null}
          {state.status === "error" ? (
            <ErrorState error={state.error} onRetry={state.reload} />
          ) : null}
          {state.status === "success" && rawData.length === 0 && narrowed ? (
            <EmptyState title="No saved queries match." />
          ) : null}
          {state.status === "success" && rawData.length === 0 && !narrowed ? (
            <EmptyState
              title="No saved queries"
              description="Write a query in the Studio and save it to reuse it here."
              action={
                <Button size="sm" render={<Link href="/query-studio" />}>
                  Open Studio
                </Button>
              }
            />
          ) : null}
          {state.status === "success" && rawData.length > 0 ? (
            <div className="space-y-4">
              <DataTableAdvancedToolbar
                table={table}
                onRefresh={state.reload}
                exportName="Saved queries"
              />
              <div className="rounded-md border">
                <DataTable table={table} />
              </div>
            </div>
          ) : null}
        </div>
      </div>
      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.title ?? "Saved query"}
        wide
      >
        {selected ? <SavedQueryDrawerContent query={selected} /> : null}
      </DetailDrawer>
      {renaming ? (
        <SavedQueryRenameDialog
          key={renaming.id}
          query={renaming}
          onClose={() => setRenaming(null)}
          onRename={(title) => {
            void renameAct.run(renaming.id, title).then((done) => {
              if (done) {
                setRenaming(null)
                state.reload()
              }
            })
          }}
          busy={renameAct.status === "pending"}
          error={
            renameAct.error ? readableErrorMessage(renameAct.error.message) : null
          }
        />
      ) : null}
      {deleting ? (
        <ConfirmActionDialog
          open
          onOpenChange={(open) => {
            if (!open) setDeleting(null)
          }}
          title="Delete query"
          description={`"${deleting.title}" goes to the trash and leaves this list.`}
          confirmLabel="Delete"
          destructive
          confirming={deleteAct.status === "pending"}
          onConfirm={() => {
            void runDelete(deleting.id).then((done) => {
              if (done) {
                setDeleting(null)
                reload()
              }
            })
          }}
        >
          {deleteAct.error ? (
            <p role="alert" className="text-sm text-destructive">
              {readableErrorMessage(deleteAct.error.message)}
            </p>
          ) : null}
        </ConfirmActionDialog>
      ) : null}
      {moving ? (
        <SavedQueryMoveDialog
          key={moving.id}
          folders={folders.data ?? []}
          subject={moving.title}
          currentFolderId={moving.folderId}
          onClose={() => setMoving(null)}
          onMove={(target) => {
            void runMove(moving.id, target).then((done) => {
              if (done) {
                setMoving(null)
                state.reload()
              }
            })
          }}
          busy={moveAct.status === "pending"}
          error={moveAct.error?.message ?? null}
        />
      ) : null}
      {historyFor ? (
        <SavedQueryHistoryDialog
          key={historyFor.id}
          queryId={historyFor.id}
          title={historyFor.title}
          level={historyFor.level}
          onClose={() => setHistoryFor(null)}
          onReverted={() => state.reload()}
        />
      ) : null}
      {trashOpen ? (
        <SavedQueryTrashDialog
          folders={folders.data ?? []}
          level={TRASH_LEVEL}
          onClose={() => setTrashOpen(false)}
          onRestored={() => state.reload()}
        />
      ) : null}
      {sharing ? (
        <SavedQueryAccessPanel
          key={sharing.id}
          kind="query"
          id={sharing.id}
          title={sharing.title}
          onClose={() => setSharing(null)}
        />
      ) : null}
    </div>
  )
}
