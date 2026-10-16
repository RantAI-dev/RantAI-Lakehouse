"use client"

import * as React from "react"
import Link from "next/link"
import { Plus } from "lucide-react"

import { PageHeader } from "@/components/patterns/page-header"
import { EmptyState, ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import { metricService } from "@/services"
import type { Metric } from "@/services/contracts/metrics"
import { MetricDialog } from "./metric-dialog"
import { SegmentsDialog } from "./segments-dialog"

/**
 * Reusable metrics (BI-2): the list, with what each is computed from and how many charts use it.
 * Anyone who can see dashboards can read it; defining and changing need `metric:write`, which the
 * server decides (a refusal is shown in plain words in the editor).
 */
export function MetricsPage() {
  const state = useService((signal) => metricService.list(signal), [])
  const [editing, setEditing] = React.useState<Metric | "new" | null>(null)
  const [segmentsOpen, setSegmentsOpen] = React.useState(false)
  const metrics = state.data ?? []

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Metrics"
        description="One definition of a number, used by every chart, KPI and the assistant."
        actions={
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" render={<Link href="/dashboards/browse" />}>Dashboards</Button>
            <Button variant="outline" size="sm" onClick={() => setSegmentsOpen(true)}>Segments</Button>
            <Button size="sm" onClick={() => setEditing("new")}><Plus data-icon="inline-start" />New metric</Button>
          </div>
        }
      />
      {state.status === "loading" ? <LoadingSkeleton /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" && metrics.length === 0 ? (
        <EmptyState title="No metrics yet" description="Define a number once, such as total visitors, and pick it by name in any chart." />
      ) : null}
      {state.status === "success" && metrics.length > 0 ? (
        <div className="overflow-x-auto rounded-md border border-border">
          <table className="w-full border-collapse text-sm">
            <thead className="bg-muted/40 text-left text-xs text-muted-foreground">
              <tr>
                <th className="px-3 py-2 font-medium">Metric</th>
                <th className="px-3 py-2 font-medium">Description</th>
                <th className="px-3 py-2 font-medium">Source</th>
                <th className="px-3 py-2 text-right font-medium">Used by</th>
              </tr>
            </thead>
            <tbody>
              {metrics.map((m) => (
                <tr key={m.id} className="cursor-pointer border-t border-border hover:bg-muted/30" onClick={() => setEditing(m)}>
                  <td className="px-3 py-2">
                    <button type="button" className="text-left font-medium hover:underline" onClick={() => setEditing(m)}>{m.label || m.name}</button>
                    <div className="font-mono text-xs text-muted-foreground">{m.name}</div>
                  </td>
                  <td className="max-w-md px-3 py-2 text-muted-foreground"><span className="line-clamp-2">{m.description}</span></td>
                  <td className="px-3 py-2">{m.sourceTitle ?? m.sourceId}</td>
                  <td className="px-3 py-2 text-right tabular-nums">{m.usedBy ?? 0} {m.usedBy === 1 ? "chart" : "charts"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      <MetricDialog
        open={editing !== null}
        onOpenChange={(o) => { if (!o) setEditing(null) }}
        existing={editing === "new" ? undefined : (editing ?? undefined)}
        onChanged={() => { setEditing(null); state.reload() }}
      />
      <SegmentsDialog open={segmentsOpen} onOpenChange={setSegmentsOpen} />
    </div>
  )
}
