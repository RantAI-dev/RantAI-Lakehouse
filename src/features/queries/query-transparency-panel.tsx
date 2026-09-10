"use client"

import { StatusBadge } from "@/components/patterns/status-badge"
import { formatBytes } from "@/lib/format"
import type { QueryPlanStage } from "@/services/contracts/queries"

/**
 * Simple federated / multi-source plan — product language first.
 *
 * `plan` is `null` when no real plan was computed (both `/api/query/run` and
 * `/api/query/estimate` currently emit `null` here — see WS1 task 1.6). Null
 * renders an honest "not measured" line rather than an empty list, which
 * would read as "we checked and there were no stages."
 */
export function QueryPlanPanel({ plan }: { plan: QueryPlanStage[] | null }) {
  if (plan === null) {
    return (
      <div className="mt-3 space-y-1">
        <p className="text-xs font-medium text-muted-foreground">Execution plan</p>
        <p className="text-xs text-muted-foreground">Not measured.</p>
      </div>
    )
  }
  if (plan.length === 0) return null
  return (
    <div className="mt-3 space-y-2">
      <p className="text-xs font-medium text-muted-foreground">Execution plan</p>
      <ol className="space-y-2 border-l border-border pl-3">
        {plan.map((stage) => (
          <li key={stage.id} className="relative text-sm">
            <span className="absolute -left-[17px] top-1.5 size-2 rounded-full bg-muted-foreground/50" />
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{stage.label}</span>
              {stage.status ? <StatusBadge status={stage.status} /> : null}
            </div>
            <p className="text-xs text-muted-foreground">
              {stage.location} · {stage.operation}
              {stage.estimatedBytes != null
                ? ` · ~${formatBytes(stage.estimatedBytes)}`
                : ""}
            </p>
          </li>
        ))}
      </ol>
    </div>
  )
}
