"use client"

import * as React from "react"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import {
  formatCompactNumber,
  formatDate,
  formatDateTime,
  formatNumber,
  formatPercent,
  formatRelativeTime,
} from "@/lib/format"
import { snapshotRelativeTime } from "@/lib/lakehouse-view"
import { schemaHistory } from "@/lib/schema-history"
import { assetService } from "@/services"
import type { AssetColumn, AssetDetail, AssetProfile, ColumnProfile } from "@/services/contracts/assets"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
import { icebergTableOf, type IcebergTableState } from "./asset-storage"
import { StandInNotice } from "./asset-stand-in"
import { ALL, CountToggle } from "./count-toggle"

/** How many columns the table shows before the reader asks for more. */
const COLUMN_PAGE_SIZES = [25, 50, 100] as const

/**
 * The columns to list: those whose name or description contains `query`,
 * cut to `limit`. `matched` is how many there are before the cut.
 */
export function visibleColumns<T extends { column: AssetColumn }>(
  rows: T[],
  query: string,
  limit: number
): { shown: T[]; matched: number } {
  const needle = query.trim().toLowerCase()
  const matching =
    needle === ""
      ? rows
      : rows.filter(
          (r) =>
            r.column.name.toLowerCase().includes(needle) ||
            (r.column.description ?? "").toLowerCase().includes(needle)
        )
  return { shown: matching.slice(0, limit), matched: matching.length }
}

/**
 * Null share as a thin meter plus its number. The number carries the
 * meaning; the bar only lets the eye find the gappy columns in a long list.
 */
function NullMeter({ fraction }: { fraction: number }) {
  return (
    <span className="flex items-center gap-2">
      <span className="h-1.5 w-12 overflow-hidden rounded-full bg-muted" aria-hidden>
        <span
          className="block h-full rounded-full bg-foreground/40"
          style={{ width: `${Math.min(fraction, 1) * 100}%` }}
        />
      </span>
      <span className="tabular-nums">{formatPercent(fraction)}</span>
    </span>
  )
}

/** One row of the table: a catalog column with what the Iceberg table says about it. */
type SchemaRow = {
  column: AssetColumn
  /** `null` when neither Iceberg nor the declared type settles it. */
  nullable: boolean | null
  /** The partition transform this column feeds, e.g. `day`. */
  partition: string | null
}

/**
 * The catalog's columns in the table's own order, each with whether it can
 * be null and whether the table is partitioned by it, plus the table's
 * columns the catalog does not list (load bookkeeping such as
 * `_ingested_at`).
 *
 * Nullability comes from Iceberg's `required` flag when the table is
 * loaded; otherwise from a ClickHouse `Nullable(...)` type, which says
 * "yes" but whose absence proves nothing about a registry-declared type.
 */
export function schemaRows(
  schema: AssetColumn[],
  table: LakehouseTableDetail | null,
  clickhouseTyped: boolean
): { rows: SchemaRow[]; system: SchemaRow[] } {
  const fields = table?.schema ?? []
  const position = new Map(fields.map((f, i) => [f.name, i]))
  const partitionOf = new Map((table?.partitionSpec ?? []).map((p) => [p.sourceId, p.transform]))
  const row = (column: AssetColumn): SchemaRow => {
    const field = fields.find((f) => f.name === column.name)
    const declaredNullable = column.dataType.startsWith("Nullable(")
    return {
      column,
      nullable: field ? !field.required : declaredNullable ? true : clickhouseTyped ? false : null,
      partition: field ? (partitionOf.get(field.id) ?? null) : null,
    }
  }
  const listed = new Set(schema.map((c) => c.name))
  const rows = schema
    .map((c, i) => ({ c, at: position.get(c.name) ?? fields.length + i }))
    .sort((a, b) => a.at - b.at)
    .map(({ c }) => row(c))
  const system = fields
    .filter((f) => !listed.has(f.name))
    .map((f) => row({ name: f.name, dataType: f.type }))
  return { rows, system }
}

type ProfileState =
  | { kind: "restricted" }
  | { kind: "loading" }
  | { kind: "error"; message: string; retry: () => void }
  | { kind: "unsupported"; reason: string }
  | { kind: "ready"; profile: Extract<AssetProfile, { supported: true }>; byName: Map<string, ColumnProfile> }

/** Per-column stats, read from the data — so they need `query:read`, like running a query. */
function useProfile(assetId: string): ProfileState {
  const { hasPermission } = useAuth()
  const allowed = hasPermission("query:read")
  const state = useService(
    (s) =>
      !allowed
        ? Promise.resolve(null)
        : assetService.getAssetProfile
          ? assetService.getAssetProfile(assetId, s)
          : Promise.reject(new Error("This deployment cannot profile assets.")),
    [allowed, assetId]
  )
  if (!allowed) return { kind: "restricted" }
  if (state.status === "loading") return { kind: "loading" }
  if (state.status === "error") return { kind: "error", message: state.error.message, retry: state.reload }
  const p = state.data
  if (p === null) return { kind: "restricted" }
  if (!p.supported) return { kind: "unsupported", reason: p.reason }
  return { kind: "ready", profile: p, byName: new Map(p.columns.map((c) => [c.name, c])) }
}

/** What the stats were computed over, or why there are none. */
function ProfileNote({ state }: { state: ProfileState }) {
  if (state.kind === "loading") return <span>Profiling columns…</span>
  if (state.kind === "restricted") {
    return <span>Column statistics are read from the data, so they need the query:read permission.</span>
  }
  if (state.kind === "unsupported") return <span>No column statistics: {state.reason}</span>
  if (state.kind === "error") {
    return (
      <span className="flex items-center gap-2">
        Column statistics failed to load: {state.message}
        <Button size="sm" variant="ghost" onClick={state.retry}>
          Retry
        </Button>
      </span>
    )
  }
  const p = state.profile
  return (
    <span>
      Statistics over {formatNumber(p.rowsProfiled)} rows of <span className="font-mono">{p.source}</span>
      {p.sourceKind === "iceberg" ? " (Iceberg)" : ""}
      {p.sampled ? ` (first ${formatNumber(p.rowLimit)} only)` : ""}
      {p.columnsCapped ? " · some columns left out" : ""}. Masking and row filters apply.
    </span>
  )
}

function StatCells({ state, name }: { state: ProfileState; name: string }) {
  if (state.kind === "loading") {
    return (
      <TableCell colSpan={4} className="py-1.5 align-top">
        <Skeleton className="h-3.5 w-40" />
      </TableCell>
    )
  }
  const c = state.kind === "ready" ? state.byName.get(name) : undefined
  if (!c) return <TableCell colSpan={4} className="py-1.5 align-top text-xs text-muted-foreground">—</TableCell>
  if (!c.profiled) {
    return (
      <TableCell colSpan={4} className="py-1.5 align-top text-xs text-muted-foreground">
        Not profiled (type not supported)
      </TableCell>
    )
  }
  const range =
    c.min != null && c.max != null ? (c.min === c.max ? c.min : `${c.min} – ${c.max}`) : "—"
  return (
    <>
      <TableCell className="py-1.5 align-top text-xs">
        {c.nullFraction == null ? "—" : <NullMeter fraction={c.nullFraction} />}
      </TableCell>
      <TableCell className="py-1.5 align-top text-xs tabular-nums" title="Approximate">
        {c.distinctCount == null ? "—" : `≈ ${formatCompactNumber(c.distinctCount)}`}
      </TableCell>
      <TableCell className="max-w-56 truncate py-1.5 align-top font-mono text-xs" title={range}>
        {range}
      </TableCell>
      <TableCell className="py-1.5 align-top">
        {c.topValues && c.topValues.length > 0 ? (
          <span className="flex flex-wrap gap-1">
            {c.topValues.map((t) => (
              <Pill
                key={t.value}
                tone="neutral"
                className="max-w-40 font-mono"
                title={`${t.value} · ${formatNumber(t.count)} rows`}
              >
                <span className="truncate">{t.value === "" ? "(empty)" : t.value}</span>
                <span className="text-muted-foreground/70 tabular-nums">
                  {formatCompactNumber(t.count)}
                </span>
              </Pill>
            ))}
          </span>
        ) : (
          <span className="text-xs text-muted-foreground">Mostly unique</span>
        )}
      </TableCell>
    </>
  )
}

function ColumnCells({ row }: { row: SchemaRow }) {
  const c = row.column
  return (
    <>
      <TableCell className="py-1.5 align-top">
        <span className="flex flex-wrap items-center gap-1.5">
          <span className="font-mono text-xs font-medium">{c.name}</span>
          {c.masked ? <Pill tone="warning">masked</Pill> : null}
          {c.classification ? <ClassificationBadge classification={c.classification} /> : null}
          {row.partition ? (
            <Pill tone="neutral" title="The table is partitioned by this column">
              partition · {row.partition}
            </Pill>
          ) : null}
        </span>
        {c.description ? (
          <div className="mt-0.5 max-w-md text-xs text-muted-foreground">{c.description}</div>
        ) : null}
      </TableCell>
      <TableCell className="py-1.5 align-top font-mono text-xs text-muted-foreground">{c.dataType}</TableCell>
      <TableCell className="py-1.5 align-top text-xs">
        {row.nullable === null ? "—" : row.nullable ? "Yes" : "No"}
      </TableCell>
    </>
  )
}

/**
 * How the table's schema got to where it is, newest first, each saying what
 * changed from the one before. An Iceberg table carries every schema it has
 * had, so those are listed as they are. A ClickHouse table keeps only its
 * current columns, so for a `silver.*`/`serving.*` table the list is the one
 * the console recorded itself (ADR 0015): it starts when the console first
 * looked, and the card says so rather than presenting it as the table's
 * whole history.
 */
function SchemaVersions({ asset: a, table }: { asset: AssetDetail; table: LakehouseTableDetail | null }) {
  const history = schemaHistory(table?.schemaVersions ?? [])
  const recorded = a.schemaVersions
  // The list is newest first, so the oldest entry is the last.
  const firstRecorded = recorded.length > 0 ? recorded[recorded.length - 1].at : null
  return (
    <SectionCard
      size="sm"
      title="Schema versions"
      description={
        history.length > 0 || a.type === "iceberg-table"
          ? "Every schema the table has had, most recent first."
          : firstRecorded
            ? `Recorded by the console each time this table's columns change, since ${formatDate(firstRecorded, { month: "short" })}. Changes before that are not known, and a renamed column shows as one dropped and one added.`
            : "Recorded by the console each time this table's columns change. A renamed column shows as one dropped and one added."
      }
    >
      {history.length > 0 ? (
        <ul className="divide-y divide-border text-sm">
          {history.map((v) => (
            <li key={v.schemaId} className="flex flex-wrap items-baseline gap-x-2 gap-y-1 py-1.5">
              <span className="font-mono text-xs text-muted-foreground">v{v.schemaId}</span>
              {v.current ? <Pill tone="neutral">current</Pill> : null}
              <span className="min-w-0 flex-1">{v.changes.join(" · ")}</span>
              <span
                className="text-xs text-muted-foreground"
                title={v.sinceMs === null ? "No snapshot the table still keeps was written with this schema" : undefined}
              >
                {v.sinceMs === null ? "no data written" : `since ${snapshotRelativeTime(v.sinceMs)}`}
              </span>
            </li>
          ))}
        </ul>
      ) : recorded.length > 0 ? (
        <ul className="divide-y divide-border text-sm">
          {recorded.map((v) => (
            <li key={v.version} className="flex flex-wrap items-baseline gap-x-2 gap-y-1 py-1.5">
              <span className="font-mono text-xs text-muted-foreground">v{v.version}</span>
              {v.current ? <Pill tone="neutral">current</Pill> : null}
              <span className="min-w-0 flex-1">{v.change}</span>
              <span
                className="text-xs text-muted-foreground"
                title={`When the console saw the table with these columns: ${formatDateTime(v.at)}`}
              >
                recorded {formatRelativeTime(v.at)}
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState
          title={a.type === "iceberg-table" ? "No schema history available" : "No schema version recorded yet"}
          description={
            a.type === "iceberg-table"
              ? undefined
              : "The console records a version the next time it looks at this table: when a run finishes, and on a schedule."
          }
          className="py-4"
        />
      )}
    </SectionCard>
  )
}

/**
 * The Schema tab: one table of the asset's columns — type, nullability,
 * sensitivity, and what the data in each actually looks like — then the
 * table's own bookkeeping columns and its schema history.
 */
export function AssetSchema({ asset: a, iceberg }: { asset: AssetDetail; iceberg: IcebergTableState }) {
  const profile = useProfile(a.id)
  const table = icebergTableOf(iceberg)
  // A `silver.*`/`serving.*` asset's types are ClickHouse's own, where a
  // column that is not `Nullable(...)` cannot be null.
  const clickhouseTyped = a.type !== "iceberg-table"
  const { rows, system } = schemaRows(a.schema, table, clickhouseTyped)
  const showStats = profile.kind === "ready" || profile.kind === "loading"
  const [limit, setLimit] = React.useState<number>(COLUMN_PAGE_SIZES[0])
  const [query, setQuery] = React.useState("")
  // A table that fits the first page needs neither control.
  const wide = rows.length > COLUMN_PAGE_SIZES[0]
  // Only the sizes that would cut this table short, then all of it.
  const sizes = [...COLUMN_PAGE_SIZES.filter((n) => n < rows.length), ALL]
  const { shown, matched } = visibleColumns(rows, query, wide ? limit : ALL)
  const total = `${rows.length} column${rows.length === 1 ? "" : "s"}`
  const summary =
    query.trim() !== ""
      ? `${matched} of ${total} match "${query.trim()}"${shown.length < matched ? `, showing the first ${shown.length}` : ""}.`
      : shown.length < rows.length
        ? `Showing the first ${shown.length} of ${total}, in table order.`
        : `${total}, in table order.`

  return (
    <div className="flex flex-col gap-2">
      <StandInNotice asset={a} />
      <SectionCard
        size="sm"
        title="Columns"
        description={summary}
        action={
          wide ? (
            <div className="flex flex-wrap items-center justify-end gap-x-3 gap-y-1.5">
              <Input
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter columns…"
                aria-label="Filter columns"
                className="h-8 w-44"
              />
              <CountToggle
                label="Show"
                ariaLabel="Columns to show"
                options={sizes}
                value={limit}
                onChange={setLimit}
              />
            </div>
          ) : undefined
        }
      >
        {rows.length === 0 ? (
          <EmptyState title="No columns registered" className="py-4" />
        ) : shown.length === 0 ? (
          <EmptyState title={`No column matches "${query.trim()}"`} className="py-4" />
        ) : (
          <div className="flex flex-col gap-2">
            <div className="overflow-hidden rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow className="hover:bg-transparent">
                    <TableHead className="text-xs">Column</TableHead>
                    <TableHead className="text-xs">Type</TableHead>
                    <TableHead className="text-xs">Nullable</TableHead>
                    {showStats ? (
                      <>
                        <TableHead className="text-xs">Nulls</TableHead>
                        <TableHead className="text-xs">Distinct</TableHead>
                        <TableHead className="text-xs">Range</TableHead>
                        <TableHead className="text-xs">Top values</TableHead>
                      </>
                    ) : null}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {shown.map((row) => (
                    <TableRow key={row.column.name}>
                      <ColumnCells row={row} />
                      {showStats ? <StatCells state={profile} name={row.column.name} /> : null}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            <p className="text-xs text-muted-foreground">
              <ProfileNote state={profile} />
            </p>
          </div>
        )}
      </SectionCard>

      {system.length > 0 ? (
        <SectionCard
          size="sm"
          title="System columns"
          description="Written by the load itself, not part of the dataset: when a row arrived and in which load."
        >
          <div className="overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="text-xs">Column</TableHead>
                  <TableHead className="text-xs">Type</TableHead>
                  <TableHead className="text-xs">Nullable</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {system.map((row) => (
                  <TableRow key={row.column.name}>
                    <ColumnCells row={row} />
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </SectionCard>
      ) : null}

      <SchemaVersions asset={a} table={table} />
    </div>
  )
}
