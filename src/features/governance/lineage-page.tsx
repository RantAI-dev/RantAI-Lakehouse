"use client"

import * as React from "react"
import { useSearchParams } from "next/navigation"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Input } from "@/components/ui/input"
import { useDataTable } from "@/hooks/use-data-table"
import { useDebounce } from "@/hooks/use-debounce"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { governanceService } from "@/services"
import type { LineageGraph } from "@/services/contracts/governance"
import { getLineageColumns } from "./lineage-columns"
import { LineageColumns } from "./lineage-graph"

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
  // Links from an asset or a pipeline run name what to focus on
  // (`/lineage?focus=<id>`); it used to be ignored for a fixed demo id.
  const searchParams = useSearchParams()
  const urlFocus = searchParams.get("focus") ?? ""
  const [focus, setFocus] = React.useState(urlFocus)
  // Following a link to this page from this page (the sidebar entry,
  // another `?focus=`) changes only the URL: the page stays mounted, so
  // the URL's focus is taken over here, during render.
  const [seenUrlFocus, setSeenUrlFocus] = React.useState(urlFocus)
  if (urlFocus !== seenUrlFocus) {
    setSeenUrlFocus(urlFocus)
    setFocus(urlFocus)
  }
  // Not one request per keystroke.
  const query = useDebounce(focus.trim(), 300)
  const state = useService((s) => governanceService.getLineage(query, s), [query])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Lineage"
        description="Where data comes from and where it goes: connector ingests and pipelines, with column mappings. Leave the focus empty to see everything."
      />
      <Input
        value={focus}
        onChange={(e) => setFocus(e.target.value)}
        className="max-w-sm"
        aria-label="Focus asset id"
        placeholder="Pipeline, table (silver.orders), catalog asset or connector id"
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
      {state.status === "success" && state.data.supported && state.data.nodes.length === 0 ? (
        <EmptyState
          title={query ? `No lineage recorded for ${query}` : "No lineage recorded yet"}
          description={state.data.note}
        />
      ) : null}
      {state.status === "success" && state.data.supported && state.data.nodes.length > 0 ? (
        <>
          <SectionCard title="Graph" description={state.data.note}>
            <LineageColumns graph={state.data} onTrace={setFocus} />
          </SectionCard>
          <SectionCard
            title="Connections"
            description="Table alternative to the graph: every edge with its connection kind."
          >
            <ConnectionsTable graph={state.data} />
          </SectionCard>
          <SectionCard title="Column mappings">
            {state.data.columnMappings.length === 0 ? (
              <p className="text-xs text-muted-foreground">No pipeline in this graph maps columns.</p>
            ) : null}
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

