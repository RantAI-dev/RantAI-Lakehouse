"use client"

import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { Pill } from "@/components/patterns/status-badge"
import { SectionCard } from "@/components/patterns/section-card"
import { Skeleton } from "@/components/ui/skeleton"
import type { ActionState } from "@/hooks/use-service"
import { formatBytes, formatCost } from "@/lib/format"
import { ENGINE_CATEGORY_LABEL, WORKLOAD_CLASS_LABEL } from "@/lib/status"
import type { QueryEstimate } from "@/services/contracts/queries"

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-2">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="text-right">{children}</dd>
    </div>
  )
}

/** Pre-run estimate panel: scan, cost range, routing, freshness, policy. */
export function QueryTransparencyPanel({
  state,
  onRetry,
}: {
  state: ActionState<QueryEstimate>
  onRetry: () => void
}) {
  return (
    <SectionCard
      title="Execution transparency"
      description="Pre-run estimate. Engine categories stay product-neutral."
    >
      {state.status === "idle" ? (
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
      {state.data ? (
        <dl
          className={
            state.status === "pending"
              ? "space-y-2 text-sm opacity-60"
              : "space-y-2 text-sm"
          }
        >
          <Row label="Est. scan">{formatBytes(state.data.estimatedBytes)}</Row>
          <Row label="Est. cost">
            {formatCost(state.data.estimatedCostMin)}–{formatCost(state.data.estimatedCostMax)}
          </Row>
          <Row label="Workload">{WORKLOAD_CLASS_LABEL[state.data.workloadClass]}</Row>
          <Row label="Engine">{ENGINE_CATEGORY_LABEL[state.data.engine]}</Row>
          <Row label="Cache">
            {state.data.cacheEligible ? "Eligible" : "Not eligible"}
          </Row>
          <Row label="Freshness">
            <FreshnessIndicator lagSeconds={state.data.freshnessLagSeconds} />
          </Row>
          <div>
            <dt className="text-muted-foreground">Policy obligations</dt>
            <dd className="mt-1 flex flex-wrap gap-1">
              {state.data.policyObligations.length ? (
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
        </dl>
      ) : null}
    </SectionCard>
  )
}
