/**
 * Reusable metrics and segments (BI-2, with AI-7). Mirrors
 * `rust/crates/lakehouse-bi/src/metrics.rs` and the routes in
 * `rust/crates/lakehouse-api/src/routes/metrics.rs`.
 *
 * A metric is a named aggregate formula over one table or SQL source, with
 * filters of its own; a chart picks it by name (`{ metric }` as a measure) and
 * every chart that does shows the same number. A segment is a named list of
 * filters, used as `{ segment }` where a chart filter stands. Reading needs
 * `dashboard:read`; defining, changing and deleting need `metric:write`.
 */

/** Exactly one of the two: a mart, or a dashboard SQL source. */
export type MetricSource = { mart: string; source?: undefined } | { source: string; mart?: undefined }

/**
 * A filter as the server stores it (the same shape as a dashboard filter:
 * `column`, `op`, `values`, `min`, `max`, `text`, `unit`, `n`, `anchor`, ...), or a
 * reference to a segment by name. Kept loose here because contracts import no client.
 */
export type StoredFilter = { column?: string; values?: string[]; op?: string; segment?: string } & Record<string, unknown>
/** A filter in a metric or a chart: a plain filter, or a reference to a segment by name. */
export type MetricFilter = StoredFilter

export type Metric = {
  id: string
  name: string
  label: string
  description: string
  sourceKind: "mart" | "sql_source" | string
  sourceId: string
  formula: string
  filters: MetricFilter[]
  /** The date column its comparisons run on, and its default period. */
  timeColumn?: string
  grain?: string
  format?: string
  createdBy: string
  updatedAt?: string
  /** List only: how many charts use it, and its source's title. */
  usedBy?: number
  sourceTitle?: string
}

/** What a metric is saved with; the name and the source cannot change afterwards. */
export type MetricDraft = {
  label: string
  description: string
  formula: string
  filters: MetricFilter[]
  timeColumn?: string
  grain?: string
  format?: string
}

export type Segment = {
  id: string
  name: string
  label: string
  sourceKind: "mart" | "sql_source" | string
  sourceId: string
  filters: StoredFilter[]
  createdBy: string
  updatedAt?: string
}

export type SegmentDraft = { label: string; filters: StoredFilter[] }

/** What uses a metric, by title (the answer of `GET /api/dashboard/metrics/{id}/usage`). */
export type MetricUsage = {
  metric: string
  charts: { id: string; title: string; kind: string; board: string; boardName: string }[]
  alerts: { id: string; name: string }[]
  dashboards: string[]
}

export type MetricCheck =
  | { ok: true; value?: { rows?: Record<string, unknown>[]; error?: string; errorId?: string } }
  | { ok: false; error: { message: string } }

export type MetricService = {
  list(signal?: AbortSignal): Promise<Metric[]>
  usage(id: string, signal?: AbortSignal): Promise<MetricUsage>
  /** Checks only; with `value` it also reads the number as the caller. A mistake is a normal `ok: false`. */
  validate(source: MetricSource, draft: MetricDraft, opts?: { name?: string; value?: boolean; signal?: AbortSignal }): Promise<MetricCheck>
  /** A refusal (a name in use, a definition that does not check out, no permission) arrives with the server's sentence. */
  create(source: MetricSource, name: string, draft: MetricDraft, signal?: AbortSignal): Promise<Metric>
  update(id: string, draft: MetricDraft, signal?: AbortSignal): Promise<Metric>
  /** 409 (as `invalid_request`) names the charts still using it. */
  remove(id: string, signal?: AbortSignal): Promise<void>
  listSegments(signal?: AbortSignal): Promise<Segment[]>
  createSegment(source: MetricSource, name: string, draft: SegmentDraft, signal?: AbortSignal): Promise<Segment>
  updateSegment(id: string, draft: SegmentDraft, signal?: AbortSignal): Promise<Segment>
  removeSegment(id: string, signal?: AbortSignal): Promise<void>
}
