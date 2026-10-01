"use client"

import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
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
import { formatCompactNumber, formatNumber, formatPercent, formatRelativeTime } from "@/lib/format"
import { snapshotRelativeTime } from "@/lib/lakehouse-view"
import { schemaHistory } from "@/lib/schema-history"
import { assetService } from "@/services"
import type { AssetColumn, AssetDetail, AssetProfile, ColumnProfile } from "@/services/contracts/assets"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
import { icebergTableOf, type IcebergTableState } from "./asset-storage"

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
 * How the table's schema got to where it is: one entry per Iceberg schema,
 * newest first, saying what changed from the one before. A ClickHouse
 * table keeps no such history, and the card says so rather than "none".
 */
function SchemaVersions({ asset: a, table }: { asset: AssetDetail; table: LakehouseTableDetail | null }) {
  const history = schemaHistory(table?.schemaVersions ?? [])
  return (
    <SectionCard
      size="sm"
      title="Schema versions"
      description="Every schema the table has had, most recent first."
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
      ) : a.schemaVersions.length > 0 ? (
        <ul className="divide-y divide-border text-sm">
          {a.schemaVersions.map((v) => (
            <li key={v.version} className="flex flex-wrap items-baseline gap-2 py-1.5">
              <span className="font-mono text-xs text-muted-foreground">v{v.version}</span>
              <span>{v.change}</span>
              <span className="ml-auto text-xs text-muted-foreground">{formatRelativeTime(v.at)}</span>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState
          title={
            a.type === "iceberg-table"
              ? "No schema history available"
              : "Only Iceberg tables record schema history"
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

  return (
    <div className="flex flex-col gap-2">
      <SectionCard
        size="sm"
        title="Columns"
        description={`${rows.length} column${rows.length === 1 ? "" : "s"}, in table order.`}
      >
        {rows.length === 0 ? (
          <EmptyState title="No columns registered" className="py-4" />
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
                  {rows.map((row) => (
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
