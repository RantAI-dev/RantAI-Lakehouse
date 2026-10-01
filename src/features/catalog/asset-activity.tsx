"use client"

import Link from "next/link"
import { EmptyState, ErrorState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { assetSnapshotQueryHref } from "@/lib/asset-query"
import { formatCompactNumber, formatDateTime, formatNumber, formatRelativeTime } from "@/lib/format"
import {
  isIcebergCandidate,
  msToIso,
  snapshotRelativeTime,
  snapshotsNewestFirst,
} from "@/lib/lakehouse-view"
import { fmtMeasured, type Measured } from "@/lib/measured"
import type { AssetDetail } from "@/services/contracts/assets"
import type { IcebergTableState } from "./asset-storage"

function QuietEmpty({ title, description }: { title: string; description?: string }) {
  return <EmptyState title={title} description={description} className="py-4" />
}

/** `+1,200` for rows a snapshot added; a dash when it added none or did not say. */
function signed(value: Measured, sign: "+" | "−") {
  return value === null || value === 0 ? "—" : `${sign}${formatNumber(value)}`
}

/**
 * The asset's Iceberg snapshots, newest first — one per load — from the
 * table the whole page shares (`useIcebergTable`). Each can be queried as
 * the table stood right after it. A `not_found` means the registry's
 * `tableName` is not a real Bronze table: a quiet empty state, not an
 * error.
 */
function IcebergSnapshots({ asset: a, state }: { asset: AssetDetail; state: IcebergTableState }) {
  if (state.status === "loading") return <QuietEmpty title="Loading snapshots…" />
  if (state.status === "error") {
    if (state.error.code === "not_found") {
      return <QuietEmpty title="No Iceberg table for this asset" />
    }
    return <ErrorState error={state.error} onRetry={state.reload} />
  }

  const snapshots = snapshotsNewestFirst(state.data?.snapshots ?? [])
  if (snapshots.length === 0) return <QuietEmpty title="No snapshots for this asset" />

  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="text-xs">Committed</TableHead>
            <TableHead className="text-xs">Operation</TableHead>
            <TableHead className="text-right text-xs">Added</TableHead>
            <TableHead className="text-right text-xs">Deleted</TableHead>
            <TableHead className="text-right text-xs">Rows after</TableHead>
            <TableHead className="text-right text-xs">Data files</TableHead>
            <TableHead className="text-xs">Snapshot</TableHead>
            <TableHead className="text-xs" aria-label="Actions" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {snapshots.map((s, i) => {
            const href = assetSnapshotQueryHref(a, s.id)
            return (
              <TableRow key={s.id}>
                <TableCell className="py-1.5 text-xs" title={formatDateTime(msToIso(s.timestampMs))}>
                  {snapshotRelativeTime(s.timestampMs)}
                  {i === 0 ? (
                    <Pill tone="neutral" className="ml-1.5">
                      current
                    </Pill>
                  ) : null}
                </TableCell>
                <TableCell className="py-1.5 text-xs">{s.operation}</TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {signed(s.summary.addedRecords, "+")}
                </TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {signed(s.summary.deletedRecords, "−")}
                </TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {fmtMeasured(s.summary.totalRecords, formatNumber)}
                </TableCell>
                <TableCell className="py-1.5 text-right text-xs tabular-nums">
                  {fmtMeasured(s.summary.totalDataFiles, formatNumber)}
                </TableCell>
                <TableCell className="py-1.5 font-mono text-xs text-muted-foreground">{s.id}</TableCell>
                <TableCell className="py-1 text-right">
                  {href ? (
                    <Button size="sm" variant="ghost" render={<Link href={href} />}>
                      Query this version
                    </Button>
                  ) : null}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

/**
 * The Activity tab: what happened to the asset over time — its loads
 * (snapshots), the changes people made to it, and who queries it.
 */
export function AssetActivity({ asset: a, iceberg }: { asset: AssetDetail; iceberg: IcebergTableState }) {
  return (
    <div className="flex flex-col gap-2">
      <SectionCard
        size="sm"
        title="Snapshots"
        description="One per load of an Iceberg table, newest first. Any of them can be queried as the table stood then."
      >
        {isIcebergCandidate(a) && a.tableName ? (
          <IcebergSnapshots asset={a} state={iceberg} />
        ) : a.snapshots.length === 0 ? (
          <QuietEmpty
            title="Only Iceberg tables keep snapshots"
            description="A ClickHouse table is rewritten in place, so there are no earlier versions to go back to."
          />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {a.snapshots.map((s) => (
              <li key={s.id} className="flex justify-between gap-2 py-1.5">
                <span>{s.operation}</span>
                <span className="text-muted-foreground">
                  {formatRelativeTime(s.committedAt)} · {formatCompactNumber(s.records)} records
                </span>
              </li>
            ))}
          </ul>
        )}
      </SectionCard>
      <div className="grid gap-2 lg:grid-cols-2">
        <SectionCard
          size="sm"
          title="Change history"
          description="Edits to this asset's description, owner, tags and policies."
        >
          {a.changeHistory.length === 0 ? (
            <QuietEmpty title="No changes recorded" />
          ) : (
            <ul className="divide-y divide-border text-sm">
              {a.changeHistory.map((c) => (
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
        <SectionCard
          size="sm"
          title="Usage (7d)"
          description="Queries that read this asset, by anyone. Only your own query text is shown."
        >
          {a.usage === null ? (
            <QuietEmpty title="Usage not measured" description="The query history could not be read." />
          ) : a.usage.queries7d === 0 ? (
            <QuietEmpty title="No queries in the last 7 days" />
          ) : (
            <div className="flex flex-col gap-3">
              <dl className="grid grid-cols-3 gap-2 text-sm">
                {(
                  [
                    ["Queries", formatNumber(a.usage.queries7d)],
                    ["People", formatNumber(a.usage.users7d)],
                    ["Avg latency", `${formatNumber(a.usage.avgLatencyMs)} ms`],
                  ] as const
                ).map(([label, value]) => (
                  <div key={label}>
                    <dt className="text-xs text-muted-foreground">{label}</dt>
                    <dd className="text-lg font-semibold tabular-nums">{value}</dd>
                  </div>
                ))}
              </dl>
              <div>
                <div className="mb-1 text-xs font-medium text-muted-foreground">Your recent queries</div>
                {a.recentQueries.length === 0 ? (
                  <p className="text-sm text-muted-foreground">None of them were yours.</p>
                ) : (
                  <ul className="divide-y divide-border text-sm">
                    {a.recentQueries.map((q) => (
                      <li key={q.id} className="flex items-center gap-2 py-1.5">
                        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={q.sql}>
                          {q.sql}
                        </span>
                        {q.status && q.status !== "completed" ? <Pill tone="warning">{q.status}</Pill> : null}
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {formatRelativeTime(q.at)}
                        </span>
                        {q.auditEventId ? (
                          <Link
                            href={`/audit?event=${encodeURIComponent(q.auditEventId)}`}
                            className="shrink-0 text-xs text-primary hover:underline"
                          >
                            Audit
                          </Link>
                        ) : null}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            </div>
          )}
        </SectionCard>
      </div>
    </div>
  )
}
