"use client"

import * as React from "react"
import Link from "next/link"
import { useSearchParams } from "next/navigation"
import { ChevronRight, ExternalLink } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable } from "@/components/data-table/data-table"
import { DataTableAdvancedToolbar } from "@/components/data-table/data-table-advanced-toolbar"
import { DataTableSearch } from "@/components/data-table/data-table-search"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { StatusBadge } from "@/components/patterns/status-badge"
import { Input } from "@/components/ui/input"
import { useDataTable } from "@/hooks/use-data-table"
import { useDebounce } from "@/hooks/use-debounce"
import { useService } from "@/hooks/use-service"
import { useTableUrlState } from "@/hooks/use-table-url-state"
import { filterDataClientSide } from "@/lib/data-table"
import { ENTITY_STATUS_LABEL, type EntityStatus } from "@/lib/status"
import { cn } from "@/lib/utils"
import { governanceService } from "@/services"
import type { LineageGraph, LineageNode } from "@/services/contracts/governance"
import { getLineageColumns } from "./lineage-columns"

const KIND_LABEL: Record<string, string> = {
  source: "Source",
  bronze: "Bronze",
  silver: "Silver",
  serving: "Serving (gold)",
  pipeline: "Pipeline",
}

/** The page a node opens, when it has one. */
function nodeHref(node: LineageNode): string | null {
  if (!node.ref) return null
  const ref = encodeURIComponent(node.ref)
  switch (node.kind) {
    case "pipeline":
      return `/pipelines/${ref}`
    case "source":
      return `/connectors/${ref}/edit`
    case "bronze":
    case "silver":
    case "serving":
      return `/data/assets/${ref}`
    default:
      return null
  }
}

function isEntityStatus(value: string): value is EntityStatus {
  return value in ENTITY_STATUS_LABEL
}

/**
 * The graph as columns, left to right: a node's column is its longest path
 * from something with nothing upstream (`depth`, from the API), so every
 * arrow points right. Branches stack within a column; the Connections
 * table below names every edge exactly.
 */
function LineageColumns({
  graph,
  onTrace,
}: {
  readonly graph: LineageGraph
  readonly onTrace: (id: string) => void
}) {
  const focusIds = new Set(graph.focusIds ?? [])
  const columns: LineageNode[][] = []
  for (const node of graph.nodes) {
    const depth = node.depth ?? 0
    const column = columns[depth] ?? []
    column.push(node)
    columns[depth] = column
  }
  const filled = columns.filter((c) => c && c.length > 0)
  return (
    <div
      className="flex items-stretch gap-2 overflow-x-auto rounded-lg border border-border bg-muted/20 p-4"
      role="list"
      aria-label="Lineage graph"
    >
      {filled.map((column, i) => (
        <React.Fragment key={column[0].id}>
          <div className="flex shrink-0 flex-col justify-center gap-2">
            {column.map((node) => {
              const href = nodeHref(node)
              return (
                <div
                  key={node.id}
                  role="listitem"
                  className={cn(
                    "flex w-52 flex-col gap-1 rounded-lg border border-border bg-card px-3 py-2.5 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]",
                    focusIds.has(node.id) && "border-primary ring-1 ring-primary/30"
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                      {KIND_LABEL[node.kind] ?? node.kind}
                    </span>
                    {href ? (
                      <Link
                        href={href}
                        className="text-muted-foreground hover:text-foreground"
                        aria-label={`Open ${node.label}`}
                      >
                        <ExternalLink className="size-3.5" />
                      </Link>
                    ) : null}
                  </div>
                  <button
                    type="button"
                    onClick={() => onTrace(node.id)}
                    className="truncate text-left text-sm font-medium leading-5 text-foreground hover:text-primary"
                    title={`Trace lineage from ${node.label}`}
                  >
                    {node.label}
                  </button>
                  {node.kind === "pipeline" && node.sublabel && isEntityStatus(node.sublabel) ? (
                    <StatusBadge status={node.sublabel} className="self-start" />
                  ) : node.sublabel ? (
                    <span className="truncate text-xs text-muted-foreground">{node.sublabel}</span>
                  ) : null}
                </div>
              )
            })}
          </div>
          {i < filled.length - 1 ? (
            <ChevronRight className="size-4 shrink-0 self-center text-muted-foreground" aria-hidden />
          ) : null}
        </React.Fragment>
      ))}
    </div>
  )
}

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

