/**
 * Hierarchies (BI-10, with AI-6): a named, ordered list of 2 to 6 columns of
 * one table or SQL source (province > city > place), used to drill down a
 * chart whose dimension is one of its levels. Mirrors
 * `rust/crates/lakehouse-bi/src/hierarchies.rs` and the routes in
 * `rust/crates/lakehouse-api/src/routes/hierarchies.rs`.
 *
 * Only column names are stored; charts do not store a hierarchy, so deleting
 * one never breaks a chart. Reading needs `dashboard:read`; defining,
 * changing and deleting need `dashboard:write`.
 */

export type Hierarchy = {
  id: string
  sourceKind: "mart" | "sql_source" | string
  sourceId: string
  name: string
  /** The columns, outermost first. */
  levels: string[]
  createdBy: string
  updatedAt?: string
}

/** Exactly one of the two. */
export type HierarchySource = { mart: string; source?: undefined } | { source: string; mart?: undefined }

export const MIN_LEVELS = 2
export const MAX_LEVELS = 6

export type HierarchyService = {
  list(source: HierarchySource, signal?: AbortSignal): Promise<Hierarchy[]>
  /** A refusal (a name in use, a column the source lacks) arrives as `invalid_request` with the server's sentence. */
  create(source: HierarchySource, name: string, levels: string[], signal?: AbortSignal): Promise<Hierarchy>
  update(id: string, name: string, levels: string[], signal?: AbortSignal): Promise<Hierarchy>
  remove(id: string, signal?: AbortSignal): Promise<void>
}
