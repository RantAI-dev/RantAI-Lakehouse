"use client"

import { useCallback, useMemo, useState } from "react"
import Link from "next/link"
import { usePathname, useSearchParams } from "next/navigation"
import { PlusIcon, UploadIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { HealthBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useDataTable } from "@/hooks/use-data-table"
import { useService, useServiceAction } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { withNotify } from "@/lib/notify"
import { connectorService } from "@/services"
import type { Connector } from "@/services/contracts/connectors"
import { ConnectorDeleteDialog } from "./connector-delete-dialog"
import { ConnectorIngestPanel } from "./connector-ingest-panel"
import { ConnectorOverview } from "./connector-overview"
import { ConnectorProbeHistoryPanel } from "./connector-probe-history-panel"
import { DIRECTION_LABEL, getConnectorColumns } from "./connectors-columns"
import { UploadsPanel } from "./uploads-panel"

type DrawerTab = "overview" | "ingest" | "tests"

/**
 * Drawer body — fetches full connector detail for the selected row. The
 * status and actions stay on top; below, one tab each for the overview,
 * what the connector ingests (kept mounted, so tables picked but not yet
 * saved survive a look at another tab) and its connection tests.
 */
function ConnectorDetail({
  id,
  onDeleted,
}: {
  readonly id: string
  readonly onDeleted: () => void
}) {
  const state = useService((s) => connectorService.getConnector(id, s), [id])
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [tab, setTab] = useState<DrawerTab>("overview")
  const testAction = useServiceAction(
    withNotify(
      { success: "Connection test passed", error: "Connection test failed" },
      (signal, connectorId: string) =>
        connectorService.testConnection(connectorId, signal)
    )
  )
  const [historyKey, setHistoryKey] = useState(0)

  if (state.status === "loading") return <LoadingSkeleton rows={4} />
  if (state.status === "error")
    return <ErrorState error={state.error} onRetry={state.reload} />
  const c = state.data
  const inUse = c.dependentPipelines.length

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <HealthBadge health={c.health} />
        <Pill tone="neutral">{DIRECTION_LABEL[c.direction]}</Pill>
        {c.environment ? <Pill tone="neutral">{c.environment}</Pill> : null}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={testAction.status === "pending"}
          onClick={async () => {
            await testAction.run(id)
            state.reload()
            setHistoryKey((k) => k + 1)
          }}
        >
          {testAction.status === "pending" ? "Testing…" : "Test connection"}
        </Button>
        <Button size="sm" variant="outline" render={<Link href={`/connectors/${id}/edit`} />}>
          Edit
        </Button>
        <Button size="sm" render={<Link href={`/pipelines/create?connectorId=${id}`} />}>
          Create pipeline
        </Button>
        {c.auditEventId ? (
          <Button
            size="sm"
            variant="ghost"
            render={<Link href={`/audit?event=${c.auditEventId}`} />}
          >
            Audit
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          className="text-destructive hover:text-destructive"
          disabled={inUse > 0}
          title={
            inUse > 0
              ? `Used by ${inUse} pipeline${inUse === 1 ? "" : "s"} (see Used by); those must be deleted or moved first`
              : undefined
          }
          onClick={() => setDeleteOpen(true)}
        >
          Delete
        </Button>
      </div>
      <ConnectorDeleteDialog
        connector={c}
        dependents={c.dependentPipelines}
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        onDeleted={onDeleted}
      />
      {testAction.data ? (
        <p
          className={
            !testAction.data.supported
              ? "text-sm text-muted-foreground"
              : testAction.data.ok
                ? "text-sm text-emerald-600 dark:text-emerald-400"
                : "text-sm text-destructive"
          }
        >
          {testAction.data.supported ? (
            <>
              {testAction.data.message}
              {testAction.data.latencyMs !== null ? ` · ${testAction.data.latencyMs} ms` : ""}
            </>
          ) : (
            <>Not testable · {testAction.data.message}</>
          )}
        </p>
      ) : null}
      <Tabs value={tab} onValueChange={(v) => setTab(v as DrawerTab)} className="gap-4">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="ingest">Ingest</TabsTrigger>
          <TabsTrigger value="tests">Connection tests</TabsTrigger>
        </TabsList>
        <TabsContent value="overview">
          {/* Remounted on every visit (and after a test), so it never
              shows a schedule or a run from before a change elsewhere. */}
          <ConnectorOverview key={historyKey} detail={c} onOpenTab={setTab} />
        </TabsContent>
        <TabsContent value="ingest" keepMounted>
          <ConnectorIngestPanel connectorId={id} connectorName={c.name} />
        </TabsContent>
        <TabsContent value="tests">
          <ConnectorProbeHistoryPanel connectorId={id} refreshKey={historyKey} />
        </TabsContent>
      </Tabs>
    </>
  )
}

/**
 * The "Connectors" tab: what the page showed before it had tabs, unchanged.
 */
function ConnectorsTab() {
  const state = useService((s) => connectorService.listConnectors(s), [])
  const [selected, setSelected] = useState<Connector | null>(null)

  const columns = useMemo(
    () => getConnectorColumns({ onSelect: setSelected }),
    []
  )

  const tableUrlState = useTableUrlState()
  const filteredData = useMemo(
    () =>
      filterDataClientSide(state.data ?? [], {
        search: tableUrlState.search,
        searchFields: [
          (c) => c.name,
          (c) => c.type,
          (c) => c.tenant,
          (c) => c.environment,
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [state.data, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
  )

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/connectors",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
    getRowId: (row) => row.id,
  })

  return (
    <div className="flex flex-col gap-4">
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) === 0 ? (
        <EmptyState
          title="No connectors"
          description="Add a source or sink to start ingesting and delivering data."
          action={
            <Button size="sm" render={<Link href="/connectors/create" />}>
              New Connector
            </Button>
          }
        />
      ) : null}
      {state.status === "success" && (state.data?.length ?? 0) > 0 ? (
        <div className="space-y-4">
          <DataTableAdvancedToolbar table={table} onRefresh={state.reload}>
            <DataTableSearch
              placeholder="Search connectors..."
            />
          </DataTableAdvancedToolbar>
          <div className="rounded-md border">
            <DataTable table={table} />
          </div>
        </div>
      ) : null}

      <DetailDrawer
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelected(null)
        }}
        title={selected?.name ?? ""}
        description={selected?.type}
        wide
      >
        {selected ? (
          <ConnectorDetail
            id={selected.id}
            onDeleted={() => {
              setSelected(null)
              state.reload()
            }}
          />
        ) : null}
      </DetailDrawer>
    </div>
  )
}

type SourcesTab = "connectors" | "uploads"

/**
 * Sources: the connectors, and the files uploaded into raw tables. The open
 * tab lives in `?tab=uploads` (the connectors tab is the bare address), so a
 * refresh or a shared link lands on it. Plan T11, feature page
 * `docs/core/features/upload-file.md`.
 */
export function ConnectorsPage() {
  const pathname = usePathname()
  const searchParams = useSearchParams()
  const urlTab: SourcesTab = searchParams.get("tab") === "uploads" ? "uploads" : "connectors"
  const [tab, setTab] = useState<SourcesTab>(urlTab)
  // A link to the other tab of this same page changes only the URL: the
  // page stays mounted, so the URL's tab is taken over here, during render.
  const [seenUrlTab, setSeenUrlTab] = useState(urlTab)
  if (urlTab !== seenUrlTab) {
    setSeenUrlTab(urlTab)
    setTab(urlTab)
  }

  const selectTab = useCallback(
    (next: SourcesTab) => {
      setTab(next)
      const params = new URLSearchParams(searchParams.toString())
      if (next === "connectors") params.delete("tab")
      else params.set("tab", next)
      const query = params.toString()
      // The History API rather than the router: no server round trip.
      window.history.replaceState(null, "", query ? `${pathname}?${query}` : pathname)
    },
    [pathname, searchParams]
  )

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Connectors"
        description="Sources and sinks for CDC, messaging, object storage, SaaS, and federation. Data enters the platform here before processing."
        actions={
          <>
            <Button variant="outline" size="sm" render={<Link href="/connectors/upload" />}>
              <UploadIcon data-icon="inline-start" />
              Upload file
            </Button>
            <Button size="sm" render={<Link href="/connectors/create" />}>
              <PlusIcon data-icon="inline-start" />
              New Connector
            </Button>
          </>
        }
      />
      <Tabs value={tab} onValueChange={(v) => selectTab(v as SourcesTab)} className="gap-4">
        <TabsList>
          <TabsTrigger value="connectors">Connectors</TabsTrigger>
          <TabsTrigger value="uploads">Uploaded files</TabsTrigger>
        </TabsList>
        <TabsContent value="connectors">
          <ConnectorsTab />
        </TabsContent>
        <TabsContent value="uploads">
          <UploadsPanel />
        </TabsContent>
      </Tabs>
    </div>
  )
}
