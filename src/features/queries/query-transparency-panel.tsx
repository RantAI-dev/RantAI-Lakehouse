"use client"

import type { ReactNode } from "react"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { Pill, StatusBadge } from "@/components/patterns/status-badge"
import { SectionCard } from "@/components/patterns/section-card"
import { Skeleton } from "@/components/ui/skeleton"
import type { ActionState } from "@/hooks/use-service"
import { formatBytes, formatCost } from "@/lib/format"
import { ENGINE_CATEGORY_LABEL, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type { QueryEstimate, QueryPlanStage } from "@/services/contracts/queries"

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-2">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="text-right">{children}</dd>
    </div>
  )
}

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

/** Pre-run estimate panel: scan, cost range, routing, freshness, policy. */
export function QueryTransparencyPanel({
  state,
  estimatable,
  onRetry,
}: {
  state: ActionState<QueryEstimate>
  /** Whether the editor holds a statement worth estimating. */
  estimatable?: boolean
  onRetry: () => void
}) {
  return (
    <SectionCard
      title="Execution transparency"
      description="Pre-run estimate. Engine categories stay product-neutral."
    >
      {/* An empty editor gets no estimate and says so, rather than
          reporting a confident "0 B · 1.00 cu · Fresh · 0 s" about
          nothing. */}
      {state.status === "idle" || estimatable === false ? (
        <p className="text-sm text-muted-foreground">
          Type SQL for a pre-run estimate.
        </p>
      ) : null}
      {state.status === "error" ? (
        <p className="text-sm text-muted-foreground">
          Estimate unavailable.{" "}
          <button
            type="button"
            onClick={onRetry}
            className="text-primary underline-offset-4 hover:underline"
          >
            Retry
          </button>
        </p>
      ) : null}
      {state.status === "pending" && !state.data ? (
        <div className="space-y-2" role="status" aria-label="Estimating">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-3/4" />
          <Skeleton className="h-4 w-5/6" />
        </div>
      ) : null}
      {/* A query ClickHouse cannot even plan has no estimate — and its
          reason (unknown column, missing table) is the most useful thing
          on this panel, so it is shown instead of a row of zeros. */}
      {state.data?.error && estimatable !== false ? (
        <div className="space-y-1">
          <p className="text-sm font-medium">This query cannot be planned</p>
          <p className="font-mono text-xs leading-relaxed text-muted-foreground">
            {state.data.error}
          </p>
        </div>
      ) : null}
      {state.data && !state.data.error && estimatable !== false ? (
        <dl
          className={
            state.status === "pending"
              ? "space-y-2 text-sm opacity-60"
              : "space-y-2 text-sm"
          }
        >
          <Row label="Est. scan">{formatBytes(state.data.estimatedBytes)}</Row>
          <Row label="Est. cost">
            {state.data.estimatedCostMin != null &&
            state.data.estimatedCostMax != null &&
            (state.data.estimatedBytes ?? 0) > 0
              ? `${formatCost(state.data.estimatedCostMin)}–${formatCost(state.data.estimatedCostMax)}`
              : "—"}
          </Row>
          <Row label="Workload">{WORKLOAD_CLASS_LABEL[state.data.workloadClass]}</Row>
          <Row label="Engine">{ENGINE_CATEGORY_LABEL[state.data.engine]}</Row>
          <Row label="Cache">
            {/* Null means nobody asked the cache, which is not the same as
                "not eligible" — saying either would be a guess. */}
            {state.data.cacheEligible == null ? (
              <span className="text-muted-foreground">Unknown</span>
            ) : state.data.cacheEligible ? (
              "Eligible"
            ) : (
              "Not eligible"
            )}
          </Row>
          <Row label="Freshness">
            <FreshnessIndicator lagSeconds={state.data.freshnessLagSeconds} />
          </Row>
          <div>
            <dt className="text-muted-foreground">Policy obligations</dt>
            <dd className="mt-1 flex flex-wrap gap-1">
              {state.data.policyObligations == null ? (
                // No policy engine runs before a query here. "None" would
                // read as "checked, nothing applies".
                <span className="text-xs text-muted-foreground">
                  Not evaluated
                </span>
              ) : state.data.policyObligations.length ? (
                state.data.policyObligations.map((o) => (
                  <Pill key={o} tone="neutral">
                    {o}
                  </Pill>
                ))
              ) : (
                <span className="text-xs text-muted-foreground">None</span>
              )}
            </dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Sources</dt>
            <dd className="mt-1">
              <ul className="space-y-0.5">
                {state.data.sources.map((s) => (
                  <li key={s} className="font-mono text-xs">
                    {s}
                  </li>
                ))}
              </ul>
            </dd>
          </div>
          <QueryPlanPanel plan={state.data.plan} />
        </dl>
      ) : null}
    </SectionCard>
  )
}
