"use client"

import * as React from "react"
import Link from "next/link"
import { ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { LINEAGE_REFUSED, refreshWords, STEP_KIND_LABEL } from "@/lib/lineage-words"
import { dashboardService } from "@/services"
import { useReporting } from "./reporting-context"

/**
 * "Where does this come from" (`BI-14` feature decision 6, plan D7): a side
 * panel with the chart's path as a vertical list, the chart first and the
 * connector or upload last, each step with its kind and when it was last
 * refreshed ("Not recorded" where no record exists), then the columns behind
 * each field where column lineage was recorded, and a link to the full lineage
 * page focused on the chart.
 *
 * Needs `lineage:read` (owner decision Q6): the menu item is shown to everyone
 * who can see the chart, and a person without the permission gets the sentence
 * instead of the path; nothing is asked for. The route
 * (`GET /api/dashboard/charts/{id}/lineage`, T2 of the BI-14 plan) is not on
 * the server yet; until it is, the panel shows its read error.
 */
export function ChartLineagePanel({
  chart, onOpenChange,
}: {
  /** The chart whose path is shown; `null` keeps the panel closed. */
  readonly chart: { id: string; title: string } | null
  readonly onOpenChange: (open: boolean) => void
}) {
  const { hasPermission } = useAuth()
  const { timeZone } = useReporting()
  const allowed = hasPermission("lineage:read")
  const id = chart?.id ?? ""
  const state = useService((s) => (allowed && id ? dashboardService.chartLineage(id, s) : Promise.resolve(null)), [allowed, id])
  const refused = !allowed || (state.status === "error" && state.error.code === "permission_denied")

  return (
    <Sheet open={chart !== null} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full overflow-y-auto sm:max-w-md">
        <SheetHeader>
          <SheetTitle>Where does this come from</SheetTitle>
          <SheetDescription>{chart?.title}</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-4 px-4 pb-4">
          {refused ? (
            <p role="status" className="text-sm text-muted-foreground">{LINEAGE_REFUSED}</p>
          ) : state.status === "loading" ? (
            <Skeleton className="h-40 w-full" />
          ) : state.status === "error" ? (
            <ErrorState error={state.error} onRetry={state.reload} />
          ) : state.data && !state.data.supported ? (
            <p role="status" className="text-sm text-muted-foreground">Lineage is not recorded on this deployment.</p>
          ) : state.data ? (
            <>
              <ol aria-label="Path from the chart to its sources" className="flex flex-col gap-3">
                {state.data.steps.map((step) => (
                  <li key={step.id} className="rounded-md border p-3" data-step-kind={step.kind}>
                    <p className="text-xs text-muted-foreground">{STEP_KIND_LABEL[step.kind] ?? step.kind}</p>
                    <p className="text-sm font-medium">{step.label}</p>
                    <p className={step.notRecorded || !step.refreshedAt ? "text-xs text-muted-foreground" : "text-xs"}>{refreshWords(step, timeZone)}</p>
                  </li>
                ))}
              </ol>
              {state.data.fields.length > 0 ? (
                <section aria-label="Columns behind each field" className="flex flex-col gap-2">
                  <h3 className="text-sm font-medium">Columns behind each field</h3>
                  {state.data.fields.map((f) => (
                    <div key={f.field} className="text-sm">
                      <p className="font-medium">{f.field}</p>
                      {f.columns.length === 0 ? (
                        <p className="text-xs text-muted-foreground">No column mapping recorded</p>
                      ) : (
                        <ul className="font-mono text-xs text-muted-foreground">
                          {f.columns.map((c) => <li key={`${c.table}.${c.column}:${c.via}`}>{c.table}.{c.column} <span className="font-sans">({c.via})</span></li>)}
                        </ul>
                      )}
                    </div>
                  ))}
                </section>
              ) : null}
              {state.data.coverage.length > 0 ? (
                <ul className="list-disc pl-5 text-xs text-muted-foreground">
                  {state.data.coverage.map((c) => <li key={c}>{c}</li>)}
                </ul>
              ) : null}
              <Button variant="outline" size="sm" className="w-fit" render={<Link href={`/lineage?focus=${encodeURIComponent(`chart:${state.data.chart.id}`)}`} />}>
                Open in lineage
              </Button>
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  )
}
