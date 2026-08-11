"use client"

import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import { ErrorState, MetricSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { CheckBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import { formatLagSeconds, formatPercent } from "@/lib/format"
import { opsService } from "@/services"

export function ObservabilityPage() {
  const state = useService((s) => opsService.getObservability(s), [])
  return (
    <div className="flex flex-col gap-4">
      <PageHeader title="Observability" description="Platform SLOs, lag, cache behavior, policy latency, and agent performance." />
      {state.status === "loading" ? <MetricSkeleton cards={8} /> : null}
      {state.status === "error" ? <ErrorState error={state.error} onRetry={state.reload} /> : null}
      {state.status === "success" ? (
        <>
          <MetricGrid>
            <MetricCard label="Query p95" value={`${state.data.queryP95Ms} ms`} />
            <MetricCard label="Query errors" value={formatPercent(state.data.queryErrorRate)} />
            <MetricCard label="Ingest lag" value={formatLagSeconds(state.data.ingestLagSeconds)} />
            <MetricCard label="Streaming lag" value={formatLagSeconds(state.data.streamingLagSeconds)} trendTone="negative" />
            <MetricCard label="Cache hit" value={formatPercent(state.data.cacheHitRate)} />
            <MetricCard label="Policy p95" value={`${state.data.policyDecisionP95Ms} ms`} />
            <MetricCard label="Agent success" value={formatPercent(state.data.agentSuccessRate)} />
            <MetricCard label="Incidents" value={state.data.activeIncidents} />
          </MetricGrid>
          <SectionCard title="SLO board">
            <ul className="space-y-2 text-sm">
              {state.data.slos.map((s) => (
                <li key={s.name} className="flex flex-wrap items-center justify-between gap-2 border-b border-border py-2">
                  <span className="flex items-center gap-2">
                    <CheckBadge status={s.ok ? "passed" : "failed"} />
                    {s.name}
                  </span>
                  <span className="text-muted-foreground">
                    <span className="font-mono text-foreground">{s.current}</span>{" "}
                    (target {s.target})
                  </span>
                </li>
              ))}
            </ul>
          </SectionCard>
        </>
      ) : null}
    </div>
  )
}
