"use client"

import Link from "next/link"
import { Copy } from "lucide-react"
import { CodeBlock } from "@/components/patterns/code-block"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { HealthTile } from "@/components/patterns/health-tile"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import {
  CheckBadge,
  ClassificationBadge,
  HealthBadge,
  Pill,
} from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { formatRelativeTime } from "@/lib/format"
import { assetQueryStudioHref, assetQueryTarget, assetStarterSql } from "@/lib/asset-query"
import { isIcebergCandidate, snapshotRelativeTime } from "@/lib/lakehouse-view"
import { notifyError, notifySuccess } from "@/lib/notify"
import { cn } from "@/lib/utils"
import type { AssetColumn, AssetDetail } from "@/services/contracts/assets"
import { AssetAbout } from "./asset-about"
import { NO_HEALTH_SIGNAL } from "./asset-badges"
import {
  assetDependents,
  lineageSides,
  relatedTables,
  type AssetLineageState,
} from "./asset-lineage"
import {
  AssetStorage,
  ClickHouseStorage,
  icebergTableOf,
  type IcebergTableState,
} from "./asset-storage"

/** Tabs the overview can jump to; must match the `value`s in `AssetDetailTabs`. */
export type AssetTab =
  | "overview"
  | "schema"
  | "sample"
  | "quality"
  | "access"
  | "lineage"
  | "activity"

/** How many recent changes the overview shows before pointing at History. */
const RECENT_CHANGES = 3

/**
 * A column a reader should know is sensitive before they query it: masked
 * by policy, or classified above "internal".
 */
function isSensitive(c: AssetColumn) {
  return (
    c.masked === true ||
    c.classification === "confidential" ||
    c.classification === "restricted"
  )
}

/**
 * Verdicts by result. A rule nothing has evaluated has no result, so it
 * counts toward `evaluated` neither as a pass nor as a failure.
 */
export function qualitySummary(checks: AssetDetail["qualityChecks"]) {
  const passed = checks.filter((q) => q.status === "passed").length
  const warning = checks.filter((q) => q.status === "warning").length
  const failed = checks.filter((q) => q.status === "failed").length
  const evaluated = passed + warning + failed
  // ISO timestamps sort lexically, so the max string is the latest run.
  const lastRun = checks.reduce<string | null>(
    (latest, q) => (q.lastRun !== null && (latest === null || q.lastRun > latest) ? q.lastRun : latest),
    null
  )
  return { passed, warning, failed, evaluated, unevaluated: checks.length - evaluated, lastRun }
}

async function copyText(value: string, what: string) {
  try {
    await navigator.clipboard.writeText(value)
    notifySuccess(`Copied ${what}`)
  } catch (err) {
    notifyError("Failed to copy", err)
  }
}

/**
 * The first tab of an asset: can I trust this data, who is it for, and how
 * do I start using it — answered from `AssetDetail` alone, with each
 * section pointing at the tab that holds the full story.
 */
export function AssetOverview({
  asset: a,
  iceberg,
  lineage,
  onNavigate,
  onAssetChanged,
}: {
  asset: AssetDetail
  iceberg: IcebergTableState
  lineage: AssetLineageState
  onNavigate: (tab: AssetTab) => void
  /** Reloads the asset after its details were edited. */
  onAssetChanged: () => void
}) {
  // The same recorded graph the Lineage tab draws, so the two never disagree.
  const graph = lineage.status === "success" ? lineage.data : null
  const sides = lineageSides(graph)
  const connections = [
    ["Upstream", sides.upstream.length],
    ["Downstream", sides.downstream.length],
    ["Dependents", assetDependents(a, graph).length],
    ["Related tables", relatedTables(a, graph).length],
  ] as const
  // "Related tables" is the rare case; without one it is a row of noise.
  const shownConnections = connections.filter(([label, n]) => label !== "Related tables" || n > 0)
  // An Iceberg table's own snapshots, once loaded; the API's list otherwise.
  const snapshotCount = icebergTableOf(iceberg)?.snapshots.length ?? a.snapshots.length
  const quality = qualitySummary(a.qualityChecks)
  const sensitive = a.schema.filter(isSensitive)
  const maskedCount = a.schema.filter((c) => c.masked).length
  const healthReasons = a.healthReasons ?? []
  // The Iceberg table's current schema version, once loaded; the API's
  // own list otherwise.
  const icebergSchema = icebergTableOf(iceberg)?.schemaVersions?.find((v) => v.current)
  const latestSchema = a.schemaVersions[0]
  const schemaLabel = icebergSchema
    ? `v${icebergSchema.schemaId}` +
      (icebergSchema.sinceMs === null ? "" : ` · since ${snapshotRelativeTime(icebergSchema.sinceMs)}`)
    : latestSchema
      ? `v${latestSchema.version} · ${formatRelativeTime(latestSchema.at)}`
      : null
  const recentChanges = a.changeHistory.slice(0, RECENT_CHANGES)
  const sql = assetStarterSql(a)

  return (
    <div className="flex flex-col gap-2">
      {/* ── Can I trust it? ─────────────────────────────────────────── */}
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        <HealthTile
          label="Health"
          hint={
            // What the status rests on, signal by signal — or that there
            // is nothing to judge it by.
            healthReasons.length === 0 ? (
              NO_HEALTH_SIGNAL
            ) : (
              <span className="flex flex-col gap-0.5">
                {healthReasons.map((reason) => (
                  <span key={reason}>{reason}</span>
                ))}
              </span>
            )
          }
        >
          <HealthBadge health={a.health} />
        </HealthTile>
        <HealthTile
          label="Freshness"
          hint={
            snapshotCount > 0
              ? `${snapshotCount} snapshot${snapshotCount === 1 ? "" : "s"}`
              : undefined
          }
          onClick={() => onNavigate("activity")}
        >
          <FreshnessIndicator
            lagSeconds={a.freshnessLagSeconds}
            targetSeconds={a.freshnessTargetSeconds ?? null}
            targetSource={a.freshnessTargetSource}
          />
        </HealthTile>
        <HealthTile
          label="Quality"
          hint={
            a.qualityChecks.length === 0
              ? "No checks configured"
              : quality.lastRun === null
                ? "Not evaluated yet"
                : `Last run ${formatRelativeTime(quality.lastRun)}`
          }
          onClick={() => onNavigate("quality")}
        >
          {a.qualityChecks.length === 0 ? (
            <span className="text-muted-foreground">—</span>
          ) : quality.evaluated === 0 ? (
            <span className="tabular-nums">
              {a.qualityChecks.length} rule{a.qualityChecks.length === 1 ? "" : "s"}
            </span>
          ) : (
            <>
              <span className="tabular-nums">
                {quality.passed}/{quality.evaluated} passed
              </span>
              {quality.failed > 0 ? <CheckBadge status="failed" /> : null}
              {quality.failed === 0 && quality.warning > 0 ? (
                <CheckBadge status="warning" />
              ) : null}
            </>
          )}
        </HealthTile>
        <HealthTile
          label="Governance"
          hint={`${maskedCount} masked column${maskedCount === 1 ? "" : "s"} · ${
            a.classificationSource === "rule" ? "classified by rule" : "default level"
          }`}
          onClick={() => onNavigate("access")}
        >
          <ClassificationBadge classification={a.classification} />
          <span className="tabular-nums text-muted-foreground">
            {a.policySummary.length} polic{a.policySummary.length === 1 ? "y" : "ies"}
          </span>
        </HealthTile>
      </div>

      <div className="grid gap-2 lg:grid-cols-2">
        {/* ── What is it? ───────────────────────────────────────────── */}
        <AssetAbout asset={a} schemaLabel={schemaLabel} onChanged={onAssetChanged} />

        {/* ── What should I be careful with? ────────────────────────── */}
        <SectionCard
          size="sm"
          title="Sensitive columns"
          description="Masked by policy or classified confidential and above."
        >
          {sensitive.length === 0 ? (
            <EmptyState title="No sensitive columns" className="py-4" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {sensitive.map((c) => (
                <li key={c.name} className="flex flex-wrap items-center gap-2 py-1.5">
                  <span className="font-mono font-medium">{c.name}</span>
                  <span className="text-muted-foreground">{c.dataType}</span>
                  <span className="ml-auto flex items-center gap-1.5">
                    {c.classification ? (
                      <ClassificationBadge classification={c.classification} />
                    ) : null}
                    {c.masked ? <Pill tone="warning">masked</Pill> : null}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </div>

      {/* ── Where and how is it stored? ─────────────────────────────── */}
      {isIcebergCandidate(a) && a.tableName ? (
        <AssetStorage tableName={a.tableName} detail={iceberg} />
      ) : (
        <ClickHouseStorage asset={a} />
      )}

      {/* ── How do I use it? ────────────────────────────────────────── */}
      <SectionCard
        size="sm"
        title="Use this data"
        description={`A starter query for ${
          assetQueryTarget(a).engine === "trino" ? "Trino" : "ClickHouse"
        }. Masking still applies when it runs.`}
        action={
          <div className="flex gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => void copyText(sql, "SQL")}>
              <Copy />
              Copy
            </Button>
            <Button
              size="sm"
              variant="outline"
              render={<Link href={assetQueryStudioHref(a)} />}
            >
              Open in Query Studio
            </Button>
          </div>
        }
      >
        <CodeBlock>{sql}</CodeBlock>
      </SectionCard>

      <div className="grid gap-2 lg:grid-cols-2">
        {/* ── Where does it come from and go? ───────────────────────── */}
        <SectionCard size="sm" title="Connections">
          <ul className="divide-y divide-border text-sm">
            {shownConnections.map(([label, count]) => (
              <li key={label}>
                <button
                  type="button"
                  onClick={() => onNavigate("lineage")}
                  className="flex w-full items-center gap-2 py-1.5 text-left hover:text-primary"
                >
                  {label}
                  <span
                    className={cn(
                      "ml-auto tabular-nums",
                      count === 0 ? "text-muted-foreground/50" : "text-muted-foreground"
                    )}
                  >
                    {count}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </SectionCard>

        {/* ── What changed lately? ──────────────────────────────────── */}
        <SectionCard
          size="sm"
          title="Recent changes"
          action={
            a.changeHistory.length > RECENT_CHANGES ? (
              <Button size="sm" variant="ghost" onClick={() => onNavigate("activity")}>
                View all
              </Button>
            ) : undefined
          }
        >
          {recentChanges.length === 0 ? (
            <EmptyState title="No changes recorded" className="py-4" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {recentChanges.map((c) => (
                <li key={c.id} className="flex flex-wrap items-baseline gap-2 py-1.5">
                  <span className="font-medium">{c.actor}</span>
                  <span className="text-muted-foreground">{c.summary}</span>
                  <span className="ml-auto text-xs text-muted-foreground">
                    {formatRelativeTime(c.at)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </SectionCard>
      </div>
    </div>
  )
}
