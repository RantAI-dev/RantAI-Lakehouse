"use client"

import Link from "next/link"
import { PageHeader } from "@/components/patterns/page-header"
import { MetricCard, MetricGrid } from "@/components/patterns/metric-card"
import {
  ErrorState,
  LoadingSkeleton,
  MetricSkeleton,
} from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { SeverityBadge, TierBadge } from "@/components/patterns/status-badge"
import { useService } from "@/hooks/use-service"
import {
  formatBytes,
  formatCompactNumber,
  formatLagSeconds,
  formatPercent,
  formatRelativeTime,
} from "@/lib/format"
import { STORAGE_TIER_LABEL, type StorageTier } from "@/lib/status"
import { overviewService } from "@/services"

const TIERS: StorageTier[] = ["hot", "warm", "cold", "ai"]

/** Overview dashboard — executive-operational KPIs for the lakehouse console. */
export function OverviewPage() {
  const summary = useService((s) => overviewService.getSummary(s), [])
  const activity = useService((s) => overviewService.listActivity(s), [])

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Overview"
        description="Platform health across storage tiers, pipelines, queries, agents, and governance."
      />

      {summary.status === "loading" ? <MetricSkeleton cards={8} /> : null}
      {summary.status === "error" ? (
        <ErrorState error={summary.error} onRetry={summary.reload} />
      ) : null}
      {summary.status === "success" ? (
        <>
          <MetricGrid className="lg:grid-cols-4">
            <MetricCard
              label="Catalog assets"
              value={formatCompactNumber(summary.data.assetsTotal)}
              hint={`${summary.data.staleAssets} stale by watermark`}
              trendTone={summary.data.staleAssets > 0 ? "negative" : "positive"}
              trend={`${summary.data.staleAssets} stale`}
            />
            <MetricCard
              label="Pipelines"
              value={summary.data.pipelines.active}
              hint={`${summary.data.pipelines.failed} failed · ${summary.data.pipelines.delayed} delayed`}
            />
            <MetricCard
              label="Streaming jobs"
              value={summary.data.streaming.jobs}
              hint={`max lag ${formatLagSeconds(summary.data.streaming.maxLagSeconds)} · ${summary.data.streaming.unhealthy} unhealthy`}
              trendTone={summary.data.streaming.unhealthy > 0 ? "negative" : "positive"}
            />
            <MetricCard
              label="Query volume (24h)"
              value={formatCompactNumber(summary.data.queries.volume24h)}
              hint={`p95 ${summary.data.queries.p95Ms} ms · cache ${formatPercent(summary.data.queries.cacheAssistRate)}`}
            />
          </MetricGrid>
          <MetricGrid className="lg:grid-cols-4">
            <MetricCard
              label="Query failure rate"
              value={formatPercent(summary.data.queries.failureRate)}
              hint={`${formatBytes(summary.data.queries.scannedBytes24h)} scanned (24h)`}
              trendTone={summary.data.queries.failureRate > 0 ? "negative" : "positive"}
            />
            <MetricCard
              label="Policy violations (7d)"
              value={summary.data.policyViolations7d}
              trendTone={summary.data.policyViolations7d > 0 ? "negative" : "positive"}
            />
            <MetricCard
              label="Pending approvals"
              value={summary.data.pendingApprovals}
            />
            <MetricCard
              label="Agent runs"
              value={summary.data.agents.activeRuns}
              hint={`${formatPercent(summary.data.agents.budgetUsedRate)} budget used`}
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
              description="Access layer, stores, streaming, and retrieval planes."
            >
              <div className="flex flex-wrap gap-3 text-sm">
                <span className="text-emerald-600 dark:text-emerald-400">
                  {summary.data.services.healthy} healthy
                </span>
                <span className="text-amber-600 dark:text-amber-400">
                  {summary.data.services.degraded} degraded
                </span>
                <span className="text-destructive">
                  {summary.data.services.unhealthy} unhealthy
                </span>
                <Link href="/services" className="ml-auto text-primary hover:underline">
                  View services
                </Link>
              </div>
            </SectionCard>
            <SectionCard
              title="Recent incidents"
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
                        className="-mx-2 flex items-start gap-2 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-muted/40"
                      >
                        <SeverityBadge severity={inc.severity} />
                        <span className="min-w-0">
                          <span className="block font-medium">{inc.title}</span>
                          <span className="block text-xs text-muted-foreground">
                            {inc.source} · {formatRelativeTime(inc.at)}
                          </span>
                        </span>
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
                  <span className="text-xs text-muted-foreground tabular-nums">
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
