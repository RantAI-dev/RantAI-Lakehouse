/**
 * Threshold-alert and scheduled-digest rules, mirroring
 * `lakehouse_alerts::{AlertRule, AlertRuleInput, RunResult}`
 * (`rust/crates/lakehouse-alerts/src/lib.rs`) and
 * `lakehouse_notify::DeliverResult` exactly.
 */

/**
 * Mirrors `AlertKind` (`rust/crates/lakehouse-alerts/src/lib.rs`): `alert`,
 * `digest`, `freshness`, the four pipeline kinds (which have no form here and
 * are written `pipelinefailure` etc. on the wire because the enum is
 * `rename_all = "lowercase"`), and the six `SRC-7` kinds below, which carry an
 * explicit snake_case rename. A `freshness` rule reuses the `mart` field as its
 * `<namespace>.<table>` target and the backend clears
 * `measure`/`agg`/`threshold`/`board` for that kind (WS5 item E3).
 */
export type AlertRuleKind = "alert" | "digest" | "freshness" | string

/**
 * The six rule kinds for connector and upload loads (`SRC-7`). The first five
 * are scoped to one connector id, or `*` for all connectors (an administrator
 * who sees every tenant only; never for `connector_success`); `upload_failure`
 * is always scoped to every upload and sends no connector.
 */
export const LOAD_ALERT_KINDS = [
  "connector_failure",
  "connector_repeated_failure",
  "connector_disabled",
  "connector_schema_change",
  "connector_success",
  "upload_failure",
] as const

export type LoadAlertKind = (typeof LOAD_ALERT_KINDS)[number]

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
  /**
   * A connector id, or `*` for all connectors, on the `SRC-7` kinds
   * (`upload_failure` stores `*`). Omitted from the wire body when unset, like
   * `pipeline`.
   */
  connector?: string
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
  /** Connector id or `*`; see [`AlertRule.connector`]. */
  connector?: string
}

/** Body of `GET /api/alerts/status`. */
export type AlertsStatus = {
  /**
   * True when the API has `PIPELINE_RUN_TOKEN` set, the condition under which
   * the orchestrator's run sensors can report failed and finished runs.
   * False means no connector or upload alert can fire. It cannot see the
   * orchestrator's own copy of the token.
   */
  runEventsConfigured: boolean
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
  getStatus(signal?: AbortSignal): Promise<AlertsStatus>
  createRule(input: SaveAlertRuleInput, signal?: AbortSignal): Promise<AlertRule>
  updateRule(rule: SaveAlertRuleInput & { id: string }, signal?: AbortSignal): Promise<AlertRule>
  deleteRule(id: string, signal?: AbortSignal): Promise<void>
  runRules(
    id: string | undefined,
    signal?: AbortSignal
  ): Promise<{ ran: number; results: AlertRunResult[] }>
}
