"use client"

import Link from "next/link"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Skeleton } from "@/components/ui/skeleton"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { hiddenWords, LINEAGE_REFUSED } from "@/lib/lineage-words"
import { governanceService } from "@/services"

const VIA_WORDS: Record<string, string> = { direct: "reads the table", sql_source: "through a SQL source", metric: "through a metric" }

/**
 * "Used by" (`BI-14` feature decision 5, plan D5): the charts and dashboards
 * that read a table, directly or through a SQL source or a metric, as many as
 * the person can see (up to 200), with the count of those they cannot. Needs
 * `lineage:read`: without it the owner's sentence (Q6) is shown and nothing is
 * asked for. The route is not on the server yet (T3 of the BI-14 plan); until
 * then this shows the read error, and no list.
 */
export function UsedBy({ table, size = "sm" }: { readonly table: string; readonly size?: "sm" | "default" }) {
  const { hasPermission } = useAuth()
  const allowed = hasPermission("lineage:read")
  const state = useService((s) => (allowed && table ? governanceService.getUsedBy(table, s) : Promise.resolve(null)), [allowed, table])
  const refused = !allowed || (state.status === "error" && state.error.code === "permission_denied")
  return (
    <SectionCard size={size} title="Used by" description="Charts and dashboards that read this table, directly or through a SQL source or a metric.">
      {refused ? (
        <p role="status" className="text-sm text-muted-foreground">{LINEAGE_REFUSED}</p>
      ) : state.status === "loading" ? (
        <Skeleton className="h-16 w-full" />
      ) : state.status === "error" ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : state.data && !Array.isArray(state.data.charts) ? (
        // An answer without the list is no answer: say so rather than draw an empty list (principle 2).
        <p role="status" className="text-sm text-muted-foreground">Used by could not be read just now.</p>
      ) : state.data && !state.data.supported ? (
        <EmptyState title="Not available" description="Chart lineage is not recorded on this deployment." className="py-4" />
      ) : state.data && state.data.charts.length === 0 && state.data.hiddenCount === 0 ? (
        <EmptyState title="No chart reads this table" className="py-4" />
      ) : state.data ? (
        <div className="flex flex-col gap-2">
          <ul className="flex flex-col gap-1 text-sm">
            {state.data.charts.map((c) => (
              <li key={`${c.dashboardId}:${c.chartId}`}>
                <Link href={`/dashboards?board=${encodeURIComponent(c.dashboardId)}`} className="text-primary hover:underline">
                  {c.chartTitle}
                </Link>{" "}
                <span className="text-muted-foreground">
                  on {c.dashboardTitle}, {VIA_WORDS[c.via] ?? c.via}
                </span>
              </li>
            ))}
          </ul>
          {state.data.truncated ? <p className="text-xs text-muted-foreground">Showing the first {state.data.charts.length.toLocaleString("en-US")} charts.</p> : null}
          {state.data.hiddenCount > 0 ? <p className="text-xs text-muted-foreground">{hiddenWords(state.data.hiddenCount)}</p> : null}
        </div>
      ) : null}
    </SectionCard>
  )
}
