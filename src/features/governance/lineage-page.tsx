"use client"

import * as React from "react"
import { PageHeader } from "@/components/patterns/page-header"
import { DataTable, type ColumnDef } from "@/components/patterns/data-table"
import { FlowCanvas } from "@/components/patterns/flow-canvas"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Input } from "@/components/ui/input"
import { useService } from "@/hooks/use-service"
import { governanceService } from "@/services"
import type { LineageEdge, LineageGraph } from "@/services/contracts/governance"

function ConnectionsTable({ graph }: { graph: LineageGraph }) {
  const labelById = React.useMemo(() => {
    const map = new Map(graph.nodes.map((n) => [n.id, n.label]))
    return (id: string) => map.get(id) ?? id
  }, [graph.nodes])

  const columns: ColumnDef<LineageEdge>[] = [
    { key: "from", header: "From", render: (r) => labelById(r.from) },
    { key: "to", header: "To", render: (r) => labelById(r.to) },
    { key: "via", header: "Via", render: (r) => <Pill tone="neutral">{r.kind}</Pill> },
  ]

  return (
    <DataTable
      columns={columns}
      rows={graph.edges}
      rowKey={(r) => r.id}
      emptyMessage="No connections for this asset."
    />
  )
}

export function LineagePage() {
  const [focus, setFocus] = React.useState("tbl-orders-events")
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
      {state.status === "success" ? (
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
              {state.data.columnMappings.map((m, i) => (
                <li key={i} className="font-mono text-xs">
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
