//! Threshold alerts & scheduled digests over `serving.*` marts, persisted in
//! `console.alert_rule`, delivered via webhook/email (see
//! [`lakehouse_notify`]).
//!
//! Ports `src/services/clients/alert-store.ts` in full: rule CRUD
//! ([`list_rules`], [`save_rule`], [`delete_rule`]), the evaluator
//! ([`run_rules`]), the threshold comparison, and delivery dispatch. SQL is
//! assembled server-side from validated identifiers and escaped literals —
//! never raw string interpolation — matching the TypeScript's `esc()` +
//! `IDENT` regex discipline, but with the escaping baked into the
//! [`lakehouse_core::ident`] newtypes instead of a hand-rolled function.
//!
//! # Live schema
//!
//! `console.alert_rule` holds real data today. The `CREATE TABLE IF NOT
//! EXISTS` bootstrap in [`ensure`] is kept byte-identical to the
//! TypeScript's (same columns, same defaults, same engine) — verified
//! against `DESCRIBE console.alert_rule` / `SHOW CREATE TABLE
//! console.alert_rule` on the live cluster before this port was written.
//!
//! # What is NOT exported
//!
//! `compare`, `currentValue`, `digestText`, and `fmt` are private helpers in
//! the TypeScript (no `export` keyword) and stay private here too — only
//! [`list_rules`], [`get_rule`], [`save_rule`], [`delete_rule`], and
//! [`run_rules`], plus the [`AlertRule`]/[`AlertOp`]/[`RunResult`] types,
//! are part of this crate's public surface, mirroring the TypeScript
//! module's actual export list exactly.

use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::{Ident, SqlLiteral};
use lakehouse_notify::{DeliverResult, EmailSender, deliver};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// Aggregate functions `saveRule`/`currentValue` accept, mirroring the
/// TypeScript `AGGS` set.
const AGGS: &[&str] = &["sum", "avg", "max", "min", "count"];

fn aggregate_allowed(agg: &str) -> bool {
    AGGS.contains(&agg)
}

/// The five severities Postgres's `alert_instance.severity` `CHECK`
/// admits (`0038_alert_instance_rule_link.sql`, mirroring
/// `0011_overview_alerts.sql`'s original five) — the single place both
/// [`normalize_input`] and `0038`'s `CHECK` must agree, named so a future
/// edit to either side is easy to find the other.
const SEVERITIES: [&str; 5] = ["critical", "high", "medium", "low", "info"];

/// Errors produced while validating input or talking to `ClickHouse`
/// through this module. Ports the `Error` throws in `saveRule`.
#[derive(Debug, Error)]
pub enum AlertError {
    /// A user-facing validation failure (blank name, bad webhook URL, ...).
    /// Carries the same Indonesian-language message the TypeScript throws,
    /// so error bodies stay byte-identical for parity.
    #[error("{0}")]
    Validation(String),
    /// A `ClickHouse` failure while querying or writing.
    #[error(transparent)]
    Clickhouse(#[from] ChError),
}

/// The five comparison operators a threshold alert can use, mirroring the
/// TypeScript `AlertOp` union (`">" | ">=" | "<" | "<=" | "=="`).
///
/// Serializes/deserializes as the literal operator string (e.g. `">="`),
/// matching the JSON shape `AlertRule.op` has on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlertOp {
    /// `>`
    #[serde(rename = ">")]
    #[default]
    Gt,
    /// `>=`
    #[serde(rename = ">=")]
    Ge,
    /// `<`
    #[serde(rename = "<")]
    Lt,
    /// `<=`
    #[serde(rename = "<=")]
    Le,
    /// `==`
    #[serde(rename = "==")]
    Eq,
}

impl AlertOp {
    /// The literal operator string, as stored in `console.alert_rule.op`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
        }
    }

    /// Parse `s` into an [`AlertOp`], returning `None` unless it matches one
    /// of the five operator strings exactly.
    ///
    /// Ports the `OPS.includes(x as AlertOp)` allowlist check used in both
    /// `saveRule` and `toRule` — deliberately permissive at the call site:
    /// every caller here falls back to [`AlertOp::Gt`] on `None` rather than
    /// surfacing a parse error, exactly like the TypeScript.
    #[must_use]
    pub fn parse_exact(s: &str) -> Option<Self> {
        match s {
            ">" => Some(Self::Gt),
            ">=" => Some(Self::Ge),
            "<" => Some(Self::Lt),
            "<=" => Some(Self::Le),
            "==" => Some(Self::Eq),
            _ => None,
        }
    }
}

/// Whether a rule is a threshold alert or a scheduled digest, mirroring the
/// TypeScript `AlertRule["type"]` union (`"alert" | "digest"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertKind {
    /// Watches one Gold-mart aggregate; fires when it crosses a threshold.
    Alert,
    /// Sends a KPI/gauge tile summary of a board, on demand or on a
    /// schedule.
    Digest,
    /// Evaluates a `dataset_sla` row's freshness, not a `serving.*`
    /// `ClickHouse` mart — see `run_freshness` (a later commit; this kind
    /// has no evaluator yet, see `run_one`'s `match` below). WAL-slot
    /// health needs NO new kind: an ordinary `AlertKind::Alert` rule
    /// already targets `serving.replication_slot_health`
    /// (`dagster/dispar_orchestrate/replication_metrics.py`'s own header
    /// names the exact rule to create). WS5 plan review Y4.
    Freshness,
}

impl AlertKind {
    /// The literal kind string, as stored in `console.alert_rule.type`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Digest => "digest",
            Self::Freshness => "freshness",
        }
    }
}

/// Delivery channel, mirroring the TypeScript `AlertRule["channel"]` union
/// (`"webhook" | "email"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertChannel {
    /// `POST` to an incoming-webhook URL.
    Webhook,
    /// Send via `SMTP`.
    Email,
}

impl AlertChannel {
    /// The literal channel string, as stored in `console.alert_rule.channel`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Email => "email",
        }
    }
}

/// A stored alert or digest rule, mirroring the TypeScript `AlertRule`
/// type.
///
/// `mart`/`measure`/`board` are `Option<String>` because `toRule` in the
/// TypeScript maps an empty `ClickHouse` column to `undefined`
/// (`r.mart || undefined`), which `JSON.stringify` then omits from the
/// response body entirely — reproduced here with
/// `skip_serializing_if = "Option::is_none"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    /// Stable identifier (`al_<8 hex>` when server-generated).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this is a threshold alert or a scheduled digest.
    #[serde(rename = "type")]
    pub kind: AlertKind,
    /// Source mart (unqualified, e.g. `mart_wisman`). Only set for
    /// `AlertKind::Alert`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mart: Option<String>,
    /// Measure column. Only set for `AlertKind::Alert`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub measure: Option<String>,
    /// Aggregate function (`sum`/`avg`/`max`/`min`/`count`). Always
    /// present, defaulting to `"sum"`.
    #[serde(default = "default_agg")]
    pub agg: String,
    /// Comparison operator. Always present, defaulting to
    /// [`AlertOp::Gt`].
    #[serde(default)]
    pub op: AlertOp,
    /// Threshold value. Always present (`0` for digest rules, which don't
    /// use it).
    #[serde(default)]
    pub threshold: f64,
    /// Board id to summarize. Only set for `AlertKind::Digest`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub board: Option<String>,
    /// Delivery channel.
    pub channel: AlertChannel,
    /// Webhook URL or email address.
    pub target: String,
    /// Whether this rule is evaluated by [`run_rules`].
    pub enabled: bool,
    /// `ClickHouse`-formatted creation timestamp.
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
    /// Severity this rule's fired instances carry (WS5 plan review Y3: a
    /// rule's severity is copied verbatim from this field into a fired
    /// `alert_instance` row, never invented from `kind`). `None` when the
    /// rule was saved without one — an honest "no severity," not a
    /// silently-defaulted one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub severity: Option<String>,
}

fn default_agg() -> String {
    "sum".to_owned()
}

/// Input accepted by [`save_rule`], mirroring the TypeScript's
/// `Partial<AlertRule> & { id?: string }` (the `PUT` route reads `id` off
/// this same shape).
///
/// Every field is a raw, unvalidated `Option<String>`/`Option<f64>` —
/// deliberately, not the strict [`AlertKind`]/[`AlertChannel`]/[`AlertOp`]
/// enums — because the TypeScript never rejects an unrecognized `type`,
/// `channel`, or `op` value on input; it silently falls back to a default
/// (`saveRule`'s `input.type === "digest" ? "digest" : "alert"` and
/// friends). A strict-enum field here would turn an unrecognized value into
/// a hard deserialize failure instead of that same permissive fallback, so
/// this struct stays untyped and [`save_rule`] does the TypeScript's exact
/// string-equality checks itself.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlertRuleInput {
    /// Rule id, for `PUT` (update). Ignored by [`save_rule`] itself — the
    /// caller passes it separately as `id`; this field exists only so the
    /// route handler can read `body.id` from the same deserialized struct
    /// used for `POST`.
    pub id: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Raw `"alert"`/`"digest"` string; anything else falls back to
    /// `"alert"`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Source mart, optionally `serving.`-prefixed (the prefix is
    /// stripped).
    pub mart: Option<String>,
    /// Measure column.
    pub measure: Option<String>,
    /// Raw aggregate string, lowercased and checked against [`AGGS`].
    pub agg: Option<String>,
    /// Raw operator string; anything not exactly one of the five operators
    /// falls back to `">"`.
    pub op: Option<String>,
    /// Threshold value.
    pub threshold: Option<f64>,
    /// Board id, for digests.
    pub board: Option<String>,
    /// Raw `"webhook"`/`"email"` string; anything else falls back to
    /// `"webhook"`.
    pub channel: Option<String>,
    /// Webhook URL or email address.
    pub target: Option<String>,
    /// Whether the rule is active. `Some(false)` disables it; anything
    /// else (including `None`) enables it, matching `input.enabled ===
    /// false ? 0 : 1`.
    pub enabled: Option<bool>,
    /// Raw severity string, validated against [`SEVERITIES`] by
    /// [`normalize_input`] — unlike `kind`/`channel`/`op`, an unrecognized
    /// value here is rejected outright rather than silently falling back
    /// to a default (WS5 plan review U12): a rule that claims a severity
    /// `alert_instance`'s `CHECK` will not actually admit must never save.
    pub severity: Option<String>,
}

/// The validated, normalized shape [`save_rule`] persists — split out of
/// [`save_rule`] so every validation rule in `saveRule` can be unit tested
/// without a `ClickHouse` connection.
#[derive(Debug)]
struct NormalizedRule {
    name: String,
    kind: AlertKind,
    channel: AlertChannel,
    target: String,
    mart: String,
    measure: String,
    agg: String,
    op: AlertOp,
    threshold: f64,
    board: String,
    severity: Option<String>,
}

/// Validate and normalize `input`, exactly reproducing `saveRule`'s
/// validation order in `alert-store.ts`. Pure — no `ClickHouse` access, so
/// this is fully unit-testable.
///
/// # Errors
///
/// Returns [`AlertError::Validation`] with the same Indonesian-language
/// message the TypeScript throws, at the same point in the check order.
fn normalize_input(input: &AlertRuleInput) -> Result<NormalizedRule, AlertError> {
    let name = input.name.as_deref().unwrap_or("").trim().to_owned();
    if name.is_empty() {
        return Err(AlertError::Validation("name is required.".to_owned()));
    }
    let kind = match input.kind.as_deref() {
        Some("digest") => AlertKind::Digest,
        Some("freshness") => AlertKind::Freshness,
        _ => AlertKind::Alert,
    };
    let channel = if input.channel.as_deref() == Some("email") {
        AlertChannel::Email
    } else {
        AlertChannel::Webhook
    };
    let target = input.target.as_deref().unwrap_or("").trim().to_owned();
    if target.is_empty() {
        return Err(AlertError::Validation(
            "target (webhook URL / email) is required.".to_owned(),
        ));
    }
    if channel == AlertChannel::Webhook {
        let lower = target.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(AlertError::Validation("invalid webhook URL.".to_owned()));
        }
    }
    if channel == AlertChannel::Email && !target.contains('@') {
        return Err(AlertError::Validation("invalid email.".to_owned()));
    }

    // WS5 plan review U12: validated once, at the source, against the same
    // five values `0038`'s `alert_instance.severity` `CHECK` admits —
    // applies uniformly to all three kinds (Alert/Digest/Freshness), not
    // special-cased per kind. A rule saved with an out-of-range severity
    // would insert cleanly into `ClickHouse` (no `CHECK` there) and then
    // silently fail every firing's `alert_instance` insert later, with no
    // way for the operator to learn their rule can never actually alert.
    let severity = match input.severity.as_deref() {
        None | Some("") => None,
        Some(s) if SEVERITIES.contains(&s) => Some(s.to_owned()),
        Some(_) => return Err(AlertError::Validation("invalid severity.".to_owned())),
    };

    let common = NormalizedCommon {
        name,
        kind,
        channel,
        target,
        severity,
    };
    if kind == AlertKind::Alert {
        normalize_alert(input, common)
    } else if kind == AlertKind::Freshness {
        normalize_freshness(input, common)
    } else {
        normalize_digest(input, common)
    }
}

/// The fields [`normalize_input`] validates uniformly, before branching on
/// `kind` — split out (alongside [`normalize_alert`]/[`normalize_freshness`]/
/// [`normalize_digest`]) purely to keep [`normalize_input`] itself under
/// `clippy::too_many_lines`; no behavior change from having it all inline.
struct NormalizedCommon {
    name: String,
    kind: AlertKind,
    channel: AlertChannel,
    target: String,
    severity: Option<String>,
}

fn normalize_alert(
    input: &AlertRuleInput,
    common: NormalizedCommon,
) -> Result<NormalizedRule, AlertError> {
    let raw_mart = input.mart.as_deref().unwrap_or("");
    let mart = raw_mart
        .strip_prefix("serving.")
        .unwrap_or(raw_mart)
        .to_owned();
    let measure = input.measure.as_deref().unwrap_or("").to_owned();
    let agg = input.agg.as_deref().unwrap_or("sum").to_ascii_lowercase();
    let op = input
        .op
        .as_deref()
        .and_then(AlertOp::parse_exact)
        .unwrap_or(AlertOp::Gt);
    let threshold = input.threshold.unwrap_or(0.0);
    if Ident::new(&mart).is_err() || Ident::new(&measure).is_err() {
        return Err(AlertError::Validation("invalid mart/measure.".to_owned()));
    }
    if !aggregate_allowed(&agg) {
        return Err(AlertError::Validation("invalid aggregate.".to_owned()));
    }
    if !threshold.is_finite() {
        return Err(AlertError::Validation("invalid threshold.".to_owned()));
    }
    Ok(NormalizedRule {
        name: common.name,
        kind: common.kind,
        channel: common.channel,
        target: common.target,
        mart,
        measure,
        agg,
        op,
        threshold,
        board: String::new(),
        severity: common.severity,
    })
}

/// A Freshness rule's `mart` field is reinterpreted as the
/// `dataset_sla.table_name` it targets — validated as
/// `<namespace>.<table>`, each half a real `Ident` (the same
/// lexical-safety guarantee `Alert`'s `mart`/`measure` already get), never
/// passed through to a query unchecked. This branch used to not exist:
/// before it, every Freshness rule fell into [`normalize_digest`]'s
/// (Digest-only) path and was rejected with "digest requires a board." —
/// or, if a caller supplied a `board` anyway to dodge that, had its `mart`
/// (the actual freshness target) silently blanked to `String::new()`.
///
/// The split-and-check itself lives in
/// [`lakehouse_core::ident::split_namespaced_table`], shared with
/// `lakehouse-api`'s `routes::governance::validate_namespaced_table`
/// (WS5 item C1a review finding: this used to be a second, independent
/// copy — see that helper's doc comment for why the shared copy had to
/// move down to `lakehouse-core`).
fn normalize_freshness(
    input: &AlertRuleInput,
    common: NormalizedCommon,
) -> Result<NormalizedRule, AlertError> {
    let raw_target = input.mart.as_deref().unwrap_or("").trim().to_owned();
    if let Err(err) = lakehouse_core::ident::split_namespaced_table(&raw_target) {
        return Err(AlertError::Validation(match err {
            lakehouse_core::ident::NamespacedTableError::MissingSeparator => {
                "freshness target table must be <namespace>.<table>.".to_owned()
            }
            lakehouse_core::ident::NamespacedTableError::Invalid(_) => {
                "invalid freshness target table.".to_owned()
            }
        }));
    }
    Ok(NormalizedRule {
        name: common.name,
        kind: common.kind,
        channel: common.channel,
        target: common.target,
        mart: raw_target,
        measure: String::new(),
        agg: "sum".to_owned(),
        op: AlertOp::Gt,
        threshold: 0.0,
        board: String::new(),
        severity: common.severity,
    })
}

/// Unchanged from today's behavior.
fn normalize_digest(
    input: &AlertRuleInput,
    common: NormalizedCommon,
) -> Result<NormalizedRule, AlertError> {
    let board = input.board.as_deref().unwrap_or("").to_owned();
    if board.is_empty() {
        return Err(AlertError::Validation(
            "digest requires a board.".to_owned(),
        ));
    }
    Ok(NormalizedRule {
        name: common.name,
        kind: common.kind,
        channel: common.channel,
        target: common.target,
        mart: String::new(),
        measure: String::new(),
        agg: "sum".to_owned(),
        op: AlertOp::Gt,
        threshold: 0.0,
        board,
        severity: common.severity,
    })
}

// ── id generation ──────────────────────────────────────────────────────
// Mirrors `randomUUID().slice(0, 8)` (first 8 hex chars of a v4 UUID's first
// group, which carries no version/variant bits and is therefore uniformly
// random).

fn random_hex8() -> String {
    use std::fmt::Write as _;

    use rand::Rng;
    let mut bytes = [0_u8; 4];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(8);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn new_rule_id() -> String {
    format!("al_{}", random_hex8())
}

// ── DDL bootstrap ──────────────────────────────────────────────────────

/// Create the `console` database and `alert_rule` table if they do not
/// already exist (idempotent). Ports `ensure` in `alert-store.ts`.
///
/// # Errors
///
/// Returns [`ChError`] if any DDL statement fails.
pub async fn ensure(ch: &ChClient) -> Result<(), ChError> {
    ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
        .await?;
    ch.exec(
        "CREATE TABLE IF NOT EXISTS console.alert_rule (\n\
           id String, name String, type String DEFAULT 'alert',\n\
           mart String DEFAULT '', measure String DEFAULT '', agg String DEFAULT 'sum',\n\
           op String DEFAULT '>', threshold Float64 DEFAULT 0,\n\
           board String DEFAULT '', channel String DEFAULT 'webhook', target String DEFAULT '',\n\
           enabled UInt8 DEFAULT 1,\n\
           created_at DateTime DEFAULT now(), updated_at DateTime DEFAULT now(), is_deleted UInt8 DEFAULT 0\n\
         ) ENGINE = ReplacingMergeTree(updated_at) ORDER BY id",
        None,
    )
    .await?;
    // WS5 item C1: additive, idempotent widening of a table that already
    // holds real data in production — the same convention every other
    // optional text column on this table follows (e.g. `mart String
    // DEFAULT ''` above); `non_empty()` already turns an empty string
    // back into `None` on read, so this needs no migration for existing
    // rows.
    ch.exec(
        "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS severity String DEFAULT ''",
        None,
    )
    .await?;
    Ok(())
}

const COLS: &str = "id,name,type,mart,measure,agg,op,threshold,board,channel,target,enabled,toString(created_at) AS created_at,severity";

fn row_str<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

/// Parse a `ClickHouse` numeric column that may come back as either a JSON
/// number or a JSON string (`ClickHouse`'s `FORMAT JSON` stringifies some
/// integer types but not `Float64`), defaulting to `0.0` when missing.
fn row_f64(row: &Map<String, Value>, key: &str) -> f64 {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn row_enabled(row: &Map<String, Value>) -> bool {
    match row.get("enabled") {
        Some(Value::String(s)) => s == "1",
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    }
}

/// Ports `toRule`: maps a raw `console.alert_rule` row to an [`AlertRule`],
/// applying the same permissive fallbacks (`r.agg || "sum"`, `OPS.includes`
/// on `op`, `r.channel === "email"`, `r.mart || undefined`, ...).
fn row_to_rule(row: &Map<String, Value>) -> AlertRule {
    let kind = match row_str(row, "type") {
        "digest" => AlertKind::Digest,
        "freshness" => AlertKind::Freshness,
        _ => AlertKind::Alert,
    };
    let channel = if row_str(row, "channel") == "email" {
        AlertChannel::Email
    } else {
        AlertChannel::Webhook
    };
    let op = AlertOp::parse_exact(row_str(row, "op")).unwrap_or(AlertOp::Gt);
    let agg = non_empty(row_str(row, "agg")).unwrap_or_else(default_agg);
    AlertRule {
        id: row_str(row, "id").to_owned(),
        name: row_str(row, "name").to_owned(),
        kind,
        mart: non_empty(row_str(row, "mart")),
        measure: non_empty(row_str(row, "measure")),
        agg,
        op,
        threshold: row_f64(row, "threshold"),
        board: non_empty(row_str(row, "board")),
        channel,
        target: row_str(row, "target").to_owned(),
        enabled: row_enabled(row),
        created_at: non_empty(row_str(row, "created_at")),
        severity: non_empty(row_str(row, "severity")),
    }
}

/// List every non-deleted rule, oldest first. Ports `listRules`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn list_rules(ch: &ChClient) -> Result<Vec<AlertRule>, ChError> {
    ensure(ch).await?;
    let rows = ch
        .rows(
            &format!(
                "SELECT {COLS} FROM console.alert_rule FINAL WHERE is_deleted = 0 ORDER BY created_at"
            ),
            None,
        )
        .await?;
    Ok(rows.iter().map(row_to_rule).collect())
}

/// Fetch a rule by id. Ports `getRule`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn get_rule(ch: &ChClient, id: &str) -> Result<Option<AlertRule>, ChError> {
    ensure(ch).await?;
    let sql = format!(
        "SELECT {COLS} FROM console.alert_rule FINAL WHERE is_deleted = 0 AND id={} LIMIT 1",
        SqlLiteral::from(id)
    );
    let rows = ch.rows(&sql, None).await?;
    Ok(rows.first().map(row_to_rule))
}

/// Validate and save (create/replace) a rule. `id` present means update.
/// Ports `saveRule`.
///
/// # Errors
///
/// Returns [`AlertError::Validation`] on invalid input (see
/// [`normalize_input`]), or [`AlertError::Clickhouse`] on a `ClickHouse`
/// failure.
pub async fn save_rule(
    ch: &ChClient,
    input: &AlertRuleInput,
    id: Option<&str>,
) -> Result<AlertRule, AlertError> {
    ensure(ch).await?;
    let normalized = normalize_input(input)?;
    let rid = id.map_or_else(new_rule_id, str::to_owned);
    let enabled: u8 = u8::from(input.enabled != Some(false));
    let threshold = normalized.threshold;
    // `SqlLiteral` has no `NULL` variant (it always quotes a string), so a
    // `None` severity binds ClickHouse's own empty-string default rather
    // than `NULL` — consistent with `mart`/`measure`/`board`'s existing
    // "empty string means absent" convention on this table (`non_empty()`
    // already turns `""` back into `None` on read, so the round-trip is
    // lossless).
    let sql = format!(
        "INSERT INTO console.alert_rule (id,name,type,mart,measure,agg,op,threshold,board,channel,target,enabled,severity) VALUES ({},{},{},{},{},{},{},{threshold},{},{},{},{enabled},{})",
        SqlLiteral::from(rid.as_str()),
        SqlLiteral::from(normalized.name.as_str()),
        SqlLiteral::from(normalized.kind.as_str()),
        SqlLiteral::from(normalized.mart.as_str()),
        SqlLiteral::from(normalized.measure.as_str()),
        SqlLiteral::from(normalized.agg.as_str()),
        SqlLiteral::from(normalized.op.as_str()),
        SqlLiteral::from(normalized.board.as_str()),
        SqlLiteral::from(normalized.channel.as_str()),
        SqlLiteral::from(normalized.target.as_str()),
        SqlLiteral::from(normalized.severity.as_deref().unwrap_or("")),
    );
    ch.exec(&sql, None).await?;
    match get_rule(ch, &rid).await? {
        Some(rule) => Ok(rule),
        // Unreachable in practice: `ReplacingMergeTree` + `FINAL` sees a
        // just-inserted row on the same connection immediately. The
        // TypeScript's `(await getRule(rid))!` would throw a raw
        // "Cannot read properties of null" TypeError here instead; this
        // typed error is the Rust-idiomatic equivalent of that same
        // "should never happen" branch.
        None => Err(AlertError::Validation(
            "rule was saved but could not be found.".to_owned(),
        )),
    }
}

/// Soft-delete a rule. Ports `deleteRule`.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub async fn delete_rule(ch: &ChClient, id: &str) -> Result<(), ChError> {
    ensure(ch).await?;
    let sql = format!(
        "INSERT INTO console.alert_rule (id,name,is_deleted) VALUES ({},'',1)",
        SqlLiteral::from(id)
    );
    ch.exec(&sql, None).await
}

// ── Evaluation ──────────────────────────────────────────────────────────

/// Whether `v` crosses `threshold` under `op`. Ports `compare`.
///
/// Uses plain `f64` comparison, which has the exact same semantics as
/// `TypeScript`'s `===`/`<`/`>`/etc. on `number` for every case that
/// matters here: `NaN` compares unequal (and false) to everything
/// including itself under every operator, matching `NaN === NaN` being
/// `false` in `JavaScript`.
#[must_use]
#[allow(
    clippy::float_cmp,
    reason = "intentionally reproducing TypeScript's `===` semantics for \
              AlertOp::Eq, including NaN never comparing equal to itself"
)]
fn compare(v: f64, op: AlertOp, threshold: f64) -> bool {
    match op {
        AlertOp::Gt => v > threshold,
        AlertOp::Ge => v >= threshold,
        AlertOp::Lt => v < threshold,
        AlertOp::Le => v <= threshold,
        AlertOp::Eq => v == threshold,
    }
}

/// `Number(x)`-style coercion for a `ClickHouse` row value, matching
/// `Number((r.data[0])?.v ?? 0)` in the TypeScript. Missing/`null` becomes
/// `0.0` (the `?? 0` fallback); a present-but-unparseable string becomes
/// `NaN` (matching `Number("not-a-number")`), NOT `0.0` — those are
/// different fallback rules in `JavaScript` and callers rely on the
/// distinction (a `NaN` current value never satisfies any [`compare`]
/// operator, so a bad value fails to fire rather than falsely firing at
/// the zero threshold).
fn coerce_number(v: Option<&Value>) -> f64 {
    match v {
        None | Some(Value::Null) => 0.0,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::Bool(b)) => f64::from(*b),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                0.0 // `Number("")` is `0` in JavaScript, not `NaN`.
            } else {
                trimmed.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Array(_) | Value::Object(_)) => f64::NAN,
    }
}

/// Current aggregate value for an alert rule's `mart`/`measure`/`agg`.
/// Ports `currentValue`.
///
/// `mart`/`measure` reach `ClickHouse` as raw, unquoted `SQL` identifiers
/// (unlike every other field in this module, which is sent as a
/// [`SqlLiteral`] value), so they are validated with [`Ident`] here — the
/// same check `saveRule` already ran before persisting them, re-run as
/// defence in depth against a row that predates this validation or was
/// written by another process.
async fn current_value(
    ch: &ChClient,
    mart: &str,
    measure: &str,
    agg: &str,
) -> Result<f64, AlertError> {
    let mart_ident =
        Ident::new(mart).map_err(|e| AlertError::Validation(format!("invalid mart: {e}")))?;
    if !aggregate_allowed(agg) {
        return Err(AlertError::Validation(format!("unknown aggregate: {agg}")));
    }
    let expr = if agg == "count" {
        "count()".to_owned()
    } else {
        let measure_ident = Ident::new(measure)
            .map_err(|e| AlertError::Validation(format!("invalid measure: {e}")))?;
        format!("round({agg}({measure_ident}))")
    };
    let sql = format!("SELECT {expr} AS v FROM serving.{mart_ident}");
    let rows = ch.rows(&sql, None).await?;
    Ok(coerce_number(rows.first().and_then(|r| r.get("v"))))
}

/// `Math.round(n).toLocaleString("id-ID")` — round to the nearest integer,
/// then group digits with `.` every three places (Indonesian locale
/// convention). Ports `fmt`.
///
/// Not a full `Intl.NumberFormat` port: it handles the finite, mostly-
/// positive aggregate values this module actually produces, and passes
/// `NaN`/`±Infinity` through as their `Rust` `Display` text rather than
/// throwing (`toLocaleString` on those `JavaScript` values would render
/// `"NaN"`/`"∞"`, which this only approximates).
fn fmt_id_id(n: f64) -> String {
    let rounded = n.round();
    let raw = format!("{rounded:.0}");
    let (negative, digits) = raw
        .strip_prefix('-')
        .map_or((false, raw.as_str()), |d| (true, d));
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return raw; // NaN / inf: pass through rather than mis-group.
    }
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (i, c) in digits.bytes().enumerate() {
        if i != 0 && (len - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c as char);
    }
    if negative {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// Digest text for a board's `KPI`/gauge tiles. Ports `digestText`.
async fn digest_text(ch: &ChClient, board_id: &str) -> Result<String, ChError> {
    let Some(board) = lakehouse_bi::store::get_board(ch, board_id).await? else {
        return Ok("Dashboard not found.".to_owned());
    };
    let charts = lakehouse_bi::store::list_stored_charts(ch).await?;
    let on_board: Vec<_> = charts.iter().filter(|c| c.board == board_id).collect();
    let mut lines = vec![format!(
        "Dashboard: {} — {} tile",
        board.name,
        on_board.len()
    )];
    for chart in on_board {
        let is_kpi_or_gauge = matches!(
            chart.spec.kind,
            lakehouse_bi::specs::ChartKind::Kpi | lakehouse_bi::specs::ChartKind::Gauge
        );
        if !is_kpi_or_gauge || chart.spec.sql.is_empty() {
            continue;
        }
        // `try { ... } catch { /* skip */ }` in the TypeScript: a failing
        // tile query is silently omitted from the digest, not propagated.
        if let Ok(rows) = ch.rows(&chart.spec.sql, None).await {
            let v = coerce_number(rows.first().and_then(|r| r.get("v")));
            lines.push(format!("• {}: {}", chart.spec.title, fmt_id_id(v)));
        }
    }
    Ok(lines.join("\n"))
}

/// Outcome of evaluating one rule. Ports `RunResult`.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// The rule's id.
    pub id: String,
    /// The rule's name.
    pub name: String,
    /// Whether it was an alert or a digest.
    #[serde(rename = "type")]
    pub kind: AlertKind,
    /// Whether it fired (crossed threshold, or — for a digest — was sent).
    pub fired: bool,
    /// The observed aggregate value, for alerts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// The delivery outcome, when a delivery was attempted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered: Option<DeliverResult>,
    /// Why this rule was skipped, if evaluating it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Source of dataset-freshness data for `AlertKind::Freshness` rules,
/// injected by the caller (`routes::alerts::run`) so this crate gains no
/// dependency on `lakehouse-store` (Postgres `dataset_sla`) or
/// `lakehouse-iceberg` (WS2's Lakekeeper REST `lastUpdatedMs`) — the same
/// low-level-crate-stays-low-level pattern `connector_probe::DynSecretResolver`
/// already uses. The real, Postgres/Iceberg-backed implementation
/// (`ApiFreshnessSource`) is a later commit; every call site here passes
/// `None` until then, which [`run_freshness`] reports as an honest
/// `skipped`, never a silent pass.
#[async_trait::async_trait]
pub trait FreshnessSource: Send + Sync {
    /// `dataset_sla.expected_interval_minutes` for `table_name`, or
    /// `Ok(None)` when no `dataset_sla` row exists for it — a config gap
    /// (nobody set an SLA for this table), not a data gap.
    ///
    /// # Errors
    ///
    /// A caller-defined error message (e.g. a database failure) — never an
    /// upstream detail; implementations must classify before returning.
    async fn expected_interval_minutes(&self, table_name: &str) -> Result<Option<i64>, String>;

    /// Milliseconds since epoch of `table_name`'s current Iceberg
    /// snapshot, or `None` when it cannot be determined right now
    /// (catalog unreachable, table not found, no snapshot yet) — a data
    /// gap, handled identically to a missing `dataset_sla` row: skip,
    /// never fire or clear.
    async fn last_snapshot_ms(&self, table_name: &str) -> Option<i64>;
}

/// Whether a rule is currently silenced — injected by the caller
/// (`routes::alerts::run`) exactly like [`FreshnessSource`], so this crate
/// still has no Postgres dependency. `true` suppresses delivery for a
/// fired rule; the rule is still evaluated (`RunResult::fired`/`value`
/// stay honest — a silence hides the noise, not the fact), only the
/// [`deliver`] call is skipped. WS5 plan review U6.
#[async_trait::async_trait]
pub trait SilenceSource: Send + Sync {
    /// Whether `rule_id` is currently silenced.
    async fn is_silenced(&self, rule_id: &str) -> bool;
}

/// Evaluate every enabled rule (or just `only`, if given), delivering
/// alerts/digests/freshness breaches that fire. Ports `runRules`.
///
/// # Warning
///
/// This queries live `ClickHouse` marts and, when a rule fires, sends a
/// real webhook `POST` or a real `SMTP` email — there is no dry-run mode,
/// matching the TypeScript. Callers (and tests) must not invoke this
/// against live infrastructure without intending exactly that.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g.
/// `ClickHouse` unreachable) — the only un-caught `await` in the
/// TypeScript's `runRules`. Every per-rule failure inside the loop is
/// caught and reported via [`RunResult::skipped`] instead, never
/// propagated.
pub async fn run_rules(
    ch: &ChClient,
    http: &reqwest::Client,
    email: &EmailSender,
    only: Option<&str>,
    freshness: Option<&dyn FreshnessSource>,
    silence: Option<&dyn SilenceSource>,
) -> Result<Vec<RunResult>, ChError> {
    let rules: Vec<AlertRule> = list_rules(ch)
        .await?
        .into_iter()
        .filter(|r| r.enabled && only.is_none_or(|id| id == r.id))
        .collect();

    let mut out = Vec::with_capacity(rules.len());
    for rule in &rules {
        out.push(run_one(ch, http, email, freshness, silence, rule).await);
    }
    Ok(out)
}

async fn run_one(
    ch: &ChClient,
    http: &reqwest::Client,
    email: &EmailSender,
    freshness: Option<&dyn FreshnessSource>,
    silence: Option<&dyn SilenceSource>,
    rule: &AlertRule,
) -> RunResult {
    match rule.kind {
        AlertKind::Alert => run_alert(ch, http, email, silence, rule).await,
        AlertKind::Digest => run_digest(ch, http, email, silence, rule).await,
        AlertKind::Freshness => {
            run_freshness(freshness, http, email, silence, rule, now_millis()).await
        }
    }
}

/// Current wall-clock time in milliseconds since the Unix epoch, used only
/// as [`run_freshness`]'s `now_ms` argument from `run_one`. Kept to this
/// single call site — never inside `run_freshness` itself — so
/// `run_freshness` stays a pure function of its `now_ms` parameter and
/// testable with a fixed clock (this plan's own testing rule: never read
/// the real clock inside the function under test).
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Deliver a fired rule's webhook/email, unless a [`SilenceSource`] reports
/// this rule is currently silenced. Shared by `run_alert`/`run_digest`/
/// `run_freshness`'s otherwise-identical "check silence, then maybe
/// deliver" sequence, written once here rather than pasted three times
/// (AGENTS.md rule 4). A silence hides the noise, not the fact: each
/// caller still records `RunResult::fired`/`value` honestly — only this
/// delivery attempt is skipped. WS5 plan review U6.
async fn deliver_unless_silenced(
    http: &reqwest::Client,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    rule: &AlertRule,
    title: &str,
    text: &str,
) -> Option<DeliverResult> {
    let silenced = match silence {
        Some(source) => source.is_silenced(&rule.id).await,
        None => false,
    };
    if silenced {
        return None;
    }
    Some(
        deliver(
            http,
            email,
            rule.channel.as_str(),
            &rule.target,
            title,
            text,
        )
        .await,
    )
}

async fn run_alert(
    ch: &ChClient,
    http: &reqwest::Client,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    rule: &AlertRule,
) -> RunResult {
    let mart = rule.mart.as_deref().unwrap_or("");
    let measure = rule.measure.as_deref().unwrap_or("");
    let value = match current_value(ch, mart, measure, &rule.agg).await {
        Ok(v) => v,
        Err(err) => return skipped(rule, err.to_string()),
    };
    let fired = compare(value, rule.op, rule.threshold);
    if !fired {
        return RunResult {
            id: rule.id.clone(),
            name: rule.name.clone(),
            kind: rule.kind,
            fired: false,
            value: Some(value),
            delivered: None,
            skipped: None,
        };
    }
    let text = format!(
        "{}({measure}) on {mart} = {} {} {} (threshold breached)",
        rule.agg,
        fmt_id_id(value),
        rule.op.as_str(),
        fmt_id_id(rule.threshold),
    );
    let title = format!("⚠️ Alert: {}", rule.name);
    let delivered = deliver_unless_silenced(http, email, silence, rule, &title, &text).await;
    RunResult {
        id: rule.id.clone(),
        name: rule.name.clone(),
        kind: rule.kind,
        fired: true,
        value: Some(value),
        delivered,
        skipped: None,
    }
}

async fn run_digest(
    ch: &ChClient,
    http: &reqwest::Client,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    rule: &AlertRule,
) -> RunResult {
    let board = rule.board.as_deref().unwrap_or("");
    match digest_text(ch, board).await {
        Ok(text) => {
            let title = format!("📊 Digest: {}", rule.name);
            let delivered =
                deliver_unless_silenced(http, email, silence, rule, &title, &text).await;
            RunResult {
                id: rule.id.clone(),
                name: rule.name.clone(),
                kind: rule.kind,
                fired: true,
                value: None,
                delivered,
                skipped: None,
            }
        }
        Err(err) => skipped(rule, err.to_string()),
    }
}

/// Evaluate one `Freshness` rule: how many minutes has it been since
/// `rule.mart` (reinterpreted as the `dataset_sla.table_name` target, per
/// [`normalize_freshness`]) last got a new Iceberg snapshot, compared
/// against its `dataset_sla.expected_interval_minutes`. Three distinct
/// outcomes, never conflated:
///
/// - **late** (`fired: true`) — the snapshot is older than the expected
///   interval; a breach, delivered like any other fired rule (subject to
///   [`SilenceSource`] suppression).
/// - **on-time** (`fired: false`) — within the expected interval.
/// - **unmeasurable** (`skipped: Some(reason)`) — no `freshness` source
///   configured, no `dataset_sla` row for this table, or no snapshot time
///   available. This case must never fire and must never clear: an
///   inability to measure freshness is not evidence either way.
async fn run_freshness(
    freshness: Option<&dyn FreshnessSource>,
    http: &reqwest::Client,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    rule: &AlertRule,
    now_ms: i64,
) -> RunResult {
    let table_name = rule.mart.as_deref().unwrap_or("");
    let Some(freshness) = freshness else {
        return skipped(
            rule,
            "freshness source not configured for this call".to_owned(),
        );
    };
    let expected = match freshness.expected_interval_minutes(table_name).await {
        Ok(Some(minutes)) => minutes,
        Ok(None) => return skipped(rule, format!("no dataset_sla row for table {table_name:?}")),
        Err(err) => return skipped(rule, err),
    };
    let Some(last_snapshot_ms) = freshness.last_snapshot_ms(table_name).await else {
        return skipped(
            rule,
            format!("last snapshot time unavailable for table {table_name:?}"),
        );
    };
    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond ages and expected-interval minutes fit well within f64's exact-integer range for any realistic freshness window"
    )]
    let (age_minutes, expected_f64) = (
        (now_ms - last_snapshot_ms) as f64 / 60_000.0,
        expected as f64,
    );
    let late = age_minutes > expected_f64;
    if !late {
        return RunResult {
            id: rule.id.clone(),
            name: rule.name.clone(),
            kind: rule.kind,
            fired: false,
            value: Some(age_minutes),
            delivered: None,
            skipped: None,
        };
    }
    let text = format!(
        "{table_name} last updated {age_minutes:.1} min ago (expected within {expected} min, per dataset_sla)"
    );
    let title = format!("⏰ Freshness: {}", rule.name);
    let delivered = deliver_unless_silenced(http, email, silence, rule, &title, &text).await;
    RunResult {
        id: rule.id.clone(),
        name: rule.name.clone(),
        kind: rule.kind,
        fired: true,
        value: Some(age_minutes),
        delivered,
        skipped: None,
    }
}

fn skipped(rule: &AlertRule, reason: String) -> RunResult {
    RunResult {
        id: rule.id.clone(),
        name: rule.name.clone(),
        kind: rule.kind,
        fired: false,
        value: None,
        delivered: None,
        skipped: Some(reason),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    // ── compare / AlertOp ──────────────────────────────────────────────

    #[test]
    fn threshold_fires_when_value_crosses_above() {
        assert!(compare(105.0, AlertOp::Gt, 100.0));
        assert!(!compare(95.0, AlertOp::Gt, 100.0));
    }

    #[test]
    fn threshold_fires_when_value_crosses_below() {
        assert!(compare(5.0, AlertOp::Lt, 10.0));
        assert!(!compare(15.0, AlertOp::Lt, 10.0));
    }

    /// The TypeScript comparator (`v > t`, `v < t`, ...) uses *strict*
    /// inequality for `>`/`<` — equality never fires those operators, only
    /// `>=`/`<=`/`==` do. This pins that down for `>` specifically, since
    /// it's the default operator new rules get.
    #[test]
    fn threshold_does_not_fire_on_exact_equality() {
        assert!(!compare(100.0, AlertOp::Gt, 100.0));
        assert!(!compare(100.0, AlertOp::Lt, 100.0));
        assert!(compare(100.0, AlertOp::Ge, 100.0));
        assert!(compare(100.0, AlertOp::Le, 100.0));
        assert!(compare(100.0, AlertOp::Eq, 100.0));
    }

    #[test]
    fn eq_operator_never_fires_on_nan() {
        // `NaN === NaN` is `false` in JavaScript; `f64::NAN == f64::NAN` is
        // `false` in Rust too, so this needs no special-casing in
        // `compare`, but is pinned down explicitly since it's easy to
        // regress with a well-intentioned `is_nan` check.
        assert!(!compare(f64::NAN, AlertOp::Eq, f64::NAN));
        assert!(!compare(f64::NAN, AlertOp::Gt, 0.0));
        assert!(!compare(f64::NAN, AlertOp::Lt, 0.0));
    }

    #[test]
    fn alert_op_parse_exact_is_allowlist_only() {
        assert_eq!(AlertOp::parse_exact(">"), Some(AlertOp::Gt));
        assert_eq!(AlertOp::parse_exact(">="), Some(AlertOp::Ge));
        assert_eq!(AlertOp::parse_exact("<"), Some(AlertOp::Lt));
        assert_eq!(AlertOp::parse_exact("<="), Some(AlertOp::Le));
        assert_eq!(AlertOp::parse_exact("=="), Some(AlertOp::Eq));
        assert_eq!(AlertOp::parse_exact("!="), None);
        assert_eq!(AlertOp::parse_exact(""), None);
    }

    // ── coerce_number ───────────────────────────────────────────────────

    #[test]
    fn coerce_number_missing_or_null_is_zero() {
        assert_eq!(coerce_number(None), 0.0);
        assert_eq!(coerce_number(Some(&Value::Null)), 0.0);
    }

    #[test]
    fn coerce_number_empty_string_is_zero_not_nan() {
        assert_eq!(coerce_number(Some(&Value::String(String::new()))), 0.0);
    }

    #[test]
    fn coerce_number_unparseable_string_is_nan() {
        assert!(coerce_number(Some(&Value::String("not-a-number".to_owned()))).is_nan());
    }

    #[test]
    fn coerce_number_parses_numeric_string_and_number() {
        assert_eq!(coerce_number(Some(&Value::String("42".to_owned()))), 42.0);
        assert_eq!(coerce_number(Some(&serde_json::json!(42.5))), 42.5);
    }

    // ── fmt_id_id ───────────────────────────────────────────────────────

    #[test]
    fn fmt_id_id_groups_thousands_with_dots() {
        assert_eq!(fmt_id_id(1_234_567.0), "1.234.567");
        assert_eq!(fmt_id_id(999.0), "999");
        assert_eq!(fmt_id_id(1000.0), "1.000");
    }

    #[test]
    fn fmt_id_id_rounds_and_handles_negative() {
        assert_eq!(fmt_id_id(1234.6), "1.235");
        assert_eq!(fmt_id_id(-1234.0), "-1.234");
    }

    // ── normalize_input (saveRule validation) ──────────────────────────

    fn valid_alert_input() -> AlertRuleInput {
        AlertRuleInput {
            name: Some("Wisman spike".to_owned()),
            kind: Some("alert".to_owned()),
            mart: Some("mart_wisman".to_owned()),
            measure: Some("jumlah".to_owned()),
            agg: Some("sum".to_owned()),
            op: Some(">".to_owned()),
            threshold: Some(1000.0),
            channel: Some("webhook".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            enabled: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn normalize_rejects_blank_name() {
        let input = AlertRuleInput {
            name: Some("   ".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "name is required.");
    }

    #[test]
    fn normalize_rejects_blank_target() {
        let input = AlertRuleInput {
            target: Some(String::new()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "target (webhook URL / email) is required.");
    }

    #[test]
    fn normalize_rejects_bad_webhook_url() {
        let input = AlertRuleInput {
            target: Some("not-a-url".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid webhook URL.");
    }

    #[test]
    fn normalize_rejects_bad_email() {
        let input = AlertRuleInput {
            channel: Some("email".to_owned()),
            target: Some("not-an-email".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid email.");
    }

    #[test]
    fn normalize_accepts_valid_email() {
        let input = AlertRuleInput {
            channel: Some("email".to_owned()),
            target: Some("ops@example.com".to_owned()),
            ..valid_alert_input()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.channel, AlertChannel::Email);
    }

    #[test]
    fn normalize_strips_serving_prefix_from_mart() {
        let input = AlertRuleInput {
            mart: Some("serving.mart_wisman".to_owned()),
            ..valid_alert_input()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.mart, "mart_wisman");
    }

    #[test]
    fn normalize_rejects_invalid_mart_identifier() {
        let input = AlertRuleInput {
            mart: Some("mart; DROP TABLE x".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid mart/measure.");
    }

    #[test]
    fn normalize_rejects_invalid_measure_identifier() {
        let input = AlertRuleInput {
            measure: Some("jumlah'".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid mart/measure.");
    }

    #[test]
    fn normalize_rejects_unknown_aggregate() {
        let input = AlertRuleInput {
            agg: Some("median".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid aggregate.");
    }

    #[test]
    fn normalize_rejects_non_finite_threshold() {
        let input = AlertRuleInput {
            threshold: Some(f64::NAN),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid threshold.");
    }

    #[test]
    fn normalize_unknown_op_falls_back_to_gt() {
        let input = AlertRuleInput {
            op: Some("!=".to_owned()),
            ..valid_alert_input()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.op, AlertOp::Gt);
    }

    #[test]
    fn normalize_unknown_type_falls_back_to_alert() {
        let input = AlertRuleInput {
            kind: Some("weekly".to_owned()),
            ..valid_alert_input()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::Alert);
    }

    #[test]
    fn normalize_digest_requires_board() {
        let input = AlertRuleInput {
            kind: Some("digest".to_owned()),
            board: None,
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "digest requires a board.");
    }

    #[test]
    fn normalize_digest_accepts_board() {
        let input = AlertRuleInput {
            kind: Some("digest".to_owned()),
            board: Some("default".to_owned()),
            ..valid_alert_input()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::Digest);
        assert_eq!(normalized.board, "default");
    }

    // ── row_to_rule ─────────────────────────────────────────────────────

    #[test]
    fn row_to_rule_maps_empty_optional_columns_to_none() {
        let mut row = Map::new();
        row.insert("id".to_owned(), Value::String("al_1".to_owned()));
        row.insert("name".to_owned(), Value::String("x".to_owned()));
        row.insert("type".to_owned(), Value::String("alert".to_owned()));
        row.insert("mart".to_owned(), Value::String(String::new()));
        row.insert("measure".to_owned(), Value::String(String::new()));
        row.insert("agg".to_owned(), Value::String(String::new()));
        row.insert("op".to_owned(), Value::String("bogus".to_owned()));
        row.insert("threshold".to_owned(), Value::String("42".to_owned()));
        row.insert("board".to_owned(), Value::String(String::new()));
        row.insert("channel".to_owned(), Value::String("webhook".to_owned()));
        row.insert("target".to_owned(), Value::String("t".to_owned()));
        row.insert("enabled".to_owned(), Value::String("1".to_owned()));
        row.insert("created_at".to_owned(), Value::String(String::new()));

        let rule = row_to_rule(&row);
        assert_eq!(rule.mart, None);
        assert_eq!(rule.measure, None);
        assert_eq!(rule.board, None);
        assert_eq!(rule.created_at, None);
        assert_eq!(rule.agg, "sum");
        assert_eq!(rule.op, AlertOp::Gt);
        assert!((rule.threshold - 42.0).abs() < f64::EPSILON);
        assert!(rule.enabled);
    }

    // ── new_rule_id ─────────────────────────────────────────────────────

    #[test]
    fn new_rule_id_has_expected_shape() {
        let id = new_rule_id();
        assert!(id.starts_with("al_"));
        assert_eq!(id.len(), "al_".len() + 8);
        assert!(id["al_".len()..].bytes().all(|b| b.is_ascii_hexdigit()));
    }

    // ── ensure (WS5 item C1, Step 2) ────────────────────────────────────

    /// WS5 plan review U1: the real precedent for testing a `ChClient`-
    /// calling function without a live instance is
    /// `lakehouse-clickhouse/src/lib.rs`'s own test module — `wiremock`
    /// `MockServer` + `ChClient::new(server.uri(), ..)`, asserting on the
    /// request body wiremock received. No live `ClickHouse`, no
    /// `#[ignore]`.
    #[tokio::test]
    async fn ensure_sends_an_additive_severity_column_statement() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("ADD COLUMN IF NOT EXISTS severity"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        // Every other `ensure()` statement (CREATE DATABASE, CREATE
        // TABLE) also POSTs to the same mock — a second unconditional
        // mock answers those so `ensure` doesn't fail on an unmatched
        // request.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        ensure(&ch).await.unwrap();
        // wiremock's `.expect(1)` on the specific severity-column mock is
        // verified automatically when `server` drops at the end of scope
        // — if `ensure` never sends that statement, the test panics
        // there.
    }

    // ── severity validation (WS5 item C1, Step 3a) ──────────────────────

    #[test]
    fn normalize_rejects_unknown_severity() {
        let input = AlertRuleInput {
            severity: Some("urgent".to_owned()),
            ..valid_alert_input()
        };
        let err = normalize_input(&input).unwrap_err();
        assert_eq!(err.to_string(), "invalid severity.");
    }

    #[test]
    fn normalize_accepts_a_real_severity() {
        let input = AlertRuleInput {
            severity: Some("critical".to_owned()),
            ..valid_alert_input()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.severity, Some("critical".to_owned()));
    }

    #[test]
    fn normalize_no_severity_stays_none() {
        let normalized = normalize_input(&valid_alert_input()).unwrap();
        assert_eq!(normalized.severity, None);
    }

    // ── AlertKind::Freshness round-trip (WS5 item C1, Step 4) ───────────

    #[test]
    fn freshness_kind_as_str_round_trips() {
        assert_eq!(AlertKind::Freshness.as_str(), "freshness");
    }

    #[test]
    fn row_to_rule_maps_freshness_type_column() {
        let mut row = Map::new();
        row.insert("id".to_owned(), Value::String("al_1".to_owned()));
        row.insert("name".to_owned(), Value::String("x".to_owned()));
        row.insert("type".to_owned(), Value::String("freshness".to_owned()));
        row.insert("mart".to_owned(), Value::String("bronze.orders".to_owned()));
        row.insert("measure".to_owned(), Value::String(String::new()));
        row.insert("agg".to_owned(), Value::String(String::new()));
        row.insert("op".to_owned(), Value::String(String::new()));
        row.insert("threshold".to_owned(), Value::String("0".to_owned()));
        row.insert("board".to_owned(), Value::String(String::new()));
        row.insert("channel".to_owned(), Value::String("email".to_owned()));
        row.insert("target".to_owned(), Value::String("t".to_owned()));
        row.insert("enabled".to_owned(), Value::String("1".to_owned()));
        row.insert("created_at".to_owned(), Value::String(String::new()));
        row.insert("severity".to_owned(), Value::String("info".to_owned()));

        let rule = row_to_rule(&row);
        assert_eq!(rule.kind, AlertKind::Freshness);
        assert_eq!(rule.severity, Some("info".to_owned()));
    }

    // ── normalize_input Freshness branch (WS5 item C1, Step 4a/5a) ──────

    #[test]
    fn normalize_input_accepts_a_valid_freshness_target() {
        let input = AlertRuleInput {
            name: Some("Orders freshness".to_owned()),
            kind: Some("freshness".to_owned()),
            target: Some("ops@example.invalid".to_owned()),
            channel: Some("email".to_owned()),
            mart: Some("bronze.orders".to_owned()),
            ..AlertRuleInput::default()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::Freshness);
        assert_eq!(normalized.mart, "bronze.orders");
        // measure/agg/threshold/board are cleared for this kind — a
        // Freshness rule is target-only, nothing else on the row means
        // anything for it.
        assert!(normalized.measure.is_empty());
        assert!(normalized.board.is_empty());
    }

    #[test]
    fn normalize_input_rejects_a_freshness_rule_with_no_target_table() {
        let input = AlertRuleInput {
            name: Some("Orders freshness".to_owned()),
            kind: Some("freshness".to_owned()),
            target: Some("ops@example.invalid".to_owned()),
            channel: Some("email".to_owned()),
            mart: None,
            ..AlertRuleInput::default()
        };
        assert!(normalize_input(&input).is_err());
    }

    #[test]
    fn normalize_input_rejects_an_injection_shaped_freshness_target() {
        let input = AlertRuleInput {
            name: Some("Orders freshness".to_owned()),
            kind: Some("freshness".to_owned()),
            target: Some("ops@example.invalid".to_owned()),
            channel: Some("email".to_owned()),
            mart: Some("bronze.orders; DROP TABLE dataset_sla;--".to_owned()),
            ..AlertRuleInput::default()
        };
        assert!(normalize_input(&input).is_err());
    }

    // ── run_freshness (WS5 item C1, Step 6) ─────────────────────────────

    fn freshness_rule(table_name: &str) -> AlertRule {
        AlertRule {
            id: "al_1".to_owned(),
            name: "Orders freshness".to_owned(),
            kind: AlertKind::Freshness,
            mart: Some(table_name.to_owned()),
            measure: None,
            agg: default_agg(),
            op: AlertOp::default(),
            threshold: 0.0,
            board: None,
            channel: AlertChannel::Email,
            target: "ops@example.invalid".to_owned(),
            enabled: true,
            created_at: None,
            severity: None,
        }
    }

    /// SMTP unconfigured (`host: None`) — `deliver` returns a fast,
    /// network-free `DeliverResult::err` without an actual `SMTP` attempt,
    /// matching `EmailSender::send`'s own documented validation order.
    fn no_smtp_email_sender() -> EmailSender {
        EmailSender::new(lakehouse_notify::SmtpConfig {
            host: None,
            port: 587,
            secure: false,
            user: None,
            pass: String::new(),
            from: String::new(),
        })
    }

    struct FixedFreshness {
        expected: Option<i64>,
        last_snapshot_ms: Option<i64>,
    }

    #[async_trait::async_trait]
    impl FreshnessSource for FixedFreshness {
        async fn expected_interval_minutes(
            &self,
            _table_name: &str,
        ) -> Result<Option<i64>, String> {
            Ok(self.expected)
        }
        async fn last_snapshot_ms(&self, _table_name: &str) -> Option<i64> {
            self.last_snapshot_ms
        }
    }

    #[tokio::test]
    async fn freshness_fires_when_last_snapshot_exceeds_the_expected_interval() {
        let rule = freshness_rule("bronze.orders");
        let src = FixedFreshness {
            expected: Some(30),
            last_snapshot_ms: Some(0),
        };
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        // 60 min old, 30 min budget.
        let result = run_freshness(Some(&src), &http, &email, None, &rule, 60 * 60_000).await;
        assert!(result.fired);
        assert!(result.skipped.is_none());
    }

    #[tokio::test]
    async fn freshness_does_not_fire_within_the_expected_interval() {
        let rule = freshness_rule("bronze.orders");
        let src = FixedFreshness {
            expected: Some(30),
            last_snapshot_ms: Some(60 * 60_000 - 5 * 60_000),
        };
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        // 5 min old, 30 min budget.
        let result = run_freshness(Some(&src), &http, &email, None, &rule, 60 * 60_000).await;
        assert!(!result.fired);
        assert!(result.skipped.is_none());
        assert!(
            result.delivered.is_none(),
            "an on-time result never attempts delivery"
        );
    }

    #[tokio::test]
    async fn freshness_skips_rather_than_firing_or_clearing_on_a_missing_snapshot() {
        let rule = freshness_rule("bronze.orders");
        let src = FixedFreshness {
            expected: Some(30),
            last_snapshot_ms: None,
        };
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let result = run_freshness(Some(&src), &http, &email, None, &rule, 60 * 60_000).await;
        assert!(!result.fired, "an unmeasurable freshness must never fire");
        assert!(result.skipped.is_some());
    }

    #[tokio::test]
    async fn freshness_skips_rather_than_firing_or_clearing_on_a_missing_sla_row() {
        let rule = freshness_rule("bronze.orders");
        let src = FixedFreshness {
            expected: None,
            last_snapshot_ms: Some(0),
        };
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let result = run_freshness(Some(&src), &http, &email, None, &rule, 60 * 60_000).await;
        assert!(!result.fired, "a missing dataset_sla row must never fire");
        assert!(result.skipped.is_some());
    }

    #[tokio::test]
    async fn freshness_skips_when_no_source_is_configured() {
        let rule = freshness_rule("bronze.orders");
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let result = run_freshness(None, &http, &email, None, &rule, 60 * 60_000).await;
        assert!(!result.fired);
        assert_eq!(
            result.skipped.as_deref(),
            Some("freshness source not configured for this call")
        );
    }

    // ── SilenceSource suppresses delivery, not evaluation (WS5 plan review U6) ──

    struct AlwaysSilenced;

    #[async_trait::async_trait]
    impl SilenceSource for AlwaysSilenced {
        async fn is_silenced(&self, _rule_id: &str) -> bool {
            true
        }
    }

    struct NeverSilenced;

    #[async_trait::async_trait]
    impl SilenceSource for NeverSilenced {
        async fn is_silenced(&self, _rule_id: &str) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn a_silenced_freshness_breach_still_fires_but_is_not_delivered() {
        let rule = freshness_rule("bronze.orders");
        let src = FixedFreshness {
            expected: Some(30),
            last_snapshot_ms: Some(0),
        };
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let result = run_freshness(
            Some(&src),
            &http,
            &email,
            Some(&AlwaysSilenced),
            &rule,
            60 * 60_000,
        )
        .await;
        assert!(result.fired, "the rule is still genuinely over threshold");
        assert!(result.value.is_some(), "value stays honest under a silence");
        assert!(
            result.delivered.is_none(),
            "delivery must be suppressed during the silence"
        );
    }

    #[tokio::test]
    async fn an_unsilenced_freshness_breach_still_attempts_delivery() {
        let rule = freshness_rule("bronze.orders");
        let src = FixedFreshness {
            expected: Some(30),
            last_snapshot_ms: Some(0),
        };
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let result = run_freshness(
            Some(&src),
            &http,
            &email,
            Some(&NeverSilenced),
            &rule,
            60 * 60_000,
        )
        .await;
        assert!(result.fired);
        assert!(
            result.delivered.is_some(),
            "an unsilenced fired rule still attempts delivery (SMTP being unconfigured makes it fail fast, not skip)"
        );
    }

    #[tokio::test]
    async fn deliver_unless_silenced_skips_the_deliver_call_entirely_when_silenced() {
        let rule = freshness_rule("bronze.orders");
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let delivered =
            deliver_unless_silenced(&http, &email, Some(&AlwaysSilenced), &rule, "title", "text")
                .await;
        assert!(delivered.is_none());
    }

    #[tokio::test]
    async fn deliver_unless_silenced_with_no_source_configured_behaves_as_unsilenced() {
        let rule = freshness_rule("bronze.orders");
        let http = reqwest::Client::new();
        let email = no_smtp_email_sender();
        let delivered = deliver_unless_silenced(&http, &email, None, &rule, "title", "text").await;
        assert!(delivered.is_some());
    }
}
