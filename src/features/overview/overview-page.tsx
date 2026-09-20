"use client"

import Link from "next/link"
import { RefreshCw } from "lucide-react"
import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import {
  ErrorState,
  LoadingSkeleton,
  MetricSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { SeverityBadge, TierBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService } from "@/hooks/use-service"
import {
  formatBytes,
  formatCompactNumber,
  formatDateTime,
  formatPercent,
  formatRelativeTime,
} from "@/lib/format"
import { STORAGE_TIER_LABEL, type StorageTier } from "@/lib/status"
import type { ServiceStatus } from "@/services/contracts/overview"
import { overviewService } from "@/services"
import { cn } from "@/lib/utils"

const TIERS: StorageTier[] = ["hot", "warm", "cold", "ai"]

const STATUS_DOT: Record<ServiceStatus, string> = {
  healthy: "bg-emerald-500",
  degraded: "bg-amber-500",
  unhealthy: "bg-destructive",
  unavailable: "bg-muted-foreground/40",
}

const STATUS_LABEL: Record<ServiceStatus, string> = {
  healthy: "Healthy",
  degraded: "Degraded",
  unhealthy: "Unhealthy",
  unavailable: "Not connected",
}

/** Overview dashboard — executive-operational KPIs for the lakehouse console. */
export function OverviewPage() {
  const summary = useService((s) => overviewService.getSummary(s), [])
  const activity = useService((s) => overviewService.listActivity(s), [])

  const reloadAll = () => {
    summary.reload()
    activity.reload()
  }
  const generatedAt = summary.status === "success" ? summary.data.generatedAt : undefined
  const busy = summary.status === "loading" || activity.status === "loading"

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Overview"
        description="Platform health across storage tiers, pipelines, queries, agents, and governance."
        actions={
          <div className="flex items-center gap-2">
            {generatedAt ? (
              <span className="hidden text-xs text-muted-foreground sm:inline" title={formatDateTime(generatedAt)}>
                Updated {formatRelativeTime(generatedAt)}
              </span>
            ) : null}
            <Button variant="outline" size="sm" onClick={reloadAll} disabled={busy}>
              <RefreshCw className={cn("size-4", busy && "animate-spin")} />
              Refresh
            </Button>
          </div>
        }
      />

      {summary.status === "loading" ? <MetricSkeleton cards={7} /> : null}
      {summary.status === "error" ? (
        <ErrorState error={summary.error} onRetry={summary.reload} />
      ) : null}
      {summary.status === "success" ? (
        <>
          {/* What the platform holds and how hard it is working. */}
          <MetricGrid className="lg:grid-cols-4">
            <MetricCard
              label="Catalog assets"
              value={formatCompactNumber(summary.data.assetsTotal)}
              hint={`${summary.data.staleAssets} stale by watermark`}
              trendTone={summary.data.staleAssets > 0 ? "negative" : "positive"}
              href="/catalog"
            />
            <MetricCard
              label="Pipelines running"
              value={summary.data.pipelines.active}
              hint={`${summary.data.pipelines.failed} failed · ${summary.data.pipelines.delayed} behind schedule`}
              trendTone={summary.data.pipelines.failed > 0 ? "negative" : "neutral"}
              href="/pipelines"
            />
            <MetricCard
              label="Query volume (24h)"
              value={formatCompactNumber(summary.data.queries.volume24h)}
              hint={`p95 ${summary.data.queries.p95Ms} ms · ${formatBytes(summary.data.queries.scannedBytes24h)} scanned`}
              href="/workloads"
            />
            <MetricCard
              label="Query failure rate (24h)"
              value={formatPercent(summary.data.queries.failureRate)}
              trendTone={summary.data.queries.failureRate > 0 ? "negative" : "positive"}
              href="/workloads"
            />
          </MetricGrid>

          {/* What is waiting for a human. */}
          <MetricGrid className="lg:grid-cols-3">
            <MetricCard
              label="Policy violations (7d)"
              value={summary.data.policyViolations7d}
              hint="Actions the policy gate refused"
              trendTone={summary.data.policyViolations7d > 0 ? "negative" : "positive"}
              href="/audit"
            />
            <MetricCard
              label="Pending approvals"
              value={summary.data.pendingApprovals}
              hint="Agent actions waiting for a decision"
              href="/agents/approvals"
            />
            <MetricCard
              label="Agent runs in flight"
              value={summary.data.agents.activeRuns}
              href="/agents/runs"
            />
          </MetricGrid>

          <SectionCard
            title="Storage distribution"
            description="Primary physical tiers. Logical layers remain available as secondary filters in Data Explorer."
          >
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {TIERS.map((tier) => {
                const t = summary.data.assetsByTier[tier]
                return (
                  <Link
                    key={tier}
                    href={`/data?tier=${tier}`}
                    className="rounded-lg border border-border bg-muted/20 p-3 transition-colors hover:bg-muted/40"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <TierBadge tier={tier} />
                      <span className="text-xs text-muted-foreground">
                        {t.count} assets
                      </span>
                    </div>
                    <p className="mt-2 text-lg font-semibold tabular-nums">
                      {formatBytes(t.bytes)}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {STORAGE_TIER_LABEL[tier]} tier
                    </p>
                  </Link>
                )
              })}
            </div>
          </SectionCard>

          <div className="grid gap-4 lg:grid-cols-2">
            <SectionCard
              title="Service health"
              description="What this page could reach just now."
              action={
                <Link href="/services" className="text-sm text-primary hover:underline">
                  View services
                </Link>
              }
            >
              {summary.data.services.items?.length ? (
                <ul className="space-y-1.5">
                  {summary.data.services.items.map((svc) => (
                    <li key={svc.name} className="flex items-center gap-2 text-sm">
                      <span className={cn("size-2 shrink-0 rounded-full", STATUS_DOT[svc.status])} aria-hidden />
                      <span className="font-medium">{svc.name}</span>
                      <span className="ml-auto text-xs text-muted-foreground">{STATUS_LABEL[svc.status]}</span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">
                  {summary.data.services.healthy} healthy · {summary.data.services.degraded} degraded ·{" "}
                  {summary.data.services.unhealthy} unhealthy
                </p>
              )}
            </SectionCard>

            <SectionCard
              title="Open incidents"
              action={
                <Link href="/alerts" className="text-sm text-primary hover:underline">
                  View alerts
                </Link>
              }
            >
              {summary.data.incidents.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No open incidents right now.
                </p>
              ) : (
                <ul className="space-y-1">
                  {summary.data.incidents.map((inc) => (
                    <li key={inc.id}>
                      <Link
                        href="/alerts"
                        className="-mx-2 flex items-center gap-3 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-muted/40"
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate font-medium">{inc.title}</span>
                          <span className="block truncate text-xs text-muted-foreground">
                            {inc.source} · {formatRelativeTime(inc.at)}
                          </span>
                        </span>
                        <SeverityBadge severity={inc.severity} />
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </SectionCard>
          </div>
        </>
      ) : null}

      <SectionCard
        title="Recent activity"
        description="Every console and Copilot action, from the audit trail."
        action={
          <Link href="/activity" className="text-sm text-primary hover:underline">
            View all
          </Link>
        }
      >
        {activity.status === "loading" ? <LoadingSkeleton rows={4} /> : null}
        {activity.status === "error" ? (
          <ErrorState error={activity.error} onRetry={activity.reload} />
        ) : null}
        {activity.status === "success" ? (
          activity.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">No recent activity.</p>
          ) : (
            <ul className="divide-y divide-border">
              {activity.data.slice(0, 6).map((item) => (
                <li
                  key={item.id}
                  className="flex flex-wrap items-baseline gap-x-2 gap-y-1 py-2.5 text-sm"
                >
                  <span
                    className="text-xs tabular-nums text-muted-foreground"
                    title={formatDateTime(item.at)}
                  >
                    {formatRelativeTime(item.at)}
                  </span>
                  <span className="font-medium">{item.actor}</span>
                  <span className="text-muted-foreground">{item.action}</span>
                  {item.targetHref ? (
                    <Link href={item.targetHref} className="text-primary hover:underline">
                      {item.target}
                    </Link>
                  ) : (
                    <span>{item.target}</span>
                  )}
                </li>
              ))}
            </ul>
          )
        ) : null}
      </SectionCard>
    </div>
  )
}
