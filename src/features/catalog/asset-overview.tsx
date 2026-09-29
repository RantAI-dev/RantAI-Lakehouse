"use client"

import Link from "next/link"
import { Copy } from "lucide-react"
import { CodeBlock } from "@/components/patterns/code-block"
import { FreshnessIndicator } from "@/components/patterns/freshness-indicator"
import { MetadataList } from "@/components/patterns/metadata-list"
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
import { isIcebergCandidate } from "@/lib/lakehouse-view"
import { notifyError, notifySuccess } from "@/lib/notify"
import { cn } from "@/lib/utils"
import type { AssetColumn, AssetDetail } from "@/services/contracts/assets"
import { AssetStorage } from "./asset-storage"

/** Tabs the overview can jump to; must match the `value`s in `AssetDetailTabs`. */
export type AssetTab =
  | "overview"
  | "schema"
  | "sample"
  | "quality"
  | "policies"
  | "lineage"
  | "dependents"
  | "history"
  | "snapshots"
  | "usage"

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

function qualitySummary(checks: AssetDetail["qualityChecks"]) {
  const passed = checks.filter((q) => q.status === "passed").length
  const warning = checks.filter((q) => q.status === "warning").length
  const failed = checks.filter((q) => q.status === "failed").length
  // ISO timestamps sort lexically, so the max string is the latest run.
  const lastRun = checks.reduce<string | null>(
    (latest, q) => (latest === null || q.lastRun > latest ? q.lastRun : latest),
    null
  )
  return { passed, warning, failed, lastRun }
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
 * One tile of the health strip. A button, not a link: every tile opens a
 * tab on this same page, where the detail behind the number lives.
 */
function HealthTile({
  label,
  children,
  hint,
  onClick,
}: {
  label: string
  children: React.ReactNode
  hint?: React.ReactNode
  onClick?: () => void
}) {
  const className = cn(
    "flex flex-col items-start gap-1.5 rounded-lg border border-border bg-card p-3 text-left",
    onClick && "transition-colors hover:border-primary/40 hover:bg-muted/30"
  )
  const body = (
    <>
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <span className="flex min-h-6 flex-wrap items-center gap-1.5 text-sm font-medium">
        {children}
      </span>
      {hint ? <span className="text-xs text-muted-foreground">{hint}</span> : null}
    </>
  )
  return onClick ? (
    <button type="button" className={className} onClick={onClick}>
      {body}
    </button>
  ) : (
    <div className={className}>{body}</div>
  )
}

/**
 * The first tab of an asset: can I trust this data, who is it for, and how
 * do I start using it — answered from `AssetDetail` alone, with each
 * section pointing at the tab that holds the full story.
 */
export function AssetOverview({
  asset: a,
  onNavigate,
}: {
  asset: AssetDetail
  onNavigate: (tab: AssetTab) => void
}) {
  const quality = qualitySummary(a.qualityChecks)
  const sensitive = a.schema.filter(isSensitive)
  const maskedCount = a.schema.filter((c) => c.masked).length
  const latestSchema = a.schemaVersions[0]
  const recentChanges = a.changeHistory.slice(0, RECENT_CHANGES)
  const sql = assetStarterSql(a)

  return (
    <div className="flex flex-col gap-2">
      {/* ── Can I trust it? ─────────────────────────────────────────── */}
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        <HealthTile
          label="Health"
          hint={a.lastUpdated === null ? "Never updated" : `Updated ${formatRelativeTime(a.lastUpdated)}`}
        >
          <HealthBadge health={a.health} />
        </HealthTile>
        <HealthTile
          label="Freshness"
          hint={
            a.snapshots.length > 0
              ? `${a.snapshots.length} snapshot${a.snapshots.length === 1 ? "" : "s"}`
              : undefined
          }
          onClick={() => onNavigate("snapshots")}
        >
          <FreshnessIndicator lagSeconds={a.freshnessLagSeconds} />
        </HealthTile>
        <HealthTile
          label="Quality"
          hint={
            quality.lastRun === null
              ? "No checks configured"
              : `Last run ${formatRelativeTime(quality.lastRun)}`
          }
          onClick={() => onNavigate("quality")}
        >
          {a.qualityChecks.length === 0 ? (
            <span className="text-muted-foreground">—</span>
          ) : (
            <>
              <span className="tabular-nums">
                {quality.passed}/{a.qualityChecks.length} passed
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
          hint={`${maskedCount} masked column${maskedCount === 1 ? "" : "s"}`}
          onClick={() => onNavigate("policies")}
        >
          <ClassificationBadge classification={a.classification} />
          <span className="tabular-nums text-muted-foreground">
            {a.policySummary.length} polic{a.policySummary.length === 1 ? "y" : "ies"}
          </span>
        </HealthTile>
      </div>

      <div className="grid gap-2 lg:grid-cols-2">
        {/* ── What is it? ───────────────────────────────────────────── */}
        <SectionCard size="sm" title="About">
          <p className={cn("mb-3 text-sm", !a.description && "text-muted-foreground")}>
            {a.description || "No description yet. Ask the owner to document what one row represents."}
          </p>
          <MetadataList
            density="compact"
            columns={2}
            items={[
              { label: "Domain", value: a.domain || "—" },
              { label: "Owner", value: a.owner || "—" },
              { label: "Update frequency", value: a._meta?.frekuensi || "—" },
              ...(a._meta?.satuan ? [{ label: "Unit", value: a._meta.satuan }] : []),
              ...(a._meta?.klasifikasi
                ? [{ label: "Publisher classification", value: a._meta.klasifikasi }]
                : []),
              { label: "Columns", value: String(a.schema.length || a.columnCount) },
              {
                label: "Schema",
                value: latestSchema
                  ? `v${latestSchema.version} · ${formatRelativeTime(latestSchema.at)}`
                  : "—",
              },
            ]}
          />
        </SectionCard>

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
      {isIcebergCandidate(a) && a.tableName ? <AssetStorage tableName={a.tableName} /> : null}

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
            {(
              [
                ["Upstream assets", a.upstream.length, "lineage"],
                ["Downstream assets", a.downstream.length, "lineage"],
                ["Dependents", a.dependents.length, "dependents"],
              ] as const
            ).map(([label, count, tab]) => (
              <li key={label}>
                <button
                  type="button"
                  onClick={() => onNavigate(tab)}
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
              <Button size="sm" variant="ghost" onClick={() => onNavigate("history")}>
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
