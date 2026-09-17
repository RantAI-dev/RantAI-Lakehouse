/**
 * Client-side model for the pipeline-create page's transform-vocabulary
 * builder (WS4 item F5). Mirrors
 * `rust/crates/lakehouse-api/src/transform_grammar.rs`'s `parse_transform`
 * grammar field for field — the same five verbs
 * (`dedupe`/`filter`/`rename`/`cast`/`select`), the same
 * `ALLOWED_FILTER_OPERATORS` set, and the same `ALLOWED_CAST_TYPES` set —
 * so the browser only ever offers a string the server can already accept.
 *
 * This mirror is a UX nicety (a `<select>` a user cannot type outside of),
 * never a second source of truth: `POST /api/pipelines` re-validates every
 * rendered string through `transform_grammar::parse_transform` regardless
 * of what produced it, and remains the sole validating authority (a stale
 * client build, or any caller bypassing this UI, is still rejected there).
 */

/** Mirrors `transform_grammar.rs`'s `ALLOWED_FILTER_OPERATORS` exactly. */
export const FILTER_OPERATORS = ["=", "!=", "<", "<=", ">", ">="] as const

/** Mirrors `transform_grammar.rs`'s `ALLOWED_CAST_TYPES` exactly. */
export const CAST_TYPES = [
  "String",
  "Int32",
  "Int64",
  "Float64",
  "Boolean",
  "Date",
  "DateTime",
  "UUID",
] as const

export type FilterOperator = (typeof FILTER_OPERATORS)[number]
export type CastType = (typeof CAST_TYPES)[number]

/** One transform row as built by the create page, before it is rendered to the grammar string the server parses. */
export type TransformDraft =
  | { verb: "dedupe"; key: string }
  | { verb: "filter"; column: string; operator: FilterOperator; value: string }
  | { verb: "rename"; from: string; to: string }
  | { verb: "cast"; column: string; type: CastType }
  | { verb: "select"; columns: string }

/**
 * Renders a draft into the exact grammar string
 * `transform_grammar::parse_transform` accepts for that verb. Every
 * shape here corresponds 1:1 to a `parse_transform` match arm:
 * `dedupe(key)`, `filter(col op 'val')`, `rename(from,to)`,
 * `cast(col,type)`, `select(cols)`.
 */
export function renderTransformDraft(draft: TransformDraft): string {
  switch (draft.verb) {
    case "dedupe":
      return `dedupe(${draft.key})`
    case "filter":
      return `filter(${draft.column} ${draft.operator} '${draft.value}')`
    case "rename":
      return `rename(${draft.from},${draft.to})`
    case "cast":
      return `cast(${draft.column},${draft.type})`
    case "select":
      return `select(${draft.columns})`
  }
}

/**
 * Extracts the failing row index from a `POST /api/pipelines` 400 body's
 * `error` message, which `routes/pipelines.rs::create` always shapes as
 * `"invalid transform at transforms[<index>]: <reason>"` (see
 * `rust/crates/lakehouse-api/src/routes/pipelines.rs`, the
 * `parse_transform` validation loop). Returns `null` when the message
 * does not name a `transforms[N]` index — a different validation failure,
 * or a non-validation error (network, 500, etc.) — so the caller can fall
 * back to a generic, page-level error instead of guessing a row.
 */
export function transformErrorRowIndex(message: string): number | null {
  const match = /transforms\[(\d+)\]/.exec(message)
  if (!match) return null
  return Number(match[1])
}
