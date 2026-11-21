"use client"

import * as React from "react"
import Link from "next/link"
import { Copy, Play } from "lucide-react"
import { CodeBlock } from "@/components/patterns/code-block"
import { Button } from "@/components/ui/button"
import { formatRelativeTime } from "@/lib/format"
import {
  assetHasReadableTable,
  assetQueryStudioHref,
  assetStarterSql,
  assetStudioTarget,
  NO_READABLE_TABLE,
} from "@/lib/asset-query"
import { copyTextWithToast } from "@/lib/copy-text"
import { snapshotRelativeTime } from "@/lib/lakehouse-view"
import type { AssetColumn, AssetDetail } from "@/services/contracts/assets"
import { AssetDetails } from "./asset-about"
import { schemaRows, useProfile } from "./asset-column-data"
import { ColumnsPreview } from "./asset-columns"
import type { AssetLineageState } from "./asset-lineage"
import { LineageShortList } from "./asset-lineage-strip"
import { icebergTableOf, type IcebergTableState } from "./asset-storage"
import { TableGrantNote } from "./table-grant-note"

/** Tabs the overview can jump to; must match the `value`s in `AssetDetailTabs`. */
export type AssetTab =
  | "overview"
  | "schema"
  | "sample"
  | "quality"
  | "access"
  | "lineage"
  | "activity"

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

/** How many columns the Overview shows before pointing at the Columns tab. */
const PREVIEW_COLUMNS = 6

/**
 * The sensitive columns in one line (`UX-8`): each name with why it is
 * sensitive. The rows above carry the same marks inline, so this is the
 * summary, not a second list.
 */
export function sensitiveLine(columns: AssetColumn[]): string {
  const sensitive = columns.filter(isSensitive)
  if (sensitive.length === 0) return "No sensitive columns · nothing masked for you"
  const why = (c: AssetColumn) =>
    [c.masked ? "masked" : null, c.classification === "confidential" || c.classification === "restricted" ? c.classification : null]
      .filter(Boolean)
      .join(", ")
  return `${sensitive.length} sensitive column${sensitive.length === 1 ? "" : "s"}: ${sensitive
    .map((c) => `${c.name} (${why(c)})`)
    .join(" · ")}`
}

/** A section of the Overview: its title, an optional action on the right, and its body. */
function Section({
  title,
  action,
  children,
}: {
  title: string
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section aria-label={title} className="flex min-w-0 flex-col gap-2">
      <div className="flex min-h-7 items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  )
}

/**
 * The first tab of an asset, read like a readme (`UX-8`, mockup A): its
 * real columns with what they hold, the query to start from, then on the
 * side the details and where it comes from and goes.
 *
 * What the old Overview's tiles and cards said is said once, elsewhere:
 * health in the header, freshness, quality and access in the trust strip
 * above the tabs, storage on the Activity tab, publishing in the ⋯ menu,
 * the connections on the Lineage tab and the changes on the Activity tab.
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
  /** Reloads the asset after its sensitivity was set. */
  onAssetChanged: () => void
}) {
  const profile = useProfile(a.id)
  // The Iceberg table, once loaded. It is what proves the table exists, so
  // the links below open Trino only when it is there (owner, 2026-11-08).
  const table = icebergTableOf(iceberg)
  // Whether this asset has a table the console can read at all. Without one
  // the starter query and the Query Studio link would name a table that is
  // not there, so the section says so instead (owner, 2026-11-09). While the
  // Iceberg read is still running the asset is given the benefit of the
  // doubt: the sentence waits for the answer.
  const readable = assetHasReadableTable(a, table, iceberg.status === "loading")
  // QA `LH-DATA-15-07` F2 (`DATA-54`): the count is the registry's, and a
  // table that is gone has no rows to count. Any other failure of the read
  // keeps the count: it does not prove the table is gone.
  const rowsGone = iceberg.status === "error" && iceberg.error.code === "not_found"
  const { rows } = schemaRows(a.schema, table, a.type !== "iceberg-table")
  // The Iceberg table's current schema version, once loaded; the API's
  // own list otherwise — for a Silver or Gold table, the version the console
  // recorded. That one is dated when the console saw it, so it says
  // "recorded", not a time that would read as the table's own age.
  const icebergSchema = table?.schemaVersions?.find((v) => v.current)
  const latestSchema = a.schemaVersions.find((v) => v.current) ?? a.schemaVersions[0]
  const schemaLabel = icebergSchema
    ? `v${icebergSchema.schemaId}` +
      (icebergSchema.sinceMs === null ? "" : ` · since ${snapshotRelativeTime(icebergSchema.sinceMs)}`)
    : latestSchema
      ? `v${latestSchema.version} · recorded ${formatRelativeTime(latestSchema.at)}`
      : null
  const sql = assetStarterSql(a, table)
  const engine = assetStudioTarget(a, table).engine === "trino" ? "Trino" : "ClickHouse"

  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_300px]">
      <div className="flex min-w-0 flex-col gap-6">
        <Section
          title="Columns"
          action={
            rows.length > PREVIEW_COLUMNS ? (
              <Button size="sm" variant="ghost" onClick={() => onNavigate("schema")}>
                {`All ${rows.length}`}
              </Button>
            ) : undefined
          }
        >
          <ColumnsPreview rows={rows} profile={profile} sortingKey={a.storage?.sortingKey} count={PREVIEW_COLUMNS} />
          <p className="text-xs text-muted-foreground">{sensitiveLine(a.schema)}</p>
        </Section>

        <Section
          title="Start a query"
          action={
            readable ? (
              <div className="flex gap-1.5">
                <Button size="sm" variant="ghost" onClick={() => void copyTextWithToast(sql, "Copied SQL")}>
                  <Copy />
                  Copy
                </Button>
                <Button size="sm" variant="outline" render={<Link href={assetQueryStudioHref(a, table)} />}>
                  <Play />
                  Open in Query Studio
                </Button>
              </div>
            ) : undefined
          }
        >
          {readable ? (
            <>
              <CodeBlock>{sql}</CodeBlock>
              <p className="text-xs text-muted-foreground">{`A starter query for ${engine}. Masking still applies when it runs.`}</p>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">{NO_READABLE_TABLE}</p>
          )}
        </Section>
      </div>

      <aside className="flex min-w-0 flex-col gap-6">
        <Section title="Details">
          <AssetDetails asset={a} schemaLabel={schemaLabel} rowsGone={rowsGone} onChanged={onAssetChanged} />
        </Section>
        <Section title="Lineage">
          <LineageShortList asset={a} state={lineage} onOpen={() => onNavigate("lineage")} />
          {/* GOV-11 (review round 1 R2): the API withheld what reads it and
              what it relates to, so a short list is not "none". */}
          <TableGrantNote asset={a} />
        </Section>
      </aside>
    </div>
  )
}
