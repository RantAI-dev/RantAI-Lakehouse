"use client"

/**
 * What a column list is built from, shared by the two tabs that open a
 * column: the Schema tab's explorer and the Sample tab's inspector. Both
 * read the table's columns through `schemaRows` and its statistics through
 * `useProfile`, so the same column states the same facts on either tab.
 */
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { assetService } from "@/services"
import type { AssetColumn } from "@/services/contracts/assets"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
import type { ProfileState, SchemaRow } from "./asset-columns"

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
export function useProfile(assetId: string): ProfileState {
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
