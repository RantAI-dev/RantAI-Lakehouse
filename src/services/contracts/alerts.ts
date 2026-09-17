/**
 * Threshold-alert and scheduled-digest rules, mirroring
 * `lakehouse_alerts::{AlertRule, AlertRuleInput, RunResult}`
 * (`rust/crates/lakehouse-alerts/src/lib.rs`) and
 * `lakehouse_notify::DeliverResult` exactly.
 */

/**
 * Mirrors `AlertKind` (`#[serde(rename_all = "lowercase")]`,
 * `rust/crates/lakehouse-alerts/src/lib.rs`), which accepts exactly
 * `alert`, `digest`, `freshness`. A `freshness` rule reuses the `mart`
 * field as its `<namespace>.<table>` target and the backend clears
 * `measure`/`agg`/`threshold`/`board` for that kind (WS5 item E3).
 */
export type AlertRuleKind = "alert" | "digest" | "freshness" | string

/** Mirrors `AlertChannel` (`#[serde(rename_all = "lowercase")]`). */
export type AlertChannel = "webhook" | "email" | string

/** Mirrors `AlertOp` (rename to the literal operator string). */
export type AlertOp = ">" | ">=" | "<" | "<=" | "==" | string

/**
 * A stored alert or digest rule. `mart`/`measure`/`board` are omitted from
 * the wire body (not `null`) when unset — `AlertRule`'s
 * `skip_serializing_if = "Option::is_none"` — hence optional here rather
 * than `| null`.
 */
export type AlertRule = {
  id: string
  name: string
  type: AlertRuleKind
  mart?: string
  measure?: string
  agg: string
  op: AlertOp
  threshold: number
  board?: string
  channel: AlertChannel
  target: string
  enabled: boolean
  createdAt?: string
  /**
   * Copied verbatim onto a fired instance's `AlertItem.severity` (WS5 item
   * C1). Omitted from the wire body (not `null`) when unset, mirroring
   * `mart`/`measure`/`board` above — a rule with no severity fires
   * instances with `severity: null`, never a fabricated default.
   */
  severity?: string
}

/**
 * Body accepted by `POST`/`PUT /api/alerts`, mirroring `AlertRuleInput`.
 * Every field is optional/raw on the wire — the server normalizes and
 * falls back permissively (see `normalize_input`) rather than rejecting an
 * unrecognized `type`/`channel`/`op`.
 */
export type SaveAlertRuleInput = {
  id?: string
  name?: string
  type?: string
  mart?: string
  measure?: string
  agg?: string
  op?: string
  threshold?: number
  board?: string
  channel?: string
  target?: string
  enabled?: boolean
  severity?: string
}

/** Mirrors `lakehouse_notify::DeliverResult`. */
export type AlertDeliverResult = {
  ok: boolean
  error?: string
}

/** Mirrors `lakehouse_alerts::RunResult`. */
export type AlertRunResult = {
  id: string
  name: string
  type: AlertRuleKind
  fired: boolean
  value?: number
  delivered?: AlertDeliverResult
  skipped?: string
}

export interface AlertRuleService {
  listRules(signal?: AbortSignal): Promise<AlertRule[]>
  createRule(input: SaveAlertRuleInput, signal?: AbortSignal): Promise<AlertRule>
  updateRule(rule: SaveAlertRuleInput & { id: string }, signal?: AbortSignal): Promise<AlertRule>
  deleteRule(id: string, signal?: AbortSignal): Promise<void>
  runRules(
    id: string | undefined,
    signal?: AbortSignal
  ): Promise<{ ran: number; results: AlertRunResult[] }>
}
