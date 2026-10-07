"use client"

import { useCallback, useMemo, useState } from "react"
import Link from "next/link"
import { usePathname, useRouter, useSearchParams } from "next/navigation"
import { PlusIcon, UploadIcon } from "lucide-react"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { PageHeader } from "@/components/patterns/page-header"
import {
  EmptyState,
  ErrorState,
  LoadingSkeleton,
} from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { connectorService } from "@/services"
import { getConnectorColumns } from "./connectors-columns"
import { UploadsPanel } from "./uploads-panel"

/**
 * The "Connectors" tab: the list. A connector opens on its own page,
 * `/connectors/<id>`, from its name, the row menu, or a press anywhere on its
 * row (plan `docs/superpowers/plans/2026-10-05-connector-detail-page.md`).
 */
function ConnectorsTab() {
  const router = useRouter()
  const state = useService((s) => connectorService.listConnectors(s), [])

  const columns = useMemo(() => getConnectorColumns(), [])

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
            <DataTable
              table={table}
              onRowClick={(connector) => router.push(`/connectors/${connector.id}`)}
            />
          </div>
        </div>
      ) : null}
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
