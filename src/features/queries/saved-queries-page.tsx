"use client"

import * as React from "react"
import Link from "next/link"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { DataTableSkeleton } from "@/components/data-table/data-table-skeleton"
import { CodeBlock } from "@/components/patterns/code-block"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useDataTable } from "@/hooks/use-data-table"
import { filterDataClientSide } from "@/lib/data-table"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { useService } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { queryService } from "@/services"
import type { SavedQuery } from "@/services/contracts/queries"
import { QueryStudioTabs } from "./query-studio-tabs"
import { getSavedQueryColumns, TagPills } from "./saved-query-columns"

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
  const state = useService((s) => queryService.listSaved(s), [])
  // `GET /api/query/scheduling` is a static capability probe: no safe
  // per-principal execution path exists before WS7's policy-obligations
  // engine, so this always resolves `supported: false`. Rendered as plain
  // text, not a schedule control — there is nothing for a control to
  // submit to.
  const scheduling = useService((s) => queryService.getSchedulingCapability(s), [])
  const [selected, setSelected] = React.useState<SavedQuery | null>(null)

  const columns = React.useMemo(
    () =>
      getSavedQueryColumns({
        onInspect: (query) => setSelected(query),
      }),
    []
  )

  const rawData = React.useMemo(() => state.data ?? [], [state.data])

  const tableUrlState = useTableUrlState()
  const filteredData = React.useMemo(
    () =>
      filterDataClientSide(rawData, {
        search: tableUrlState.search,
        searchFields: [
          (r) => r.title,
          (r) => r.owner,
          (r) => r.tags.join(" "),
        ],
        filters: tableUrlState.filters,
        joinOperator: tableUrlState.joinOperator,
      }),
    [rawData, tableUrlState.search, tableUrlState.filters, tableUrlState.joinOperator]
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

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Saved Queries"
        description="Reusable SQL shared with the team, with owners and tags."
        actions={
          <Button size="sm" render={<Link href="/query-studio" />}>
            New query
          </Button>
        }
      />
      <QueryStudioTabs />
      {scheduling.status === "success" && scheduling.data.supported === false ? (
        <p className="text-sm text-muted-foreground">
          Scheduled queries: not supported yet — {scheduling.data.reason}.
        </p>
      ) : null}
      {state.status === "loading" ? (
        <DataTableSkeleton columnCount={5} rowCount={6} filterCount={1} />
      ) : null}
      {state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}
      {state.status === "success" && rawData.length === 0 ? (
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
          >
            <DataTableSearch placeholder="Search title, owner..." />
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
        title={selected?.title ?? "Saved query"}
        wide
      >
        {selected ? <SavedQueryDrawerContent query={selected} /> : null}
      </DetailDrawer>
    </div>
  )
}
