"use client"

import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
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
import { formatDate, formatDateTime, formatRelativeTime } from "@/lib/format"
import { snapshotRelativeTime } from "@/lib/lakehouse-view"
import { schemaHistory } from "@/lib/schema-history"
import { assetService } from "@/services"
import type { AssetColumn, AssetDetail } from "@/services/contracts/assets"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
import { ColumnsCard, type ProfileState, type SchemaRow } from "./asset-columns"
import { icebergTableOf, type IcebergTableState } from "./asset-storage"
import { StandInNotice } from "./asset-stand-in"

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
 * The Schema tab: the asset's columns as a column explorer (type, nullability,
 * sensitivity, and what the data in each actually looks like; see
 * `asset-columns.tsx`), then the table's own bookkeeping columns and its
 * schema history.
 */
export function AssetSchema({ asset: a, iceberg }: { asset: AssetDetail; iceberg: IcebergTableState }) {
  const profile = useProfile(a.id)
  const table = icebergTableOf(iceberg)
  // A `silver.*`/`serving.*` asset's types are ClickHouse's own, where a
  // column that is not `Nullable(...)` cannot be null.
  const clickhouseTyped = a.type !== "iceberg-table"
  const { rows, system } = schemaRows(a.schema, table, clickhouseTyped)

  return (
    <div className="flex flex-col gap-2">
      <StandInNotice asset={a} />
      <ColumnsCard rows={rows} profile={profile} sortingKey={a.storage?.sortingKey} />

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
