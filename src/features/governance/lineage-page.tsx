"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { FlowCanvas } from "@/components/patterns/flow-canvas"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Input } from "@/components/ui/input"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { governanceService } from "@/services"
import type { LineageGraph } from "@/services/contracts/governance"
import { getLineageColumns } from "./lineage-columns"

function ConnectionsTable({ graph }: { readonly graph: LineageGraph }) {
  const tableUrlState = useTableUrlState()
  const columns = React.useMemo(
    () => getLineageColumns({ nodes: graph.nodes }),
    [graph.nodes]
  )

  const nodeMap = React.useMemo(
    () => new Map(graph.nodes.map((n) => [n.id, n.label])),
    [graph.nodes]
  )

  const filteredData = React.useMemo(() => {
    return filterDataClientSide(graph.edges, {
      search: tableUrlState.search,
      searchFields: [
        (r) => nodeMap.get(r.from) ?? r.from,
        (r) => nodeMap.get(r.to) ?? r.to,
        (r) => r.kind,
      ],
      filters: tableUrlState.filters,
      joinOperator: tableUrlState.joinOperator,
    })
  }, [
    graph.edges,
    nodeMap,
    tableUrlState.search,
    tableUrlState.filters,
    tableUrlState.joinOperator,
  ])

  const { table } = useDataTable({
    data: filteredData,
    columns,
    enableAdvancedFilter: true,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: false,
    manualFiltering: true,
    persistKey: "/governance/lineage",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  return (
    <div className="flex flex-col gap-4">
      <DataTableAdvancedToolbar table={table}>
        <DataTableSearch placeholder="Search source, target, kind…" />
      </DataTableAdvancedToolbar>
      <DataTable table={table} />
    </div>
  )
}

export function LineagePage() {
  const [focus, setFocus] = React.useState("event-pariwisata")
  const state = useService((s) => governanceService.getLineage(focus, s), [focus])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Lineage"
        description="Dataset, pipeline, query, and agent-action lineage with column mappings."
      />
      <Input
        value={focus}
        onChange={(e) => setFocus(e.target.value)}
        className="max-w-sm"
        aria-label="Focus asset id"
        placeholder="Focus asset id"
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && !state.data.supported ? (
        // The build has no lineage capture. Show the API's own reason
        // rather than an empty graph canvas next to it — an empty canvas
        // would read as "this dataset has no lineage" instead of "lineage
        // capture is not implemented".
        <EmptyState title="Lineage not available" description={state.data.reason} />
      ) : null}
      {state.status === "success" && state.data.supported ? (
        <>
          <SectionCard title="Graph">
            <FlowCanvas
              nodes={state.data.nodes.map((n) => ({
                id: n.id,
                label: n.label,
                kind: n.kind,
              }))}
            />
          </SectionCard>
          <SectionCard
            title="Connections"
            description="Table alternative to the graph: every edge with its connection kind."
          >
            <ConnectionsTable graph={state.data} />
          </SectionCard>
          <SectionCard title="Column mappings">
            <ul className="space-y-2 text-sm">
              {state.data.columnMappings.map((m) => (
                <li key={`${m.source}-${m.target}`} className="font-mono text-xs">
                  {m.source} → {m.target}{" "}
                  <span className="text-muted-foreground">({m.transform})</span>
                </li>
              ))}
            </ul>
          </SectionCard>
        </>
      ) : null}
    </div>
  )
}

