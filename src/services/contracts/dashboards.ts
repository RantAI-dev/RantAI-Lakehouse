/**
 * Dashboard SQL sources, mirroring `lakehouse_bi::sources::{SqlSource,
 * SourceColumn}` (`rust/crates/lakehouse-bi/src/sources.rs`) and the
 * `/api/dashboard/sources` routes in
 * `rust/crates/lakehouse-api/src/routes/dashboard_sources.rs`.
 *
 * A SQL source is a saved read-only SELECT (usually a join across several
 * `serving` marts) a chart can read instead of one mart. Authoring needs the
 * `dashboard:sql` permission; listing needs `dashboard:read`.
 */

/** One column a source returns — `SourceColumn` (`type` on the wire). */
export type SqlSourceColumn = {
  name: string
  type: string
}

/** `SqlSource` (`#[serde(rename_all = "camelCase")]`). */
export type SqlSource = {
  id: string
  title: string
  sql: string
  columns: SqlSourceColumn[]
  /** Empty string = root. */
  folderId: string
  /** Principal id of whoever last saved it. */
  createdBy: string
  /** When this version was saved; every save is a new version. */
  updatedAt?: string
}

/** `POST`/`PUT /api/dashboard/sources` body. */
export type SaveSqlSourceInput = {
  title: string
  sql: string
  folderId?: string
}

/** `POST /api/dashboard/sources/preview` response. */
export type SqlSourcePreview = {
  columns: SqlSourceColumn[]
  rows: Record<string, unknown>[]
}

export interface DashboardService {
  listSqlSources(signal?: AbortSignal): Promise<SqlSource[]>
  createSqlSource(input: SaveSqlSourceInput, signal?: AbortSignal): Promise<SqlSource>
  updateSqlSource(
    input: SaveSqlSourceInput & { id: string },
    signal?: AbortSignal
  ): Promise<SqlSource>
  deleteSqlSource(id: string, signal?: AbortSignal): Promise<void>
  previewSqlSource(sql: string, signal?: AbortSignal): Promise<SqlSourcePreview>
}
