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
use lakehouse_notify::{DeliverResult, EmailSender, WebhookSender, deliver};
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
    /// Fires when a pipeline run fails, scoped to one pipeline id or to
    /// `"*"` for every pipeline. Evaluated by `routes::pipelines::
    /// run_failed_event`, not by `run_rules`: the trigger is the
    /// orchestrator's `run_failure_sensor` posting to the API, not a
    /// periodic `run_rules` evaluation. WS6 plan §3 (1e).
    PipelineFailure,
    /// Fires when a successful pipeline run exceeded the pipeline's
    /// `pipeline_sla.max_duration_seconds` (plan 1f). Evaluated by
    /// [`evaluate_pipeline_slow`] from the `run-finished` event
    /// route, not by `run_rules`. No periodic trigger; one alert per
    /// over-duration run.
    PipelineSlow,
    /// Fires when the pipeline's `pipeline_sla.late_after_seconds`
    /// threshold is breached (no successful run within the window, or
    /// never a success at all). Evaluated by `run_rules` from the
    /// `/api/alerts/run` 15-minute pass, the same posture `freshness`
    /// rules already take. Plan 1f.
    PipelineLate,
    /// Fires when a successful run reported a row count below half the
    /// median of prior completed runs (with at least 5 prior samples;
    /// fewer samples and the alert is skipped, never silently `false`).
    /// Evaluated by [`evaluate_pipeline_volume_drop`] from the
    /// `run-finished` event route. Plan 1f.
    PipelineVolumeDrop,
    /// Fires when a pipeline is behind its target lag for 3 consecutive checks
    /// (decision 3). Evaluated by [`evaluate_pipeline_behind_target`] from the
    /// lag-check route.
    #[serde(rename = "pipeline_behind_target")]
    PipelineBehindTarget,
    /// Fires when a connector's run fails, scoped to one connector id or
    /// `"*"` for every connector. Evaluated by
    /// [`evaluate_connector_event`] from `routes::pipelines::run_failed_event`
    /// (`SRC-7`). The explicit `serde(rename)` gives the `snake_case` wire
    /// string the `rename_all = "lowercase"` above would not.
    #[serde(rename = "connector_failure")]
    ConnectorFailure,
    /// Fires once per episode, when a connector's failure streak becomes
    /// exactly `REPEATED_FAILURE_STREAK` (3, decision D2). Same scope and
    /// caller as [`AlertKind::ConnectorFailure`] (`SRC-7`).
    #[serde(rename = "connector_repeated_failure")]
    ConnectorRepeatedFailure,
    /// Fires when a connector is disabled. Nothing raises it in `SRC-7`
    /// (decision D5): `SRC-11`'s auto-disable calls
    /// [`evaluate_connector_event`] with this kind.
    #[serde(rename = "connector_disabled")]
    ConnectorDisabled,
    /// Fires when a connector's source schema changes. Raised by `SRC-8`'s
    /// schema change detection (`routes::schema_changes`, which calls
    /// [`evaluate_connector_event`] with this kind) when a source table's
    /// columns change or a new table appears. `SRC-7` built the kind
    /// (decision D5) before anything raised it; `SRC-17` D-c corrects this
    /// doc, which kept saying nothing raises it.
    #[serde(rename = "connector_schema_change")]
    ConnectorSchemaChange,
    /// Fires when a connector's run succeeds. Optional per connector
    /// (decision D4): the rule must name one connector, never `"*"`, so
    /// with no such rule no success alert is ever sent.
    #[serde(rename = "connector_success")]
    ConnectorSuccess,
    /// Fires when a file upload's load fails. Always stored with the
    /// connector scope `"*"`: an upload belongs to no connector (`SRC-7`).
    #[serde(rename = "upload_failure")]
    UploadFailure,
    /// Fires when a quality incident OPENS (`DATA-14`): a failed check, or a
    /// learned volume or freshness anomaly. Scoped to one table
    /// (`silver.orders`) or `"*"` by the `asset` column. Evaluated by
    /// [`deliver_quality_incident_to`] from the route that opens the incident,
    /// never by `run_rules`; an incident seen again does not deliver again.
    #[serde(rename = "quality_incident")]
    QualityIncident,
}

impl AlertKind {
    /// Whether the kind is scoped by the `connector` column (`SRC-7`): the
    /// five `connector_*` kinds and `upload_failure`.
    #[must_use]
    pub const fn is_connector_scoped(self) -> bool {
        matches!(
            self,
            Self::ConnectorFailure
                | Self::ConnectorRepeatedFailure
                | Self::ConnectorDisabled
                | Self::ConnectorSchemaChange
                | Self::ConnectorSuccess
                | Self::UploadFailure
        )
    }

    /// Whether the kind is scoped by the `asset` column (`DATA-14`): only
    /// `quality_incident`.
    #[must_use]
    pub const fn is_asset_scoped(self) -> bool {
        matches!(self, Self::QualityIncident)
    }

    /// Parse a literal kind string for the `DATA-14` kind only; `None` for
    /// any other string.
    #[must_use]
    pub fn parse_asset_kind(s: &str) -> Option<Self> {
        (s == "quality_incident").then_some(Self::QualityIncident)
    }

    /// Parse a literal kind string (the inverse of [`AlertKind::as_str`])
    /// for the `SRC-7` kinds only; `None` for any other string.
    #[must_use]
    pub fn parse_connector_kind(s: &str) -> Option<Self> {
        match s {
            "connector_failure" => Some(Self::ConnectorFailure),
            "connector_repeated_failure" => Some(Self::ConnectorRepeatedFailure),
            "connector_disabled" => Some(Self::ConnectorDisabled),
            "connector_schema_change" => Some(Self::ConnectorSchemaChange),
            "connector_success" => Some(Self::ConnectorSuccess),
            "upload_failure" => Some(Self::UploadFailure),
            _ => None,
        }
    }

    /// The literal kind string, as stored in `console.alert_rule.type`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Digest => "digest",
            Self::Freshness => "freshness",
            Self::PipelineFailure => "pipeline_failure",
            Self::PipelineSlow => "pipeline_slow",
            Self::PipelineLate => "pipeline_late",
            Self::PipelineVolumeDrop => "pipeline_volume_drop",
            Self::PipelineBehindTarget => "pipeline_behind_target",
            Self::ConnectorFailure => "connector_failure",
            Self::ConnectorRepeatedFailure => "connector_repeated_failure",
            Self::ConnectorDisabled => "connector_disabled",
            Self::ConnectorSchemaChange => "connector_schema_change",
            Self::ConnectorSuccess => "connector_success",
            Self::UploadFailure => "upload_failure",
            Self::QualityIncident => "quality_incident",
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
    /// Pipeline id this rule fires on (`"pl-..."`) for
    /// [`AlertKind::PipelineFailure`] rules, or `"*"` to fire on every
    /// pipeline. `None` for any other kind: a threshold alert or a
    /// digest has no notion of "this pipeline", and the `pipeline` column
    /// stores the empty string then (matches the same `mart`/`board`/
    /// `severity` convention on this table — `non_empty()` turns `""`
    /// back into `None` on read).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pipeline: Option<String>,
    /// `BI-2`: the name of the saved metric a threshold alert watches, in
    /// place of `mart`, `measure` and `agg`. The metric is read through the
    /// same builder path as a KPI tile, with the fixed viewer role every rule
    /// is evaluated with (a rule has no owner), not with its author's access.
    /// `None` for a rule that watches a column.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metric: Option<String>,
    /// Connector id (or `"*"`) for the `connector_*` kinds, and `"*"` for
    /// `upload_failure` (`SRC-7`). `None` for every other kind; the column
    /// stores the empty string then, like `pipeline`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub connector: Option<String>,
    /// Table (`silver.orders`) or `"*"` for `quality_incident` rules
    /// (`DATA-14`). `None` for every other kind; the column stores the empty
    /// string then, like `pipeline` and `connector`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub asset: Option<String>,
    /// `BI-19` part C (D12): the id of the chart a chart alert watches. `None`
    /// for a rule that watches a mart column or a metric; the column stores the
    /// empty string then, like `pipeline`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chart: Option<String>,
    /// The series (a measure's column in the chart's result) a line, bar or
    /// combo chart alert reads the latest point of. `None` for a KPI or gauge
    /// chart, which has one number, and for a "returns any rows" rule.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub series: Option<String>,
    /// The dashboard filters the chart is read with (the same shape as the
    /// dashboard's `filters`), empty for none. Stored as JSON text in
    /// `filters_json`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub filters: Vec<lakehouse_bi::filters::FilterDef>,
    /// `above`, `below` or `any_rows` for a chart rule; `None` for a legacy
    /// rule, which keeps `op` (D11).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub condition: Option<String>,
    /// The person the rule reads its data as (C4): their roles, masking and row
    /// filters apply, and their access to the chart decides whether it still
    /// runs. `None` for a rule saved before part C, which reads as the fixed
    /// viewer role it always did.
    #[serde(rename = "ownerId", skip_serializing_if = "Option::is_none", default)]
    pub owner_id: Option<String>,
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
    /// Pipeline id (or `"*"`) this rule targets, for
    /// [`AlertKind::PipelineFailure`] rules. Required for that kind, not
    /// read for any other.
    pub pipeline: Option<String>,
    /// `BI-2`: the metric a threshold alert watches, by name. When present
    /// (and not blank) `mart`, `measure` and `agg` are not read.
    pub metric: Option<String>,
    /// Connector id (or `"*"`) for the `connector_*` kinds (`SRC-7`).
    /// Required for them, ignored for `upload_failure` (always stored as
    /// `"*"`) and for every other kind.
    pub connector: Option<String>,
    /// Table (`zone.table` in `bronze`, `silver` or `serving`) or `"*"` for
    /// a `quality_incident` rule (`DATA-14`); not read for any other kind.
    pub asset: Option<String>,
    /// `BI-19` part C: the chart a chart alert watches (D12).
    pub chart: Option<String>,
    /// The series of a line, bar or combo chart the alert reads (D12).
    pub series: Option<String>,
    /// The dashboard filters the chart is read with (D12).
    pub filters: Option<Vec<lakehouse_bi::filters::FilterDef>>,
    /// `above`, `below` or `any_rows` (D11, D12).
    pub condition: Option<String>,
    /// Who the rule reads its data as. Never taken from a request body: the
    /// route sets it from the signed-in person (`serde(skip)`), so a caller
    /// cannot save a rule that reads as somebody with wider access.
    #[serde(skip)]
    pub owner_id: Option<String>,
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
    /// Pipeline id (or `"*"`) for [`AlertKind::PipelineFailure`]; empty
    /// string for every other kind (matches the `mart`/`board`/
    /// `severity` "empty means absent" convention on this table).
    pipeline: String,
    /// `BI-2`: the watched metric's name; empty for any rule that watches a column.
    metric: String,
    /// Connector id or `"*"` for the `SRC-7` kinds; empty for every other
    /// kind.
    connector: String,
    /// Table or `"*"` for `quality_incident`; empty for every other kind.
    asset: String,
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
        Some("pipeline_failure") => AlertKind::PipelineFailure,
        Some("pipeline_slow") => AlertKind::PipelineSlow,
        Some("pipeline_late") => AlertKind::PipelineLate,
        Some("pipeline_volume_drop") => AlertKind::PipelineVolumeDrop,
        Some("pipeline_behind_target") => AlertKind::PipelineBehindTarget,
        Some(other) => AlertKind::parse_connector_kind(other)
            .or_else(|| AlertKind::parse_asset_kind(other))
            .unwrap_or(AlertKind::Alert),
        None => AlertKind::Alert,
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
    } else if kind == AlertKind::PipelineFailure {
        normalize_pipeline_failure(input, common)
    } else if kind == AlertKind::PipelineSlow {
        normalize_pipeline_scoped(input, common, "pipeline_slow")
    } else if kind == AlertKind::PipelineLate {
        normalize_pipeline_scoped(input, common, "pipeline_late")
    } else if kind == AlertKind::PipelineVolumeDrop {
        normalize_pipeline_scoped(input, common, "pipeline_volume_drop")
    } else if kind == AlertKind::PipelineBehindTarget {
        normalize_pipeline_scoped(input, common, "pipeline_behind_target")
    } else if kind.is_connector_scoped() {
        normalize_connector_scoped(input, common)
    } else if kind.is_asset_scoped() {
        normalize_asset_scoped(input, common)
    } else {
        normalize_digest(input, common)
    }
}

/// The `BI-19` part C columns of a rule (D11, D12), kept apart from
/// [`NormalizedRule`] because they do not depend on the rule's kind: any
/// threshold rule may carry an owner, and only a chart rule the rest. Empty
/// strings mean absent, like every other optional column of this table.
#[derive(Debug, Default)]
struct RuleExtras {
    chart: String,
    series: String,
    filters_json: String,
    condition: String,
    owner_id: String,
}

impl RuleExtras {
    /// `input` has passed [`normalize_input`], so a chart rule's fields are
    /// already valid; nothing but the owner is kept for any other rule.
    fn of(input: &AlertRuleInput, kind: AlertKind) -> Self {
        let owner_id = input.owner_id.clone().unwrap_or_default();
        let chart = input.chart.as_deref().map_or("", str::trim);
        if kind != AlertKind::Alert || chart.is_empty() {
            return Self {
                owner_id,
                ..Self::default()
            };
        }
        let condition = input.condition.as_deref().map_or("", str::trim);
        let filters = lakehouse_bi::filters::without_inert(input.filters.as_deref().unwrap_or(&[]));
        Self {
            chart: chart.to_owned(),
            series: if condition == "any_rows" {
                String::new()
            } else {
                input.series.as_deref().map_or("", str::trim).to_owned()
            },
            filters_json: if filters.is_empty() {
                String::new()
            } else {
                serde_json::to_string(&filters).unwrap_or_default()
            },
            condition: condition.to_owned(),
            owner_id,
        }
    }
}

/// The fields [`normalize_input`] validates uniformly, before branching on
/// `kind` — split out (alongside [`normalize_alert`]/[`normalize_freshness`]/
/// [`normalize_digest`]/[`normalize_pipeline_failure`]) purely to keep
/// [`normalize_input`] itself under `clippy::too_many_lines`; no behavior
/// change from having it all inline.
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
    // BI-19 part C (D12): a rule that watches a chart reads neither a mart nor a
    // metric of its own; the chart says what is read.
    let chart = input.chart.as_deref().map_or("", str::trim);
    if !chart.is_empty() {
        return normalize_chart(input, common, chart);
    }
    // BI-2: a rule that watches a metric reads no mart, measure or aggregate.
    let metric = input.metric.as_deref().map_or("", str::trim);
    if !metric.is_empty() {
        let threshold = input.threshold.unwrap_or(0.0);
        if Ident::new(metric).is_err() {
            return Err(AlertError::Validation("invalid metric.".to_owned()));
        }
        if !threshold.is_finite() {
            return Err(AlertError::Validation("invalid threshold.".to_owned()));
        }
        return Ok(NormalizedRule {
            name: common.name,
            kind: common.kind,
            channel: common.channel,
            target: common.target,
            mart: String::new(),
            measure: String::new(),
            agg: default_agg(),
            op: input
                .op
                .as_deref()
                .and_then(AlertOp::parse_exact)
                .unwrap_or(AlertOp::Gt),
            threshold,
            board: String::new(),
            severity: common.severity,
            metric: metric.to_owned(),
            pipeline: String::new(),
            connector: String::new(),
            asset: String::new(),
        });
    }
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
        metric: String::new(),
        pipeline: String::new(),
        connector: String::new(),
        asset: String::new(),
    })
}

/// The conditions of a chart alert (D12): over a line, under it, or "returns any
/// rows".
const CONDITIONS: [&str; 3] = ["above", "below", "any_rows"];

/// Validate a chart alert (`BI-19` part C, D12). Pure; whether the chart exists
/// and has a number is checked by [`save_rule`], which can read it.
fn normalize_chart(
    input: &AlertRuleInput,
    common: NormalizedCommon,
    chart: &str,
) -> Result<NormalizedRule, AlertError> {
    let bad = |m: &str| Err(AlertError::Validation(m.to_owned()));
    if chart.len() > 128
        || !chart
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return bad("invalid chart.");
    }
    let condition = input.condition.as_deref().map_or("", str::trim);
    if !CONDITIONS.contains(&condition) {
        return bad("condition must be above, below or any_rows.");
    }
    let threshold = if condition == "any_rows" {
        0.0
    } else {
        let t = input.threshold.unwrap_or(0.0);
        if !t.is_finite() {
            return bad("invalid threshold.");
        }
        t
    };
    if let Some(series) = input.series.as_deref()
        && (series.len() > 128 || series.chars().any(char::is_control))
    {
        return bad("invalid series.");
    }
    if let Some(filters) = input.filters.as_deref() {
        lakehouse_bi::filters::validate_filters(filters).map_err(AlertError::Validation)?;
    }
    Ok(NormalizedRule {
        name: common.name,
        kind: common.kind,
        channel: common.channel,
        target: common.target,
        mart: String::new(),
        measure: String::new(),
        agg: default_agg(),
        op: if condition == "below" {
            AlertOp::Lt
        } else {
            AlertOp::Gt
        },
        threshold,
        board: String::new(),
        severity: common.severity,
        metric: String::new(),
        pipeline: String::new(),
        connector: String::new(),
        asset: String::new(),
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
        metric: String::new(),
        pipeline: String::new(),
        connector: String::new(),
        asset: String::new(),
    })
}

/// `pipeline_failure` rules require a non-empty `pipeline` — either a
/// `pl-...` id (scoped to one pipeline) or the literal `"*"` (every
/// pipeline). The id is stored verbatim in `console.alert_rule.pipeline`
/// and matched in `evaluate_pipeline_failure` against the id the
/// orchestrator's run-failure sensor reports. Empty / whitespace-only
/// values are rejected — a "fire on every pipeline failure" rule is a
/// real configuration choice (`*`), not a forgotten field.
fn normalize_pipeline_failure(
    input: &AlertRuleInput,
    common: NormalizedCommon,
) -> Result<NormalizedRule, AlertError> {
    let raw = validate_pipeline_scoped(input, "pipeline_failure")?;
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
        board: String::new(),
        severity: common.severity,
        metric: String::new(),
        pipeline: raw,
        connector: String::new(),
        asset: String::new(),
    })
}

/// Plan 1f: `pipeline_slow`, `pipeline_late`, and `pipeline_volume_drop`
/// rules share the same pipeline-id contract as `pipeline_failure` —
/// either a `pl-...` id (scoped to one pipeline) or `"*"` (every
/// pipeline) — so they share this validator.
fn normalize_pipeline_scoped(
    input: &AlertRuleInput,
    common: NormalizedCommon,
    kind_label: &str,
) -> Result<NormalizedRule, AlertError> {
    let raw = validate_pipeline_scoped(input, kind_label)?;
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
        board: String::new(),
        severity: common.severity,
        metric: String::new(),
        pipeline: raw,
        connector: String::new(),
        asset: String::new(),
    })
}

fn validate_pipeline_scoped(
    input: &AlertRuleInput,
    kind_label: &str,
) -> Result<String, AlertError> {
    let raw = input.pipeline.as_deref().unwrap_or("").trim().to_owned();
    if raw.is_empty() {
        return Err(AlertError::Validation(format!(
            "{kind_label} requires a pipeline id or \"*\"."
        )));
    }
    if raw != "*" && !raw.starts_with("pl-") {
        return Err(AlertError::Validation(format!(
            "{kind_label} pipeline must be a pl-... id or \"*\"."
        )));
    }
    Ok(raw)
}

/// The most characters a connector id may have in a rule (`SRC-7`). The
/// store mints ids as `conn-<slug>-<base36>` (at most about 50 characters);
/// the bound only stops an absurd value from reaching `ClickHouse`.
const MAX_CONNECTOR_ID_LEN: usize = 128;

/// Shape check for a connector id in a rule's scope (`SRC-7`): ASCII letters,
/// digits and `-`, bounded length. The store has no validator of its own
/// (`connectors::slug_id` only ever mints `[a-z0-9-]`), so this is the one
/// place the shape is written down; whether the connector exists, and
/// whether the caller may see it, is the API's check (`routes::alerts`).
fn is_connector_id_shape(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_CONNECTOR_ID_LEN
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Validate the scope of the `SRC-7` kinds. `upload_failure` takes no input
/// and stores `"*"`; the five `connector_*` kinds need a connector id or
/// `"*"`, and `connector_success` refuses `"*"` (decision D4). Messages are
/// fixed text, never an echo of the input.
fn validate_connector_scoped(
    input: &AlertRuleInput,
    kind: AlertKind,
) -> Result<String, AlertError> {
    if kind == AlertKind::UploadFailure {
        return Ok("*".to_owned());
    }
    let label = kind.as_str();
    let raw = input.connector.as_deref().unwrap_or("").trim().to_owned();
    if raw.is_empty() {
        return Err(AlertError::Validation(format!(
            "{label} requires a connector id or \"*\"."
        )));
    }
    if raw == "*" {
        if kind == AlertKind::ConnectorSuccess {
            return Err(AlertError::Validation(
                "connector_success must name one connector, not \"*\".".to_owned(),
            ));
        }
        return Ok(raw);
    }
    if !is_connector_id_shape(&raw) {
        return Err(AlertError::Validation(format!(
            "{label} connector is not a valid connector id."
        )));
    }
    Ok(raw)
}

/// `SRC-7`: the `connector_*` kinds and `upload_failure`. No mart, measure,
/// board or pipeline: only the connector scope.
fn normalize_connector_scoped(
    input: &AlertRuleInput,
    common: NormalizedCommon,
) -> Result<NormalizedRule, AlertError> {
    let connector = validate_connector_scoped(input, common.kind)?;
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
        board: String::new(),
        severity: common.severity,
        metric: String::new(),
        pipeline: String::new(),
        connector,
        asset: String::new(),
    })
}

/// The zones a `quality_incident` rule may name a table in (the zones a
/// quality rule can check).
const QUALITY_ZONES: [&str; 3] = ["bronze", "silver", "serving"];

/// Validate the scope of a `quality_incident` rule (`DATA-14`): `"*"` for
/// every table, or `zone.table` with a real zone and a plain table name,
/// stored lower-cased because incidents are keyed that way. Messages are
/// fixed text, never an echo of the input.
fn validate_asset_scoped(input: &AlertRuleInput) -> Result<String, AlertError> {
    let raw = input.asset.as_deref().unwrap_or("").trim().to_lowercase();
    if raw.is_empty() {
        return Err(AlertError::Validation(
            "quality_incident requires a table or \"*\".".to_owned(),
        ));
    }
    if raw == "*" {
        return Ok(raw);
    }
    let valid = raw.split_once('.').is_some_and(|(zone, table)| {
        QUALITY_ZONES.contains(&zone) && lakehouse_core::ident::Ident::new(table).is_ok()
    });
    if valid {
        Ok(raw)
    } else {
        Err(AlertError::Validation(
            "quality_incident table must be bronze.<table>, silver.<table> or serving.<table>, or \"*\"."
                .to_owned(),
        ))
    }
}

/// `DATA-14`: `quality_incident`. No mart, measure, board, pipeline or
/// connector: only the table scope.
fn normalize_asset_scoped(
    input: &AlertRuleInput,
    common: NormalizedCommon,
) -> Result<NormalizedRule, AlertError> {
    let asset = validate_asset_scoped(input)?;
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
        board: String::new(),
        severity: common.severity,
        metric: String::new(),
        pipeline: String::new(),
        connector: String::new(),
        asset,
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
        metric: String::new(),
        pipeline: String::new(),
        connector: String::new(),
        asset: String::new(),
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
    // Plan 1e: the `pipeline` column carries a `pl-...` id (scoped) or
    // "*" (every pipeline) for `pipeline_failure` rules. Same
    // additive/`IF NOT EXISTS` convention as `severity` immediately
    // above, asserted by the `ensure_sends_an_additive_pipeline_column_statement`
    // test below.
    ch.exec(
        "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS pipeline String DEFAULT ''",
        None,
    )
    .await?;
    // BI-2: the metric a threshold alert watches; same additive convention.
    ch.exec(
        "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS metric String DEFAULT ''",
        None,
    )
    .await?;
    // `SRC-7`: the `connector` column carries a connector id or "*" for the
    // `connector_*` kinds and "*" for `upload_failure`. Additive, like
    // `pipeline` above; existing rows read back as `None`.
    ch.exec(
        "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS connector String DEFAULT ''",
        None,
    )
    .await?;
    // `DATA-14`: the `asset` column carries a table or "*" for
    // `quality_incident` rules. Additive, like `connector` above; existing
    // rows read back as `None`.
    ch.exec(
        "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS asset String DEFAULT ''",
        None,
    )
    .await?;
    // `BI-19` part C (D11, D12): the chart an alert watches and how, and the
    // person it reads as. All additive and `IF NOT EXISTS`, like `asset` above;
    // a rule saved before them reads back with every one absent, which is a
    // legacy rule (it keeps `op`, and reads as the fixed viewer role).
    for column in ["chart", "series", "filters_json", "condition", "owner_id"] {
        ch.exec(
            &format!(
                "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS {column} String DEFAULT ''"
            ),
            None,
        )
        .await?;
    }
    Ok(())
}

const COLS: &str = "id,name,type,mart,measure,agg,op,threshold,board,channel,target,enabled,toString(created_at) AS created_at,severity,pipeline,metric,connector,asset,chart,series,filters_json,condition,owner_id";

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
        "pipeline_failure" => AlertKind::PipelineFailure,
        "pipeline_slow" => AlertKind::PipelineSlow,
        "pipeline_late" => AlertKind::PipelineLate,
        "pipeline_volume_drop" => AlertKind::PipelineVolumeDrop,
        "pipeline_behind_target" => AlertKind::PipelineBehindTarget,
        other => AlertKind::parse_connector_kind(other)
            .or_else(|| AlertKind::parse_asset_kind(other))
            .unwrap_or(AlertKind::Alert),
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
        pipeline: non_empty(row_str(row, "pipeline")),
        metric: non_empty(row_str(row, "metric")),
        connector: non_empty(row_str(row, "connector")),
        asset: non_empty(row_str(row, "asset")),
        chart: non_empty(row_str(row, "chart")),
        series: non_empty(row_str(row, "series")),
        // A stored text that does not parse reads as no filters: the rule then
        // watches the chart unfiltered, which `save_rule` never writes (it
        // serialises a list it has parsed), so only a hand edit gets here.
        filters: serde_json::from_str(row_str(row, "filters_json")).unwrap_or_default(),
        condition: non_empty(row_str(row, "condition")),
        owner_id: non_empty(row_str(row, "owner_id")),
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
/// [`normalize_input`]) or a webhook target `webhooks` refuses (`SEC-10`),
/// or [`AlertError::Clickhouse`] on a `ClickHouse` failure.
pub async fn save_rule(
    ch: &ChClient,
    webhooks: &WebhookSender,
    input: &AlertRuleInput,
    id: Option<&str>,
) -> Result<AlertRule, AlertError> {
    ensure(ch).await?;
    let normalized = normalize_input(input)?;
    // SEC-10: refuse a webhook that points at an address webhooks may not
    // reach when the rule is SAVED, with the sender's fixed message, so the
    // author learns at once instead of from a failed delivery. A name can
    // resolve differently later, so the sender checks again at every send;
    // this check is the early warning, that one is the guarantee.
    if normalized.channel == AlertChannel::Webhook {
        webhooks
            .check(&normalized.target)
            .await
            .map_err(|err| AlertError::Validation(err.to_string()))?;
    }
    // BI-2: a rule can only watch a metric that exists.
    if !normalized.metric.is_empty() {
        let known = lakehouse_bi::metrics::list_metrics(ch)
            .await?
            .iter()
            .any(|m| m.name == normalized.metric);
        if !known {
            return Err(AlertError::Validation(
                "no metric has that name.".to_owned(),
            ));
        }
    }
    let extras = RuleExtras::of(input, normalized.kind);
    if !extras.chart.is_empty() {
        check_chart_has_a_number(ch, &extras).await?;
    }
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
        "INSERT INTO console.alert_rule (id,name,type,mart,measure,agg,op,threshold,board,channel,target,enabled,severity,pipeline,metric,connector,asset,chart,series,filters_json,condition,owner_id) VALUES ({},{},{},{},{},{},{},{threshold},{},{},{},{enabled},{},{},{},{},{},{},{},{},{},{})",
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
        SqlLiteral::from(normalized.pipeline.as_str()),
        SqlLiteral::from(normalized.metric.as_str()),
        SqlLiteral::from(normalized.connector.as_str()),
        SqlLiteral::from(normalized.asset.as_str()),
        SqlLiteral::from(extras.chart.as_str()),
        SqlLiteral::from(extras.series.as_str()),
        SqlLiteral::from(extras.filters_json.as_str()),
        SqlLiteral::from(extras.condition.as_str()),
        SqlLiteral::from(extras.owner_id.as_str()),
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

// ── Chart alerts (BI-19 part C, D12) ────────────────────────────────────

/// What number a chart gives an alert (feature C1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberShape {
    /// A KPI or a gauge: its one value.
    Single,
    /// A line, area, bar or combo chart over its dimension: the latest point of
    /// the chosen series.
    Series,
    /// Any other chart: no single number, so only "returns any rows".
    NoNumber,
}

/// The [`NumberShape`] of a chart kind.
#[must_use]
pub const fn number_shape(kind: lakehouse_bi::specs::ChartKind) -> NumberShape {
    use lakehouse_bi::specs::ChartKind;
    match kind {
        ChartKind::Kpi | ChartKind::Gauge => NumberShape::Single,
        ChartKind::Line | ChartKind::Area | ChartKind::Bar | ChartKind::Combo => {
            NumberShape::Series
        }
        _ => NumberShape::NoNumber,
    }
}

/// The number a chart's result gives (C1): a [`NumberShape::Single`] chart's
/// first row, a [`NumberShape::Series`] chart's last row in the `series` column
/// (the result is ordered by the dimension, which `save_rule` checks).
///
/// # Errors
///
/// A fixed sentence when the result has no row, no such column, or a value that
/// is not a number; the check is then recorded as failed, never read as zero.
pub fn chart_number(
    shape: NumberShape,
    series: Option<&str>,
    columns: &[String],
    rows: &[Value],
) -> Result<f64, String> {
    let (row, column) = match shape {
        NumberShape::Single => (
            rows.first(),
            series
                .filter(|s| !s.is_empty() && columns.iter().any(|c| c == s))
                .or_else(|| columns.first().map(String::as_str)),
        ),
        NumberShape::Series => (
            rows.last(),
            series.filter(|s| columns.iter().any(|c| c == s)),
        ),
        NumberShape::NoNumber => return Err("the chart has no single number.".to_owned()),
    };
    let Some(column) = column else {
        return Err("the series is not in the chart's result.".to_owned());
    };
    let Some(cell) = row.and_then(|r| r.get(column)) else {
        return Err("the chart returned no value.".to_owned());
    };
    let value = coerce_number(Some(cell));
    if value.is_finite() && !cell.is_null() {
        Ok(value)
    } else {
        Err("the chart's value is not a number.".to_owned())
    }
}

/// Save-time check that a chart alert watches a chart that exists and has what
/// the condition needs (D12): "above" and "below" need a number (a KPI, a gauge,
/// or a series of a chart ordered by its dimension); any other chart allows only
/// "`any_rows`". Read as the server (`Viewer::system`): whether the caller may see
/// the chart is the route's check, not this crate's.
async fn check_chart_has_a_number(ch: &ChClient, extras: &RuleExtras) -> Result<(), AlertError> {
    let charts =
        lakehouse_bi::store::list_stored_charts(ch, &lakehouse_bi::access::Viewer::system())
            .await?;
    let Some(chart) = charts.iter().find(|c| c.spec.id == extras.chart) else {
        return Err(AlertError::Validation("no chart has that id.".to_owned()));
    };
    if extras.condition == "any_rows" {
        return Ok(());
    }
    let refuse = |m: &str| Err(AlertError::Validation(m.to_owned()));
    match number_shape(chart.def.kind) {
        NumberShape::Series if extras.series.is_empty() => {
            refuse("choose the series whose latest point the alert reads.")
        }
        NumberShape::Series if chart.def.effective_order() != "none" => refuse(
            "this chart is sorted by value, so it has no latest point; use 'returns any rows'.",
        ),
        NumberShape::Single | NumberShape::Series => Ok(()),
        NumberShape::NoNumber => refuse(
            "this chart has no single number; an alert on it can only be 'returns any rows'.",
        ),
    }
}

/// The reason a chart rule is skipped by [`run_rules`].
const CHART_SKIPPED: &str = "chart alerts are evaluated by /api/alerts/run after run_rules returns, \
     because they are read as the person who set them";

/// A chart rule that cannot run: record `Stopped` with `reason` (no message is
/// sent) and report it as skipped. For an owner who is disabled or can no longer
/// see the chart (C4).
pub async fn stop(state: Option<&dyn StateSource>, rule: &AlertRule, reason: &str) -> RunResult {
    if let Some(state) = state {
        state
            .record(&rule.id, AlertState::Stopped, None, Some(reason))
            .await;
    }
    skipped(rule, reason.to_owned())
}

// ── Alerts that fire once (BI-19 part C, D11) ───────────────────────────

/// Where a threshold rule stands between two checks (D11, feature C2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertState {
    /// Not over the line, or never checked: a rule without a stored state
    /// reads as this, so the first check over the line sends once.
    Ok,
    /// Over the line, and the crossing was sent.
    Firing,
    /// The rule's owner can no longer see what it watches (or is disabled), so
    /// the rule does not run; the stored reason says why. A rule that is
    /// checked again is judged afresh, as `Ok` is.
    Stopped,
}

impl AlertState {
    /// The string stored in `alert_rule_state.state`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Firing => "firing",
            Self::Stopped => "stopped",
        }
    }

    /// Parse a stored state; anything but the three known words reads as
    /// [`AlertState::Ok`] (the state a rule without a row has), never as a
    /// reason to stay silent.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "firing" => Self::Firing,
            "stopped" => Self::Stopped,
            _ => Self::Ok,
        }
    }
}

/// What a check must tell the person, given the state before it and whether the
/// value is over the line now. Called `Send` in the plan; renamed because the
/// name would shadow the `Send` marker trait in this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sending {
    /// The value crossed the line: one message.
    Crossed,
    /// The value is back: one message.
    Recovered,
    /// Nothing to say (still over, or still under).
    Nothing,
}

/// The one rule of "fires once" (D11): the state after a check and what that
/// check sends. Pure, so the whole table is a unit test.
///
/// A check that errors does not call this: it records itself and changes
/// nothing (feature C2).
///
/// | before            | over the line | after    | sends       |
/// | ----------------- | ------------- | -------- | ----------- |
/// | `Ok` / `Stopped`  | yes           | `Firing` | `Crossed`   |
/// | `Ok` / `Stopped`  | no            | `Ok`     | `Nothing`   |
/// | `Firing`          | yes           | `Firing` | `Nothing`   |
/// | `Firing`          | no            | `Ok`     | `Recovered` |
#[must_use]
pub const fn transition(previous: AlertState, crossed: bool) -> (AlertState, Sending) {
    match (previous, crossed) {
        (AlertState::Ok | AlertState::Stopped, true) => (AlertState::Firing, Sending::Crossed),
        (AlertState::Ok | AlertState::Stopped, false) => (AlertState::Ok, Sending::Nothing),
        (AlertState::Firing, true) => (AlertState::Firing, Sending::Nothing),
        (AlertState::Firing, false) => (AlertState::Ok, Sending::Recovered),
    }
}

/// Where the state of each rule is kept, injected by the caller like
/// [`SilenceSource`] so this crate stays free of Postgres. Required for a
/// threshold rule: without it a rule cannot tell a first crossing from a
/// repeat, and sending on every check is exactly what D11 ends, so a threshold
/// rule is skipped with a reason when there is none.
#[async_trait::async_trait]
pub trait StateSource: Send + Sync {
    /// The state of `rule_id`; [`AlertState::Ok`] when it has none.
    ///
    /// # Errors
    ///
    /// A fixed, non-leaking sentence when the state cannot be read; the rule is
    /// then skipped, not judged.
    async fn state_of(&self, rule_id: &str) -> Result<AlertState, String>;

    /// Record a check that ran: the state after it, the number read, and the
    /// reason when `state` is [`AlertState::Stopped`]. An implementation that
    /// cannot write logs it itself (the caller has already sent or not sent).
    async fn record(
        &self,
        rule_id: &str,
        state: AlertState,
        value: Option<f64>,
        reason: Option<&str>,
    );

    /// Record a check that could not run: the time and `reason` (a fixed
    /// sentence) move; the state and the last value do not.
    async fn record_failed(&self, rule_id: &str, reason: &str);

    /// Claim a transition before its message is sent (`BI-19` C review SF-1):
    /// move the rule from `previous` to `next` only if it is still in
    /// `previous`, and say whether this caller won. Two runners that read the
    /// same `previous` cannot both win, so only the winner sends.
    ///
    /// # Errors
    ///
    /// A fixed, non-leaking sentence when the state cannot be written; the rule
    /// is then skipped without sending, since an unclaimed send is a repeat.
    async fn claim(
        &self,
        rule_id: &str,
        previous: AlertState,
        next: AlertState,
        value: Option<f64>,
    ) -> Result<bool, String>;

    /// Undo a [`StateSource::claim`] whose message could not be delivered: put
    /// the rule back in `previous` (only if it is still in `claimed`), so the
    /// next check sends again (the accepted resend-after-failure behaviour).
    async fn release(&self, rule_id: &str, claimed: AlertState, previous: AlertState);
}

/// Whether `new` watches something other than `old` (`BI-19` C review SF-3): a
/// kind, a source, a line, a chart, a series, a filter, a condition or a
/// reading identity changed. Name, target, channel, severity and the enabled
/// flag are not part of what is watched, so editing them keeps the state (no
/// repeat of a crossing that was already sent).
#[must_use]
pub fn watches_differ(old: &AlertRule, new: &AlertRule) -> bool {
    old.kind != new.kind
        || old.mart != new.mart
        || old.measure != new.measure
        || old.agg != new.agg
        || old.op != new.op
        || old.threshold.to_bits() != new.threshold.to_bits()
        || old.board != new.board
        || old.metric != new.metric
        || old.pipeline != new.pipeline
        || old.connector != new.connector
        || old.asset != new.asset
        || old.chart != new.chart
        || old.series != new.series
        || old.filters != new.filters
        || old.condition != new.condition
        || old.owner_id != new.owner_id
}

/// The reason a rule is skipped when no [`StateSource`] was supplied.
const STATE_UNAVAILABLE: &str = "the alert's state could not be read, so the rule was not checked (it would send on every check).";

/// The reason reported when another runner claimed the same transition first
/// (`BI-19` C review SF-1): it sends, this one does not.
const CLAIMED_ELSEWHERE: &str = "another check of this alert is handling it.";

/// The reason stored when a crossing or a recovery could not be delivered: the
/// state does not advance, so the next check sends it again.
const NOT_DELIVERED_RETRY: &str =
    "the message could not be delivered; it is sent again at the next check.";

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
    gate: &dyn SqlGate,
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
    let sql = gate.gate(&sql).await.map_err(AlertError::Validation)?;
    let rows = ch.rows(&sql, None).await?;
    Ok(coerce_number(rows.first().and_then(|r| r.get("v"))))
}

/// `BI-2`: the current value of the metric `name`, read through the very
/// builder path a KPI tile on the metric uses (`lakehouse_bi::metrics::value_chart`)
/// and the same gate every rule's statement goes through (the fixed viewer
/// role: a rule has no owner, so a metric alert is evaluated with that role,
/// never with its author's access).
///
/// Every way this can fail is a fixed sentence and the rule is recorded as
/// skipped with it, so a rule whose metric is gone or cannot be evaluated is
/// neither fired nor silently treated as fine, and no engine text is kept.
async fn metric_value(ch: &ChClient, gate: &dyn SqlGate, name: &str) -> Result<f64, AlertError> {
    use lakehouse_bi::builder::ReadContext;
    use lakehouse_bi::fields::{FieldCatalog, SourceKind};
    use lakehouse_bi::metrics::{ValueQuery, value_chart};
    let fail = |reason: &str| AlertError::Validation(reason.to_owned());
    let Ok(all) = lakehouse_bi::metrics::list_metrics(ch).await else {
        return Err(fail("the metric could not be read."));
    };
    let Some(def) = all.into_iter().find(|m| m.name == name) else {
        return Err(fail("the metric no longer exists."));
    };
    let unavailable = || fail("the metric's source is not available.");
    let (cols, source) = match def.source_kind {
        SourceKind::Mart => {
            let Ok(cols) = lakehouse_bi::store::validated_mart_columns(ch, &def.source_id).await
            else {
                return Err(unavailable());
            };
            (cols, None)
        }
        SourceKind::SqlSource => {
            let Ok(Some(src)) = lakehouse_bi::sources::get_source(ch, &def.source_id).await else {
                return Err(unavailable());
            };
            (src.column_kinds(), Some(src))
        }
    };
    let Ok(fields) =
        lakehouse_bi::fields::list_catalog_fields_for(ch, def.source_kind, &def.source_id).await
    else {
        return Err(fail("the metric could not be read."));
    };
    let catalog = FieldCatalog::from_fields(fields);
    let time = lakehouse_bi::grain::TimeContext::default();
    let read = ReadContext {
        time: &time,
        grain: None,
        fields: &catalog,
    };
    let chart = value_chart(&ValueQuery {
        metric: &def.name,
        source_kind: def.source_kind,
        source_id: &def.source_id,
        group_by: None,
        grain: None,
        filters: Vec::new(),
    });
    if catalog.broken_reference(&chart.def, &cols, &time) {
        return Err(fail("the metric no longer checks out."));
    }
    let sql = if let Some(src) = &source {
        lakehouse_bi::builder::sql_for_sql_source_report(
            &chart,
            &src.sql,
            &src.column_kinds(),
            &[],
            &[],
            &read,
        )
        .map(|f| f.sql)
    } else {
        let marts = std::collections::HashMap::from([(def.source_id.clone(), cols)]);
        Some(lakehouse_bi::builder::sql_with_filters_report(&chart, &[], &[], &marts, &read).sql)
    };
    // A value chart has no stored statement: an empty one is "cannot be built".
    let Some(sql) = sql.filter(|s| !s.is_empty()) else {
        return Err(fail("the metric cannot be evaluated."));
    };
    let sql = gate.gate(&sql).await.map_err(AlertError::Validation)?;
    let Ok(rows) = ch.rows(&sql, None).await else {
        return Err(fail("the metric could not be read."));
    };
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
async fn digest_text(ch: &ChClient, gate: &dyn SqlGate, board_id: &str) -> Result<String, ChError> {
    // A digest goes to whoever the rule names (a mailbox, a webhook), not to a
    // signed-in viewer, so it is read as nobody in particular: a dashboard in
    // a personal space, or in the trash, is "not found" here, and its charts
    // are not listed (BI-18 part C, part 2).
    let viewer = lakehouse_bi::access::Viewer::anonymous();
    let Some(board) = lakehouse_bi::store::get_board(ch, board_id, &viewer).await? else {
        return Ok("Dashboard not found.".to_owned());
    };
    let charts = lakehouse_bi::store::list_stored_charts(ch, &viewer).await?;
    let on_board: Vec<_> = charts.iter().filter(|c| c.board == board_id).collect();
    // A tile on a dashboard SQL source is rebuilt from the source's current
    // text, as the dashboard itself does; the SQL stored with the chart may
    // be stale.
    let sources = if on_board.iter().any(|c| c.def.sql_source.is_some()) {
        lakehouse_bi::sources::list_sources(ch).await?
    } else {
        Vec::new()
    };
    // BI-8: a tile on a SQL source that names a calculated field is built
    // over it, so the fields are read when a source tile exists. A mart tile
    // uses the SQL stored with it, as before.
    let fields = if sources.is_empty() {
        lakehouse_bi::fields::FieldCatalog::default()
    } else {
        lakehouse_bi::fields::FieldCatalog::from_fields(
            lakehouse_bi::fields::list_catalog_fields(ch).await?,
        )
    };
    let time = lakehouse_bi::grain::TimeContext::default();
    let mut lines = vec![format!(
        "Dashboard: {} — {} tile",
        board.name,
        on_board.len()
    )];
    for chart in on_board {
        // BI-16 part B: a progress bar and a bullet graph are deliberately not
        // digest tiles. Their row is a value and a goal (`v`, `g`), and a digest
        // line is one number with its title; alerts are rules on metrics, not on
        // tiles, so no alert is offered on them either.
        let is_kpi_or_gauge = matches!(
            chart.spec.kind,
            lakehouse_bi::specs::ChartKind::Kpi | lakehouse_bi::specs::ChartKind::Gauge
        );
        if !is_kpi_or_gauge || chart.spec.sql.is_empty() {
            continue;
        }
        let sql = match chart.def.sql_source.as_deref() {
            None => Some(chart.spec.sql.clone()),
            Some(id) => sources.iter().find(|s| s.id == id).and_then(|s| {
                // BI-9: a digest tile is a KPI or gauge with no dashboard
                // filters, so no date expression, bucket or relative filter
                // is built here and the default time context is never read.
                lakehouse_bi::builder::sql_for_sql_source(
                    chart,
                    &s.sql,
                    &s.column_kinds(),
                    &[],
                    &[],
                    &lakehouse_bi::builder::ReadContext {
                        fields: &fields,
                        ..lakehouse_bi::builder::ReadContext::new(&time)
                    },
                )
            }),
        };
        // `try { ... } catch { /* skip */ }` in the TypeScript: a failing
        // tile query is silently omitted from the digest, not propagated.
        // A tile the gate refuses, or whose SQL source is gone, is omitted
        // the same way — never run ungoverned.
        let Some(sql) = sql else { continue };
        let Ok(sql) = gate.gate(&sql).await else {
            continue;
        };
        if let Ok(rows) = ch.rows(&sql, None).await {
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
    /// Whether it fired: a threshold rule crossed its line at this check (once;
    /// `BI-19` D11), or — for a digest — was sent.
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
    /// `BI-19` part C (D11): the value is back under the line and the recovery
    /// message was sent (or was silenced). `fired` is then `false`: it means a
    /// crossing was sent this check, not that the value is over the line.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub recovered: bool,
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

/// Plan 1f source for the "is this pipeline currently late" decision:
/// the `late_after_seconds` from `pipeline_sla` for `pipeline_id`, and
/// the Unix-epoch seconds of its most recent successful run (`None` when
/// it has never succeeded). Injected by the caller
/// (`routes::alerts::run`) so this crate stays free of Postgres: the
/// real implementation is `routes::alerts::ApiLateSource`.
///
/// Return shape: a `Result<Option<(i32, Option<f64>)>, String>` where:
/// * the inner `Option` is `Some` only when a SLA row exists for this
///   pipeline — a missing row is a config gap, NOT a "not late" claim;
/// * the `i32` is the SLA's `late_after_seconds` (already validated as
///   positive at the API layer);
/// * the `Option<f64>` is the last successful run's epoch seconds, or
///   `None` when no SUCCESS run is on record yet — the [`late`] helper
///   treats that as "definitely late, threshold set".
///
/// # Errors
///
/// A classified error string (e.g. a Postgres connection failure
/// already mapped to "database error") — never an upstream detail;
/// implementations classify before returning.
#[async_trait::async_trait]
pub trait LateSource: Send + Sync {
    /// `Some((threshold, last_success_seconds))` when a `pipeline_sla`
    /// row exists for `pipeline_id`; `None` when it does not (a config
    /// gap, not a "not late" claim).
    ///
    /// # Errors
    ///
    /// A classified error string (e.g. a Postgres connection failure
    /// already mapped to "database error") — never an upstream detail;
    /// implementations classify before returning.
    async fn late_inputs(&self, pipeline_id: &str) -> Result<Option<(i32, Option<f64>)>, String>;
}

/// Governance gate every statement this crate runs goes through, injected
/// by the caller like [`SilenceSource`] so this crate stays free of the
/// API's policy engine. Required, not optional: an alert value or a digest
/// leaves the console by webhook or email, so an ungoverned read would
/// deliver masked or row-filtered data to whoever the rule targets. The
/// digest used to run each tile's stored SQL with plain `ch.rows`
/// (`docs/plans/DASHBOARD-SQL-SOURCES-FOLDERS-PLAN.md` §8, item 4).
#[async_trait::async_trait]
pub trait SqlGate: Send + Sync {
    /// The statement to execute in place of `sql`.
    ///
    /// # Errors
    ///
    /// A fixed, non-leaking refusal message when `sql` may not run; the
    /// caller skips that value rather than running anything else.
    async fn gate(&self, sql: &str) -> Result<String, String>;

    /// Whether dashboard `board_id` is restricted ("only the people listed",
    /// itself or through a folder above it). A digest leaves the console to
    /// whoever the rule names, outside the dashboard's rules, so a restricted
    /// board's digest is skipped, not sent (`BI-19` B1 review R6, owner
    /// decision Q-R12). No default: an implementer must answer, and answer
    /// `true` when it cannot tell (fail closed).
    async fn board_restricted(&self, board_id: &str) -> bool;
}

/// Plan 1f pure helper: "is this pipeline currently late?" Lives here
/// (in the `lakehouse-alerts` crate) because both the routes API
/// (`routes::pipelines::dagster_pipeline_row`) and this crate's
/// [`evaluate_pipeline_late`] need the exact same logic. STRICT
/// inequality: `gap == late_after_seconds` is NOT late — same rule
/// the runs route enforces, mutation-tested by the
/// `late_is_false_when_gap_is_exactly_the_threshold` test below.
///
/// * `now_seconds = None` — returns `None`: without a clock, "late"
///   is not a claim this function can make.
/// * `late_after_seconds = None` — returns `None`: no threshold set,
///   so "late" is undefined for this pipeline.
/// * `last_success_seconds = None` AND a threshold IS set — returns
///   `Some(true)`: the pipeline has a threshold and has never had a
///   successful run, so it is by definition late.
#[must_use]
pub fn late(
    now_seconds: Option<f64>,
    last_success_seconds: Option<f64>,
    late_after_seconds: Option<i32>,
) -> Option<bool> {
    let late_after = late_after_seconds?;
    let now = now_seconds?;
    match last_success_seconds {
        None => Some(true),
        Some(last) => {
            // Strict `>`: gap == threshold is *not* late.
            let gap = now - last;
            Some(gap > f64::from(late_after))
        }
    }
}

/// Plan 1f entry point: deliver every enabled [`AlertKind::PipelineLate`]
/// rule whose `pipeline` matches `pipeline_id` (or `*`). Unlike
/// [`evaluate_pipeline_failure`], [`evaluate_pipeline_slow`], and
/// [`evaluate_pipeline_volume_drop`], this is called by
/// `/api/alerts/run` (the 15-minute pass) rather than an event route —
/// "late" is a clock-relative claim, and the clock
/// ticks on the schedule of the op, not the orchestrator.
///
/// The route handler provides a [`LateSource`] (a Postgres-backed
/// implementation) that returns `(late_after_seconds, last_success_seconds)`
/// for the pipeline id. With `None` returned by the source (no SLA row),
/// the matching rules are skipped with an honest `unsupported` reason —
/// a missing SLA is a config gap, not a "not late" claim. With
/// `Some(threshold, last)` and [`late`] returning `None` (the function
/// could not decide), the rules are also skipped.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g.
/// `ClickHouse` unreachable). A classified error from the [`LateSource`]
/// (e.g. a Postgres failure already mapped to "database error" by the
/// caller) is propagated as [`AlertError::Validation`], because the
/// message is already classified at the source and never reaches the
/// response body raw — the route handler renders it as a 500 with that
/// fixed message. Per-rule delivery failures are reported via
/// [`DeliverResult`] inside `deliver_unless_silenced`, never propagated.
pub async fn evaluate_pipeline_late(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    pipeline_id: &str,
    late_source: Option<&dyn LateSource>,
    silence: Option<&dyn SilenceSource>,
    now_seconds: Option<f64>,
) -> Result<usize, PipelineLateError> {
    let late_decision = late_decision(late_source, pipeline_id, now_seconds).await?;
    // `evaluate_pipeline_late`'s caller (the alerts-route `/api/alerts/run`
    // orchestration) needs the decision BEFORE delivery for the
    // `pipeline_run_event` dedupe (reviewer fix #2). When that
    // orchestration decides late==true and the dedupe row inserts
    // successfully, it calls `evaluate_pipeline_late` again to deliver;
    // redoing the decision here is wasted work but never wrong, and
    // keeping the single-call entry point preserves the existing
    // `evaluate_pipeline_late` test surface.
    let rules: Vec<AlertRule> = list_rules(ch)
        .await?
        .into_iter()
        .filter(|r| {
            r.enabled
                && r.kind == AlertKind::PipelineLate
                && r.pipeline
                    .as_deref()
                    .is_some_and(|p| p == pipeline_id || p == "*")
        })
        .collect();

    let mut delivered = 0_usize;
    for rule in &rules {
        let text = match late_decision {
            Some(true) => format!(
                "Pipeline {pipeline_id} has not had a successful run within its late \
                 threshold. View: /pipelines/{pipeline_id}"
            ),
            // `Some(false)` and `None` both skip: a missing SLA, or a
            // successful run within the threshold, is not a "late"
            // claim. Same `skipped` posture [`run_freshness`] takes for
            // a missing dataset SLA — honest, not silent.
            Some(false) | None => continue,
        };
        let title = format!("⏰ Pipeline late: {}", rule.name);
        if let Some(DeliverResult { .. }) =
            deliver_unless_silenced(http, email, silence, rule, &title, &text).await
        {
            delivered += 1;
        }
    }
    Ok(delivered)
}

/// Plan 1f (reviewer fix #2): compute the "is this pipeline currently
/// late?" decision without delivering anything. The route handler at
/// `/api/alerts/run` calls this to decide whether to write the
/// `pipeline_run_event` dedupe row; delivery (via
/// [`evaluate_pipeline_late`]) only happens when this returns
/// `Some(true)` AND the dedupe row was newly inserted.
///
/// Returns `Ok(None)` when no claim can be made: no source, source has
/// no SLA row, or `late()` itself cannot decide (no `now_seconds`).
/// Returns `Ok(Some(true))` / `Ok(Some(false))` for the genuine
/// decisions. Propagates a [`PipelineLateError::Source`] for any
/// classified error the [`LateSource`] implementation raises.
///
/// The crate has no `PgPool` by design (AGENTS.md rule 4: never leak
/// upstream text into responses; the route layer is the one place
/// that holds a pool and classifies messages), so this function and
/// [`evaluate_pipeline_late`] deliberately stay free of database
/// dependencies.
///
/// # Errors
///
/// Returns [`PipelineLateError::Source`] for any classified error the
/// [`LateSource`] implementation propagates (the route layer maps that
/// to a fixed "internal" message — never the upstream detail).
pub async fn late_decision(
    late_source: Option<&dyn LateSource>,
    pipeline_id: &str,
    now_seconds: Option<f64>,
) -> Result<Option<bool>, PipelineLateError> {
    // Resolve the "is this pipeline late?" inputs from the source.
    // `None` source means the route handler could not provide one (e.g.
    // Postgres unconfigured) — every `pipeline_late` rule is skipped
    // rather than silently dropped.
    let inputs = match late_source {
        None => return Ok(None),
        Some(src) => match src.late_inputs(pipeline_id).await {
            Ok(Some(inputs)) => Some(inputs),
            Ok(None) => None, // No SLA row — every rule is unsupported.
            Err(err) => {
                // Classified upstream error from the source — propagate
                // to the caller so it surfaces a 500 (not a false-skip).
                return Err(PipelineLateError::Source(err));
            }
        },
    };
    Ok(match inputs {
        Some((threshold, last_success)) => late(now_seconds, last_success, Some(threshold)),
        None => None,
    })
}

/// Errors [`evaluate_pipeline_late`] can return. Two flavours:
/// * `ListRules(ChError)` — `ClickHouse` unreachable while listing
///   alert rules (mirrors the sibling functions' `ChError` return);
/// * `Source(String)` — a classified error from the [`LateSource`]
///   implementation (the message is already classified by the source
///   before this crate sees it).
///
/// F2.7: `ListRules` previously used `#[error(transparent)]`, which
/// forwarded the upstream `ClickHouse` error body (`ChError::Server`)
/// into `routes::alerts`'s `format!("{err}")` — a leak of upstream text
/// into the 500 response. The route layer's `format!("{err}")` (lines
/// 757 / 793) now sees the FIXED display, never `ChError`'s Display —
/// the upstream detail remains available via `tracing::error!(source
/// = %err, ...)` (or by inspecting `err` at the route layer) and is
/// not echoed in the HTTP body.
#[derive(Debug, Error)]
pub enum PipelineLateError {
    /// `ClickHouse` unreachable while listing alert rules (mirrors the
    /// sibling functions' `ChError` return). F2.7: `Display` is a
    /// FIXED classified string — the upstream `ClickHouse` body is
    /// NOT echoed through `Display` here (AGENTS.md principle 4).
    #[error("database error")]
    ListRules(#[from] ChError),
    /// A classified error from the [`LateSource`] implementation — the
    /// message is already classified by the source before this crate
    /// sees it, so it is safe to surface through the route handler.
    #[error("{0}")]
    Source(String),
}

/// Plan 1f reviewer fix #1: deliver every enabled [`AlertKind::PipelineSlow`]
/// rule whose `pipeline` matches `pipeline_id`. The route handler at
/// `routes::pipelines::run_finished_event` is responsible for the
/// per-kind `pipeline_run_event` dedupe BEFORE this is invoked (same
/// pattern `run_failed_event` uses for kind `"failure"`); a `false`
/// `is_slow` skips the kind entirely. The crate has no `PgPool` by
/// design (AGENTS.md rule 4) — the route layer is the only place the
/// dedupe write happens.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g.
/// `ClickHouse` unreachable). Per-rule delivery failures are reported via
/// [`DeliverResult`] inside `deliver_unless_silenced`, never propagated.
pub async fn evaluate_pipeline_slow(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    pipeline_id: &str,
    run_id: &str,
    is_slow: bool,
    silence: Option<&dyn SilenceSource>,
) -> Result<usize, ChError> {
    if !is_slow {
        // Reviewer fix #1: the route already deduped each kind before
        // calling in — `is_slow == false` means "the rule kind did
        // not fire on this run" (no SLA, or the run finished in time).
        // Skip rather than re-list rules and discover the same answer.
        return Ok(0);
    }
    let console_link = format!("/pipelines/{pipeline_id}?run={run_id}");
    let text = format!(
        "Pipeline {pipeline_id} run {run_id} exceeded its duration SLA. View: {console_link}"
    );
    let title_prefix = "🐢 Pipeline slow";
    deliver_one_kind(
        ch,
        http,
        email,
        pipeline_id,
        silence,
        AlertKind::PipelineSlow,
        title_prefix,
        &text,
    )
    .await
}

/// Plan 1f reviewer fix #1: deliver every enabled
/// [`AlertKind::PipelineVolumeDrop`] rule whose `pipeline` matches
/// `pipeline_id`. Same posture as [`evaluate_pipeline_slow`]: the
/// route handler dedupes per kind before calling.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g.
/// `ClickHouse` unreachable). Per-rule delivery failures are reported via
/// [`DeliverResult`] inside `deliver_unless_silenced`, never propagated.
pub async fn evaluate_pipeline_volume_drop(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    pipeline_id: &str,
    run_id: &str,
    is_volume_drop: bool,
    silence: Option<&dyn SilenceSource>,
) -> Result<usize, ChError> {
    if !is_volume_drop {
        return Ok(0);
    }
    let console_link = format!("/pipelines/{pipeline_id}?run={run_id}");
    let text = format!(
        "Pipeline {pipeline_id} run {run_id} processed fewer than half the median \
         rows of its prior runs. View: {console_link}"
    );
    let title_prefix = "📉 Pipeline volume drop";
    deliver_one_kind(
        ch,
        http,
        email,
        pipeline_id,
        silence,
        AlertKind::PipelineVolumeDrop,
        title_prefix,
        &text,
    )
    .await
}

/// BLD-2 plan D9: deliver every enabled [`AlertKind::PipelineBehindTarget`]
/// rule whose `pipeline` matches `pipeline_id` (or `*`).
///
/// Evaluated in the lag-check handler when `behind_streak` becomes exactly 3
/// (`lakehouse_store::pipelines::BEHIND_STREAK_ALERT_AFTER`), the same "once per streak"
/// shape as `ConnectorRepeatedFailure`.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g. `ClickHouse` unreachable).
// BLD-2 review R3 BLOCKER 1 / decision R14: alert includes reason when provided
pub async fn evaluate_pipeline_behind_target(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    pipeline_id: &str,
    lag_seconds: Option<i64>,
    target_seconds: Option<i32>,
    reason: Option<&str>,
    silence: Option<&dyn SilenceSource>,
) -> Result<usize, ChError> {
    let console_link = format!("/pipelines/{pipeline_id}");
    let lag_str = lag_seconds.map_or_else(|| "unknown".to_owned(), |s| format!("{s}s"));
    let target_str = target_seconds.map_or_else(|| "unknown".to_owned(), |s| format!("{s}s"));
    let reason_clause = reason.map_or_else(String::new, |r| format!(", reason: {r}"));
    let text = format!(
        "Pipeline {pipeline_id} is behind its target lag (lag: {lag_str}, target: {target_str}{reason_clause}). View: {console_link}"
    );
    let title_prefix = "⚠️ Pipeline behind target";
    deliver_one_kind(
        ch,
        http,
        email,
        pipeline_id,
        silence,
        AlertKind::PipelineBehindTarget,
        title_prefix,
        &text,
    )
    .await
}

/// Shared helper for [`evaluate_pipeline_slow`] and
/// [`evaluate_pipeline_volume_drop`]: list the enabled rules for the
/// single `kind`, deliver `text` to each, and count the deliveries.
/// The pre-filter on `is_slow` / `is_volume_drop` happened in the
/// caller — this never sees a "this kind didn't fire" case.
async fn deliver_one_kind(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    pipeline_id: &str,
    silence: Option<&dyn SilenceSource>,
    kind: AlertKind,
    title_prefix: &str,
    text: &str,
) -> Result<usize, ChError> {
    deliver_matching(
        ch,
        http,
        email,
        silence,
        kind,
        RuleScope::Pipeline(pipeline_id),
        title_prefix,
        text,
    )
    .await
    .map(|delivered| delivered.len())
}

/// One rule a delivery was attempted for, with how it went (`SRC-17` F1,
/// delivery review 2026-10-12): the caller records the outcome on the alert
/// instance instead of treating "attempted" as "delivered".
///
/// `result.error` is one of the fixed [`lakehouse_notify::DeliveryFailure`]
/// sentences, never text from the webhook or the mail server.
#[derive(Debug, Clone)]
pub struct RuleDelivery {
    /// The rule the delivery was attempted for.
    pub rule: AlertRule,
    /// Whether it arrived, and if not, the fixed reason.
    pub result: DeliverResult,
}

/// Which `console.alert_rule` scope column a delivery matches against: the
/// `pipeline` column (the pipeline kinds) or the `connector` column
/// (`SRC-7`). In either, a stored `"*"` matches every id.
#[derive(Clone, Copy)]
enum RuleScope<'a> {
    Pipeline(&'a str),
    Connector(&'a str),
    /// The `asset` column (`DATA-14`): a table, lower-cased, or `"*"`.
    Asset(&'a str),
}

impl RuleScope<'_> {
    fn matches(self, rule: &AlertRule) -> bool {
        let (wanted, stored) = match self {
            Self::Pipeline(id) => (id, rule.pipeline.as_deref()),
            Self::Connector(id) => (id, rule.connector.as_deref()),
            Self::Asset(asset) => (asset, rule.asset.as_deref()),
        };
        stored.is_some_and(|s| s == wanted || s == "*")
    }
}

/// The shared filter-and-deliver of [`deliver_one_kind`] (`SRC-7`, AGENTS.md
/// rule 4: generalised rather than copied): list the enabled rules of
/// `kind` whose scope matches, deliver `text` to each unless silenced, and
/// return each rule a delivery was attempted for with its outcome (so the
/// caller can record an `alert_instance` for each, delivered or not;
/// `SRC-17` F1: the outcome used to be discarded here).
async fn deliver_matching(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    kind: AlertKind,
    scope: RuleScope<'_>,
    title_prefix: &str,
    text: &str,
) -> Result<Vec<RuleDelivery>, ChError> {
    let rules = list_rules(ch).await?;
    Ok(deliver_to_matching(
        &rules,
        http,
        email,
        silence,
        kind,
        scope,
        title_prefix,
        text,
    )
    .await)
}

/// The delivery half of [`deliver_matching`], over rules the caller already
/// read. Reading the rules and delivering are separate so a caller can read
/// them BEFORE it records anything (`SRC-17` F2, [`load_rules`]): this cannot
/// fail on the rule store.
async fn deliver_to_matching(
    rules: &[AlertRule],
    http: &WebhookSender,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    kind: AlertKind,
    scope: RuleScope<'_>,
    title_prefix: &str,
    text: &str,
) -> Vec<RuleDelivery> {
    let mut attempted = Vec::new();
    for rule in rules
        .iter()
        .filter(|r| r.enabled && r.kind == kind && scope.matches(r))
    {
        let title = format!("{title_prefix}: {}", rule.name);
        if let Some(result) =
            deliver_unless_silenced(http, email, silence, rule, &title, text).await
        {
            attempted.push(RuleDelivery {
                rule: rule.clone(),
                result,
            });
        }
    }
    attempted
}

/// Read the alert rules once, for [`deliver_connector_event_to`]
/// (`SRC-17` F2, delivery review 2026-10-12).
///
/// A run report reads the rules BEFORE it writes its dedupe row and counts
/// the run, then delivers from this list: the rule store can no longer fail
/// after the run has been recorded, so a retry of a report that failed here
/// finds nothing recorded and does everything, and a retry of one that
/// succeeded finds the dedupe row and does nothing.
///
/// # Errors
///
/// Returns [`ChError`] only if the rule store cannot be read.
pub async fn load_rules(ch: &ChClient) -> Result<Vec<AlertRule>, ChError> {
    list_rules(ch).await
}

/// [`evaluate_connector_event`] over rules already read with [`load_rules`]:
/// it cannot fail, and returns each rule a delivery was attempted for with
/// its outcome (`SRC-17` F1, F2). Non-`SRC-7` kinds deliver nothing.
pub async fn deliver_connector_event_to(
    rules: &[AlertRule],
    http: &WebhookSender,
    email: &EmailSender,
    kind: AlertKind,
    connector_id: &str,
    title_prefix: &str,
    text: &str,
    silence: Option<&dyn SilenceSource>,
) -> Vec<RuleDelivery> {
    if !kind.is_connector_scoped() {
        return Vec::new();
    }
    deliver_to_matching(
        rules,
        http,
        email,
        silence,
        kind,
        RuleScope::Connector(connector_id),
        title_prefix,
        text,
    )
    .await
}

/// Deliver an opened quality incident (`DATA-14`) to every enabled
/// `quality_incident` rule whose table is `asset` (compared lower-cased) or
/// `"*"`, over rules already read with [`load_rules`] — it cannot fail on the
/// rule store. Returns each rule a delivery was attempted for with its
/// outcome. `text` holds only the table, the rule's name and what was
/// measured, built by the caller from the incident, never upstream error
/// text.
pub async fn deliver_quality_incident_to(
    rules: &[AlertRule],
    http: &WebhookSender,
    email: &EmailSender,
    asset: &str,
    title_prefix: &str,
    text: &str,
    silence: Option<&dyn SilenceSource>,
) -> Vec<RuleDelivery> {
    let asset = asset.trim().to_lowercase();
    deliver_to_matching(
        rules,
        http,
        email,
        silence,
        AlertKind::QualityIncident,
        RuleScope::Asset(&asset),
        title_prefix,
        text,
    )
    .await
}

/// Deliver every enabled rule of the `SRC-7` `kind` whose connector scope
/// matches `connector_id` (or is `"*"`), and return the rules a delivery was
/// attempted for, each with its outcome ([`RuleDelivery`], `SRC-17` F1).
/// This is the one public entry point for all six kinds:
/// `run_failed_event` and `run_finished_event` call it for
/// `connector_failure`, `connector_repeated_failure`, `connector_success`
/// and `upload_failure`; `SRC-8` (schema detection) calls it for
/// `connector_schema_change`; `SRC-11` (auto-disable) will call it for
/// `connector_disabled`, which nothing raises yet (decision D5).
///
/// `title_prefix` is joined to each rule's name (`"{title_prefix}: {name}"`),
/// like the pipeline kinds. `text` is the caller's message and must hold
/// only ids, names and a relative console link, never upstream error text
/// (decision D8). For `upload_failure`, whose rules are always scoped `"*"`,
/// pass the upload id as `connector_id`: it is not matched, only the kind is.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g.
/// `ClickHouse` unreachable). Per-rule delivery failures are reported via
/// [`DeliverResult`] inside `deliver_unless_silenced`, never propagated.
pub async fn evaluate_connector_event(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    kind: AlertKind,
    connector_id: &str,
    title_prefix: &str,
    text: &str,
    silence: Option<&dyn SilenceSource>,
) -> Result<Vec<RuleDelivery>, ChError> {
    if !kind.is_connector_scoped() {
        // Not a `SRC-7` kind: nothing to match on the `connector` column.
        return Ok(Vec::new());
    }
    deliver_matching(
        ch,
        http,
        email,
        silence,
        kind,
        RuleScope::Connector(connector_id),
        title_prefix,
        text,
    )
    .await
}

/// Plan 1e entry point: deliver every enabled [`AlertKind::PipelineFailure`]
/// rule that scopes `pipeline_id == pipeline_id_argument ||
/// "*"`, building a body that names the pipeline, the run id, and a relative
/// console link. Unlike [`run_rules`], this is event-driven — it is called
/// by the `run-failed` route once per failure, and the route is responsible
/// for the dedupe.
///
/// # Errors
///
/// Returns [`ChError`] only if [`list_rules`] itself fails (e.g.
/// `ClickHouse` unreachable). Per-rule delivery failures are reported via
/// [`DeliverResult`] inside `deliver_unless_silenced`, never propagated.
pub async fn evaluate_pipeline_failure(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    pipeline_id: &str,
    run_id: &str,
    failed_step_keys: &[String],
    silence: Option<&dyn SilenceSource>,
) -> Result<usize, ChError> {
    // A failure message is the user's own run id + the orchestrator's
    // failure metadata; never fabricates any other text. `failed_step_keys`
    // is bounded by `run_steps`'s own step count — a typical Dagster job
    // has at most a handful, so the message stays short even for noisy
    // graphs. The console link is RELATIVE — no `console_base_url`
    // configuration exists anywhere in this crate or `lakehouse-api`
    // today, and the route hand-off spec says "use the relative link
    // exactly as the plan specifies" rather than mint a new config key.
    let step_keys = if failed_step_keys.is_empty() {
        "(no failed step recorded)".to_owned()
    } else {
        failed_step_keys.join(", ")
    };
    let console_link = format!("/pipelines/{pipeline_id}?run={run_id}");
    let text = format!(
        "Pipeline {pipeline_id} run {run_id} failed. Failed steps: {step_keys}. View: {console_link}"
    );

    let rules: Vec<AlertRule> = list_rules(ch)
        .await?
        .into_iter()
        .filter(|r| {
            r.enabled
                && r.kind == AlertKind::PipelineFailure
                && r.pipeline
                    .as_deref()
                    .is_some_and(|p| p == pipeline_id || p == "*")
        })
        .collect();

    let mut delivered = 0_usize;
    for rule in &rules {
        if let Some(DeliverResult { .. }) = deliver_unless_silenced(
            http,
            email,
            silence,
            rule,
            &format!("🔥 Pipeline failure: {}", rule.name),
            &text,
        )
        .await
        {
            delivered += 1;
        }
    }
    Ok(delivered)
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
    http: &WebhookSender,
    email: &EmailSender,
    only: Option<&str>,
    freshness: Option<&dyn FreshnessSource>,
    silence: Option<&dyn SilenceSource>,
    state: Option<&dyn StateSource>,
    gate: &dyn SqlGate,
) -> Result<Vec<RunResult>, ChError> {
    let rules: Vec<AlertRule> = list_rules(ch)
        .await?
        .into_iter()
        .filter(|r| r.enabled && only.is_none_or(|id| id == r.id))
        .collect();

    let mut out = Vec::with_capacity(rules.len());
    for rule in &rules {
        out.push(run_one(ch, http, email, freshness, (silence, state), gate, rule).await);
    }
    Ok(out)
}

async fn run_one(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    freshness: Option<&dyn FreshnessSource>,
    (silence, state): (Option<&dyn SilenceSource>, Option<&dyn StateSource>),
    gate: &dyn SqlGate,
    rule: &AlertRule,
) -> RunResult {
    match rule.kind {
        AlertKind::Alert => run_alert(ch, (http, email), (silence, state), gate, rule).await,
        AlertKind::Digest => run_digest(ch, http, email, silence, gate, rule).await,
        AlertKind::Freshness => {
            run_freshness(freshness, http, email, silence, rule, now_millis()).await
        }
        // `pipeline_failure` rules are evaluated by
        // [`evaluate_pipeline_failure`] (the route handler calls it
        // directly with the per-event pipeline id and run id), never by
        // `run_rules` — there is no periodic trigger for a failure rule,
        // only the orchestrator's `run_failure_sensor` posting each event
        // to the route. A `pipeline_failure` rule reaching this match arm
        // is the wrong call site and is treated as skipped rather than
        // silently dropped.
        AlertKind::PipelineFailure => skipped(
            rule,
            "pipeline_failure rules are evaluated by the run-failed event route, not run_rules"
                .to_owned(),
        ),
        AlertKind::PipelineSlow | AlertKind::PipelineVolumeDrop => skipped(
            rule,
            "pipeline_slow / pipeline_volume_drop rules are evaluated by the run-finished \
             event route, not run_rules"
                .to_owned(),
        ),
        // BLD-2 review R1 SHOULD-FIX 13: pipeline_behind_target rules are evaluated by lag-check
        AlertKind::PipelineBehindTarget => skipped(
            rule,
            "pipeline_behind_target rules are evaluated by the lag-check route, not run_rules"
                .to_owned(),
        ),
        // Plan 1f: `pipeline_late` is evaluated by the `/api/alerts/run`
        // 15-minute pass, but the "is this pipeline currently late"
        // check needs Postgres access (SLA + last-success timestamp),
        // which `run_one` does not have. The route handler runs the
        // pipeline-specific evaluation directly and skips the rule here
        // so `run_rules`'s count stays an honest count of *its own*
        // work. The matching arm in `routes::alerts::run` is wired to
        // deliver on every `pipeline_late` rule whose pipeline is
        // currently late.
        AlertKind::PipelineLate => skipped(
            rule,
            "pipeline_late rules are evaluated by /api/alerts/run after run_rules returns"
                .to_owned(),
        ),
        // `SRC-7`: connector and upload rules fire on an event, never on the
        // periodic pass (`evaluate_connector_event`).
        AlertKind::ConnectorFailure
        | AlertKind::ConnectorRepeatedFailure
        | AlertKind::ConnectorDisabled
        | AlertKind::ConnectorSchemaChange
        | AlertKind::ConnectorSuccess
        | AlertKind::UploadFailure => skipped(
            rule,
            "connector and upload rules are evaluated by their event, not run_rules".to_owned(),
        ),
        // `DATA-14`: a quality incident rule fires when an incident opens.
        AlertKind::QualityIncident => skipped(
            rule,
            "quality incident rules are evaluated when an incident opens, not run_rules".to_owned(),
        ),
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
    http: &WebhookSender,
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

/// What one check of a threshold rule read: the number, whether it is over the
/// line, and the two sentences a message can carry. Built by whoever reads the
/// number (the evaluator here for a mart column or a metric, the API for a chart,
/// whose statement is built by API code) so that all of them settle through
/// [`settle`] and fire once.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The number read (for "returns any rows", the row count).
    pub value: f64,
    /// Whether it is over the line.
    pub crossed: bool,
    /// The message for a crossing.
    pub breached: String,
    /// The message for the recovery.
    pub back: String,
}

impl Reading {
    /// A number compared with the rule's `op` and `threshold`.
    #[must_use]
    pub fn against_threshold(rule: &AlertRule, watched: &str, value: f64) -> Self {
        let line = format!("{} {}", rule.op.as_str(), fmt_id_id(rule.threshold));
        Self {
            value,
            crossed: compare(value, rule.op, rule.threshold),
            breached: format!(
                "{watched} = {} {line} (threshold breached)",
                fmt_id_id(value)
            ),
            back: format!(
                "{watched} = {} (back within the line: no longer {line})",
                fmt_id_id(value)
            ),
        }
    }

    /// "Returns any rows" (D12): over the line when `rows` is above zero.
    #[must_use]
    pub fn any_rows(watched: &str, rows: usize) -> Self {
        #[allow(
            clippy::cast_precision_loss,
            reason = "a row count of a chart result is far below f64's exact-integer range"
        )]
        let value = rows as f64;
        Self {
            value,
            crossed: rows > 0,
            breached: format!("{watched} returned {rows} row(s)"),
            back: format!("{watched} returns no rows any more"),
        }
    }
}

/// The reason a failed read is recorded with: a fixed sentence, never engine text
/// (principle 4).
fn failure_reason(err: &AlertError) -> String {
    match err {
        AlertError::Validation(reason) => reason.clone(),
        AlertError::Clickhouse(_) => "the value could not be read.".to_owned(),
    }
}

/// Settle one check of a threshold rule (`BI-19` part C, D11): decide with
/// [`transition`] from the state before, send the message that decision calls for
/// (or none), and record the new state.
///
/// A read that failed records itself and changes nothing (C2). The transition
/// is claimed with a compare-and-set before anything is sent (`BI-19` C review
/// SF-1), so two overlapping runners send once. A message that could not be
/// delivered gives the claim back and leaves the state where it was, so the
/// next check sends it again rather than the crossing being lost; a message that is silenced
/// counts as handled (a silence hides the noise, not the fact). A state that
/// cannot be read skips the rule rather than guessing, since a guess is a repeat.
pub async fn settle(
    (http, email): (&WebhookSender, &EmailSender),
    silence: Option<&dyn SilenceSource>,
    state: Option<&dyn StateSource>,
    rule: &AlertRule,
    reading: Result<Reading, String>,
) -> RunResult {
    let reading = match reading {
        Ok(reading) => reading,
        Err(reason) => {
            if let Some(state) = state {
                state.record_failed(&rule.id, &reason).await;
            }
            return skipped(rule, reason);
        }
    };
    let Some(state) = state else {
        return skipped(rule, STATE_UNAVAILABLE.to_owned());
    };
    let previous = match state.state_of(&rule.id).await {
        Ok(previous) => previous,
        Err(reason) => return skipped(rule, reason),
    };
    let (next, sending) = transition(previous, reading.crossed);
    let (title, text) = match sending {
        Sending::Crossed => (format!("⚠️ Alert: {}", rule.name), Some(&reading.breached)),
        Sending::Recovered => (
            format!("✅ Back to normal: {}", rule.name),
            Some(&reading.back),
        ),
        Sending::Nothing => (String::new(), None),
    };
    let Some(text) = text else {
        state
            .record(&rule.id, next, Some(reading.value), None)
            .await;
        return RunResult {
            id: rule.id.clone(),
            name: rule.name.clone(),
            kind: rule.kind,
            fired: false,
            value: Some(reading.value),
            delivered: None,
            skipped: None,
            recovered: false,
        };
    };
    // BI-19 C review SF-1: claim the transition before sending. Of two runners
    // that read the same previous state only one wins and sends; the loser
    // reports the check as handled elsewhere.
    match state
        .claim(&rule.id, previous, next, Some(reading.value))
        .await
    {
        Ok(true) => {}
        Ok(false) => return skipped(rule, CLAIMED_ELSEWHERE.to_owned()),
        Err(reason) => return skipped(rule, reason),
    }
    let delivered = deliver_unless_silenced(http, email, silence, rule, &title, text).await;
    if delivered.as_ref().is_some_and(|d| !d.ok) {
        // Roll the claim back so the next check sends again.
        state.release(&rule.id, next, previous).await;
        state.record_failed(&rule.id, NOT_DELIVERED_RETRY).await;
    }
    RunResult {
        id: rule.id.clone(),
        name: rule.name.clone(),
        kind: rule.kind,
        fired: sending == Sending::Crossed,
        value: Some(reading.value),
        delivered,
        skipped: None,
        recovered: sending == Sending::Recovered,
    }
}

async fn run_alert(
    ch: &ChClient,
    (http, email): (&WebhookSender, &EmailSender),
    (silence, state): (Option<&dyn SilenceSource>, Option<&dyn StateSource>),
    gate: &dyn SqlGate,
    rule: &AlertRule,
) -> RunResult {
    // BI-19 part C (D12): the API reads a chart rule as its owner.
    if rule.chart.is_some() {
        return skipped(rule, CHART_SKIPPED.to_owned());
    }
    let mart = rule.mart.as_deref().unwrap_or("");
    let measure = rule.measure.as_deref().unwrap_or("");
    // BI-2: a rule that watches a metric reads the metric, not a column.
    let read = match rule.metric.as_deref() {
        Some(metric) => metric_value(ch, gate, metric).await,
        None => current_value(ch, gate, mart, measure, &rule.agg).await,
    };
    let watched = rule.metric.as_deref().map_or_else(
        || format!("{}({measure}) on {mart}", rule.agg),
        |m| format!("metric {m}"),
    );
    // BI-2 kept its own sentence for a metric that could not be read.
    let reading = read
        .map(|value| Reading::against_threshold(rule, &watched, value))
        .map_err(|err| match (&err, rule.metric.is_some()) {
            (AlertError::Clickhouse(_), true) => "the metric could not be read.".to_owned(),
            _ => failure_reason(&err),
        });
    settle((http, email), silence, state, rule, reading).await
}

async fn run_digest(
    ch: &ChClient,
    http: &WebhookSender,
    email: &EmailSender,
    silence: Option<&dyn SilenceSource>,
    gate: &dyn SqlGate,
    rule: &AlertRule,
) -> RunResult {
    let board = rule.board.as_deref().unwrap_or("");
    // BI-19 B1 review R6: a restricted dashboard's digest is not sent outside
    // the console; the run records why instead of delivering.
    if gate.board_restricted(board).await {
        return skipped(
            rule,
            "The dashboard is restricted to the people listed, so its digest is not sent outside the console."
                .to_owned(),
        );
    }
    match digest_text(ch, gate, board).await {
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
                recovered: false,
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
    http: &WebhookSender,
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
            recovered: false,
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
        recovered: false,
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
        recovered: false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    // ── `late` helper (plan 1f) ─────────────────────────────────────────
    //
    // These tests live here because the helper now does. They used to
    // live in `lakehouse-api::routes::pipelines::tests::sla_volume_helpers`;
    // moving the implementation here means moving the tests too — and
    // keeping them under the helper keeps the mutation check (strict
    // `>`) honest.

    #[test]
    fn late_is_none_when_no_late_threshold_is_set() {
        assert_eq!(late(Some(1_000.0), Some(900.0), None), None);
    }

    #[test]
    fn late_is_none_without_a_clock_to_measure_against() {
        assert_eq!(late(None, Some(900.0), Some(60)), None);
    }

    #[test]
    fn late_is_none_when_neither_threshold_nor_clock_anchors_the_decision() {
        assert_eq!(late(None, None, None), None);
    }

    #[test]
    fn late_is_true_when_a_threshold_is_set_but_no_run_has_ever_succeeded() {
        assert_eq!(late(Some(1_000.0), None, Some(60)), Some(true));
    }

    #[test]
    fn late_is_false_when_gap_is_under_the_threshold() {
        assert_eq!(late(Some(1_000.0), Some(950.0), Some(60)), Some(false));
    }

    #[test]
    fn late_is_true_when_gap_is_over_the_threshold() {
        assert_eq!(late(Some(1_000.0), Some(700.0), Some(200)), Some(true));
    }

    #[test]
    fn late_is_false_when_gap_is_exactly_the_threshold() {
        // Strict `>`: gap == threshold is NOT late.
        assert_eq!(late(Some(1_000.0), Some(800.0), Some(200)), Some(false));
    }

    // ── F2.7: `PipelineLateError::ListRules` Display ────────────────────
    //
    // The route layer at `routes/alerts.rs:757,793` formats the error
    // with `format!("{err}")`. Pre-fix that carried `ChError`'s body
    // verbatim (e.g. `Code: 47. Unknown identifier: nope`) into the 500
    // response. The sentinel text MUST NOT survive Display; the route
    // layer's output MUST be the fixed `"database error"` classified
    // string (AGENTS.md principle 4: classify before surfacing).

    #[test]
    fn pipeline_late_error_list_rules_display_does_not_forward_clickhouse_body() {
        const SENTINEL: &str = "ALERTS_LIST_RULES_SENTINEL_999111";
        let err: PipelineLateError =
            ChError::Server(format!("Code: 47. Unknown identifier — {SENTINEL}")).into();
        let rendered = format!("{err}");
        assert!(
            !rendered.contains(SENTINEL),
            "`PipelineLateError::ListRules` Display MUST NOT forward `ChError`'s body, got {rendered:?}",
        );
        assert_eq!(
            rendered, "database error",
            "`PipelineLateError::ListRules` Display MUST be the fixed \"database error\" classified string",
        );
    }

    #[test]
    fn pipeline_late_error_source_display_carries_the_supplied_string() {
        // Sanity: the `Source(String)` variant stays untouched by the F2
        // fix — the doc comment promises it is ALREADY classified by
        // the LateSource implementation, so it is safe to forward
        // verbatim.
        let err = PipelineLateError::Source("classified at the source".to_owned());
        assert_eq!(format!("{err}"), "classified at the source");
    }

    // ── SqlGate ──────────────────────────────────────────────────────────

    struct RewriteGate;
    #[async_trait::async_trait]
    impl SqlGate for RewriteGate {
        async fn gate(&self, sql: &str) -> Result<String, String> {
            Ok(format!("{sql} /* GOVERNED */"))
        }
        async fn board_restricted(&self, _board_id: &str) -> bool {
            false
        }
    }

    struct RefuseGate;
    #[async_trait::async_trait]
    impl SqlGate for RefuseGate {
        async fn gate(&self, _sql: &str) -> Result<String, String> {
            Err("refused by policy".to_owned())
        }
        async fn board_restricted(&self, _board_id: &str) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn an_alert_value_is_read_through_the_gate() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("GOVERNED"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "v", "type": "Float64"}],
                "data": [{"v": 7}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        let v = current_value(&ch, &RewriteGate, "mart_x", "amount", "sum")
            .await
            .expect("the governed statement answers");

        assert_eq!(v, 7.0);
        let bodies: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert!(
            bodies.iter().all(|b| b.contains("GOVERNED")),
            "only the gated statement may reach ClickHouse: {bodies:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_alert_value_never_reaches_clickhouse() {
        let server = wiremock::MockServer::start().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        let err = current_value(&ch, &RefuseGate, "mart_x", "amount", "sum")
            .await
            .expect_err("a refusal is an error, not a value");

        assert!(matches!(err, AlertError::Validation(ref m) if m == "refused by policy"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

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
        row.insert("pipeline".to_owned(), Value::String(String::new()));

        let rule = row_to_rule(&row);
        assert_eq!(rule.mart, None);
        assert_eq!(rule.measure, None);
        assert_eq!(rule.board, None);
        assert_eq!(rule.created_at, None);
        assert_eq!(rule.pipeline, None);
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
        row.insert("pipeline".to_owned(), Value::String(String::new()));

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
        assert_eq!(normalized.measure, "");
        assert_eq!(normalized.board, "");
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

    // ── AlertKind::PipelineFailure (plan 1e) ────────────────────────────

    #[test]
    fn pipeline_failure_kind_as_str_round_trips() {
        assert_eq!(AlertKind::PipelineFailure.as_str(), "pipeline_failure");
    }

    #[test]
    fn row_to_rule_maps_pipeline_failure_type_and_pipeline_column() {
        let mut row = Map::new();
        row.insert("id".to_owned(), Value::String("al_1".to_owned()));
        row.insert("name".to_owned(), Value::String("x".to_owned()));
        row.insert(
            "type".to_owned(),
            Value::String("pipeline_failure".to_owned()),
        );
        row.insert("mart".to_owned(), Value::String(String::new()));
        row.insert("measure".to_owned(), Value::String(String::new()));
        row.insert("agg".to_owned(), Value::String(String::new()));
        row.insert("op".to_owned(), Value::String(String::new()));
        row.insert("threshold".to_owned(), Value::String("0".to_owned()));
        row.insert("board".to_owned(), Value::String(String::new()));
        row.insert("channel".to_owned(), Value::String("webhook".to_owned()));
        row.insert("target".to_owned(), Value::String("t".to_owned()));
        row.insert("enabled".to_owned(), Value::String("1".to_owned()));
        row.insert("created_at".to_owned(), Value::String(String::new()));
        row.insert("severity".to_owned(), Value::String(String::new()));
        row.insert(
            "pipeline".to_owned(),
            Value::String("pl-orders-hourly".to_owned()),
        );

        let rule = row_to_rule(&row);
        assert_eq!(rule.kind, AlertKind::PipelineFailure);
        assert_eq!(rule.pipeline.as_deref(), Some("pl-orders-hourly"));
    }

    #[test]
    fn normalize_input_accepts_a_pipeline_failure_rule_with_star() {
        let input = AlertRuleInput {
            name: Some("Any pipeline failure".to_owned()),
            kind: Some("pipeline_failure".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: Some("*".to_owned()),
            ..AlertRuleInput::default()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::PipelineFailure);
        assert_eq!(normalized.pipeline, "*");
    }

    #[test]
    fn normalize_input_accepts_a_pipeline_failure_rule_with_pipeline_id() {
        let input = AlertRuleInput {
            name: Some("Orders failure".to_owned()),
            kind: Some("pipeline_failure".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: Some("pl-orders".to_owned()),
            ..AlertRuleInput::default()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::PipelineFailure);
        assert_eq!(normalized.pipeline, "pl-orders");
    }

    #[test]
    fn normalize_input_rejects_a_pipeline_failure_rule_with_no_pipeline() {
        let input = AlertRuleInput {
            name: Some("Orders failure".to_owned()),
            kind: Some("pipeline_failure".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: None,
            ..AlertRuleInput::default()
        };
        let err = normalize_input(&input).unwrap_err();
        assert!(
            err.to_string()
                .contains("pipeline_failure requires a pipeline id or"),
            "{err}"
        );
    }

    #[test]
    fn normalize_input_rejects_a_pipeline_failure_rule_with_garbage_pipeline() {
        let input = AlertRuleInput {
            name: Some("Orders failure".to_owned()),
            kind: Some("pipeline_failure".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: Some("gold_export_job".to_owned()),
            ..AlertRuleInput::default()
        };
        let err = normalize_input(&input).unwrap_err();
        assert!(err.to_string().contains("pl-... id or \"*\""), "{err}");
    }

    /// Plan 1e: the `pipeline` column is added via the same
    /// `ADD COLUMN IF NOT EXISTS` convention `severity` already uses
    /// (`ensure`'s block comment :552-557). This test pins that the new
    /// statement is sent, so a future refactor that forgets it would not
    /// silently leave existing deployments with a missing column on
    /// production data.
    #[tokio::test]
    async fn ensure_sends_an_additive_pipeline_column_statement() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("ADD COLUMN IF NOT EXISTS pipeline"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        ensure(&ch).await.unwrap();
    }

    /// Plan 1e: when a rule fires, the webhook receives a body that names
    /// the pipeline, the run id, and a relative `/pipelines/<id>?run=<runId>`
    /// link — never a Dagster error message (AGENTS.md rule 4). The body
    /// shape is the wire contract the route tests assert against.
    #[tokio::test]
    async fn evaluate_pipeline_failure_delivers_a_message_naming_pipeline_and_run() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // The webhook the test expects to be hit once with the body shape
        // this plan's contract specifies. wiremock intercepts the URL its
        // server is bound to, so the rule's `target` is the mock's own
        // `/hooks/example` path — the contract under test is the *body*
        // the alert sends, not the URL it sends to.
        let webhook = format!("{}/hooks/example", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/example"))
            .and(body_string_contains("pl-orders"))
            .and(body_string_contains("run-deadbeef"))
            .and(body_string_contains(
                "/pipelines/pl-orders?run=run-deadbeef",
            ))
            .and(body_string_contains("bronze_load"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        // Catch-all for every other ClickHouse call (`ensure`, `save_rule`
        // INSERT, `list_rules` SELECT). Returns a rule row that round-trips
        // back through `row_to_rule` to the kind/pipeline we just saved —
        // a real fixture would persist this; the mock just gives the same
        // answer on every read.
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_failure_rule_row_json(
                    "al_pf1",
                    "Orders failure",
                    "pl-orders",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Orders failure".to_owned()),
                kind: Some("pipeline_failure".to_owned()),
                target: Some(webhook.clone()),
                channel: Some("webhook".to_owned()),
                pipeline: Some("pl-orders".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pf1"),
        )
        .await
        .unwrap();

        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_failure(
            &ch,
            &http,
            &email,
            "pl-orders",
            "run-deadbeef",
            &["bronze_load".to_owned()],
            None,
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);
    }

    /// `evaluate_pipeline_failure` matches `*` rules on every pipeline id,
    /// the same wildcard convention every other kind of rule's "every
    /// <thing>" mode uses.
    #[tokio::test]
    async fn evaluate_pipeline_failure_matches_the_wildcard_star_rule() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/wildcard", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/wildcard"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_failure_rule_row_json(
                    "al_pf2",
                    "Any failure",
                    "*",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Any failure".to_owned()),
                kind: Some("pipeline_failure".to_owned()),
                target: Some(webhook.clone()),
                channel: Some("webhook".to_owned()),
                pipeline: Some("*".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pf2"),
        )
        .await
        .unwrap();

        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched =
            evaluate_pipeline_failure(&ch, &http, &email, "pl-some-other", "run-x", &[], None)
                .await
                .unwrap();
        assert_eq!(matched, 1);
    }

    /// Rules scoped to a different pipeline id do NOT match — `*` and a
    /// specific id are the only matching values, never one rule's `pipeline`
    /// being treated as another rule's scope.
    #[tokio::test]
    async fn evaluate_pipeline_failure_does_not_match_a_rule_scoped_to_a_different_id() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/never", server.uri());
        // Only the catch-all ClickHouse mock is mounted — no webhook path
        // matcher, so if `evaluate_pipeline_failure` mistakenly tried to
        // deliver to this rule wiremock would panic on the unmatched POST.
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_failure_rule_row_json(
                    "al_pf3",
                    "Other failure",
                    "pl-other",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Other failure".to_owned()),
                kind: Some("pipeline_failure".to_owned()),
                target: Some(webhook),
                channel: Some("webhook".to_owned()),
                pipeline: Some("pl-other".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pf3"),
        )
        .await
        .unwrap();

        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched =
            evaluate_pipeline_failure(&ch, &http, &email, "pl-orders", "run-deadbeef", &[], None)
                .await
                .unwrap();
        assert_eq!(
            matched, 0,
            "a scoped rule for another pipeline must not fire"
        );
    }

    /// Single source of truth for the catch-all mock row shape the three
    /// tests above need from `ClickHouse` — every field `row_to_rule` reads
    /// populated, so a `pipeline_failure` rule survives the round-trip.
    fn pipeline_failure_rule_row_json(
        id: &str,
        name: &str,
        pipeline: &str,
        target: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "meta": [],
            "data": [{
                "id": id,
                "name": name,
                "type": "pipeline_failure",
                "mart": "",
                "measure": "",
                "agg": "",
                "op": "",
                "threshold": "0",
                "board": "",
                "channel": "webhook",
                "target": target,
                "enabled": "1",
                "created_at": "",
                "severity": "",
                "pipeline": pipeline,
            }],
            "rows": 1,
        })
    }

    // ── `evaluate_pipeline_late` (plan 1f) ───────────────────────────────

    /// A small in-process [`LateSource`] the tests below drive: a
    /// pre-built map of `(threshold_seconds, last_success_epoch_seconds)`
    /// keyed by pipeline id. Anything not in the map returns
    /// `Ok(None)` — the "no SLA row" path that [`evaluate_pipeline_late`]
    /// treats as `unsupported` for every matching rule.
    struct FixedLateSource(std::collections::HashMap<String, (i32, Option<f64>)>);

    #[async_trait::async_trait]
    impl LateSource for FixedLateSource {
        async fn late_inputs(
            &self,
            pipeline_id: &str,
        ) -> Result<Option<(i32, Option<f64>)>, String> {
            Ok(self.0.get(pipeline_id).map(|(t, l)| (*t, *l)))
        }
    }

    fn pipeline_late_rule_row_json(
        id: &str,
        name: &str,
        pipeline: &str,
        target: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "meta": [],
            "data": [{
                "id": id,
                "name": name,
                "type": "pipeline_late",
                "mart": "",
                "measure": "",
                "agg": "",
                "op": "",
                "threshold": "0",
                "board": "",
                "channel": "webhook",
                "target": target,
                "enabled": "1",
                "created_at": "",
                "severity": "",
                "pipeline": pipeline,
            }],
            "rows": 1,
        })
    }

    /// A late pipeline (no last-success within the threshold) DOES fire
    /// the matching rule with the route's text shape — no Dagster
    /// details, a relative console link.
    #[tokio::test]
    async fn evaluate_pipeline_late_delivers_when_threshold_is_breached() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/late", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/late"))
            .and(body_string_contains("pl-orders"))
            .and(body_string_contains("/pipelines/pl-orders"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_late_rule_row_json(
                    "al_pl1",
                    "Orders late",
                    "pl-orders",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Orders late".to_owned()),
                kind: Some("pipeline_late".to_owned()),
                target: Some(webhook.clone()),
                channel: Some("webhook".to_owned()),
                pipeline: Some("pl-orders".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pl1"),
        )
        .await
        .unwrap();

        // Threshold 60 s, last success 600 s ago → definitely late.
        let mut map = std::collections::HashMap::new();
        map.insert("pl-orders".to_owned(), (60_i32, Some(1_000.0_f64 - 600.0)));
        let source = FixedLateSource(map);
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_late(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(&source),
            None,
            Some(1_000.0),
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);
    }

    /// A pipeline with a SUCCESS run inside the threshold does NOT
    /// fire — the route handler must skip rather than silently
    /// passing.
    #[tokio::test]
    async fn evaluate_pipeline_late_does_not_fire_when_within_threshold() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/never", server.uri());
        // Only the catch-all ClickHouse mock; no webhook matcher.
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_late_rule_row_json(
                    "al_pl2",
                    "Within threshold",
                    "pl-orders",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Within threshold".to_owned()),
                kind: Some("pipeline_late".to_owned()),
                target: Some(webhook),
                channel: Some("webhook".to_owned()),
                pipeline: Some("pl-orders".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pl2"),
        )
        .await
        .unwrap();

        // Last success 30 s ago, threshold 60 s → not late.
        let mut map = std::collections::HashMap::new();
        map.insert("pl-orders".to_owned(), (60_i32, Some(1_000.0_f64 - 30.0)));
        let source = FixedLateSource(map);
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_late(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(&source),
            None,
            Some(1_000.0),
        )
        .await
        .unwrap();
        assert_eq!(matched, 0);
    }

    /// A pipeline with NO SLA row (`late_inputs` returns `None`) does
    /// not fire — the "config gap" path. Plan 1f rule: never silently
    /// fire on missing config.
    #[tokio::test]
    async fn evaluate_pipeline_late_does_not_fire_when_no_sla_row_exists() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/never", server.uri());
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_late_rule_row_json(
                    "al_pl3",
                    "No SLA",
                    "pl-orders",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("No SLA".to_owned()),
                kind: Some("pipeline_late".to_owned()),
                target: Some(webhook),
                channel: Some("webhook".to_owned()),
                pipeline: Some("pl-orders".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pl3"),
        )
        .await
        .unwrap();

        let source = FixedLateSource(std::collections::HashMap::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_late(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(&source),
            None,
            Some(1_000.0),
        )
        .await
        .unwrap();
        assert_eq!(matched, 0);
    }

    /// A pipeline with no recorded SUCCESS run AND a threshold IS
    /// set IS late — same posture as the runs route's
    /// `late_is_true_when_a_threshold_is_set_but_no_run_has_ever_succeeded`.
    #[tokio::test]
    async fn evaluate_pipeline_late_fires_when_no_success_run_with_a_threshold() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/late-no-success", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/late-no-success"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_late_rule_row_json(
                    "al_pl4",
                    "Never succeeded",
                    "pl-orders",
                    &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Never succeeded".to_owned()),
                kind: Some("pipeline_late".to_owned()),
                target: Some(webhook.clone()),
                channel: Some("webhook".to_owned()),
                pipeline: Some("pl-orders".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pl4"),
        )
        .await
        .unwrap();

        let mut map = std::collections::HashMap::new();
        map.insert("pl-orders".to_owned(), (60_i32, None));
        let source = FixedLateSource(map);
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_late(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(&source),
            None,
            Some(1_000.0),
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);
    }

    /// Wildcard `*` rules DO match — same convention every other
    /// pipeline-scoped kind uses. Plan 1f does not differentiate.
    #[tokio::test]
    async fn evaluate_pipeline_late_matches_the_wildcard_star_rule() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/wildcard-late", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/wildcard-late"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pipeline_late_rule_row_json(
                    "al_pl5", "Any late", "*", &webhook,
                )),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        save_rule(
            &ch,
            &test_webhook_sender(),
            &AlertRuleInput {
                name: Some("Any late".to_owned()),
                kind: Some("pipeline_late".to_owned()),
                target: Some(webhook.clone()),
                channel: Some("webhook".to_owned()),
                pipeline: Some("*".to_owned()),
                ..AlertRuleInput::default()
            },
            Some("al_pl5"),
        )
        .await
        .unwrap();

        let mut map = std::collections::HashMap::new();
        map.insert(
            "pl-some-other".to_owned(),
            (60_i32, Some(1_000.0_f64 - 600.0)),
        );
        let source = FixedLateSource(map);
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_late(
            &ch,
            &http,
            &email,
            "pl-some-other",
            Some(&source),
            None,
            Some(1_000.0),
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);
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
            pipeline: None,
            metric: None,
            connector: None,
            asset: None,
            chart: None,
            series: None,
            filters: Vec::new(),
            condition: None,
            owner_id: None,
        }
    }

    /// `SEC-10`: a webhook sender that approves every target, so these
    /// tests can deliver to a local `wiremock` listener. The address policy
    /// itself is tested where it lives, in the API crate; here only the
    /// wiring (save checks, delivery goes through the sender) is under test.
    fn test_webhook_sender() -> WebhookSender {
        lakehouse_notify::MappedResolver::new().into_sender()
    }

    /// `SEC-10`: a resolver that refuses everything, standing in for the API
    /// crate's address policy.
    struct RefuseAll;

    impl lakehouse_notify::TargetResolver for RefuseAll {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> lakehouse_notify::ResolveFuture<'a> {
            Box::pin(async { Err(lakehouse_notify::TargetRefusal::NotAllowed) })
        }
    }

    /// `SEC-10`: saving a webhook rule asks the sender first. A refused
    /// target is a validation error with the sender's fixed message, and
    /// nothing is inserted.
    #[tokio::test]
    async fn save_rule_refuses_a_webhook_the_sender_refuses() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let refusing = WebhookSender::new(std::sync::Arc::new(RefuseAll));

        let err = save_rule(
            &ch,
            &refusing,
            &AlertRuleInput {
                target: Some("http://internal.test/hook".to_owned()),
                ..valid_alert_input()
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "webhook address is not allowed");
        let inserts = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| String::from_utf8_lossy(&r.body).contains("INSERT INTO console.alert_rule"))
            .count();
        assert_eq!(inserts, 0, "a refused rule must not be written");
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
        let http = test_webhook_sender();
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
        let http = test_webhook_sender();
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
        let http = test_webhook_sender();
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
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let result = run_freshness(Some(&src), &http, &email, None, &rule, 60 * 60_000).await;
        assert!(!result.fired, "a missing dataset_sla row must never fire");
        assert!(result.skipped.is_some());
    }

    #[tokio::test]
    async fn freshness_skips_when_no_source_is_configured() {
        let rule = freshness_rule("bronze.orders");
        let http = test_webhook_sender();
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
        let http = test_webhook_sender();
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
        let http = test_webhook_sender();
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
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered =
            deliver_unless_silenced(&http, &email, Some(&AlwaysSilenced), &rule, "title", "text")
                .await;
        assert!(delivered.is_none());
    }

    #[tokio::test]
    async fn deliver_unless_silenced_with_no_source_configured_behaves_as_unsilenced() {
        let rule = freshness_rule("bronze.orders");
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered = deliver_unless_silenced(&http, &email, None, &rule, "title", "text").await;
        assert!(delivered.is_some());
    }

    // ── BI-2: a rule that watches a metric ──────────────────────────────

    #[test]
    fn a_rule_that_watches_a_metric_needs_no_mart_measure_or_aggregate() {
        let normalized = normalize_input(&AlertRuleInput {
            metric: Some(" total_visitors ".to_owned()),
            mart: None,
            measure: None,
            op: Some(">".to_owned()),
            threshold: Some(100_000.0),
            ..valid_alert_input()
        })
        .unwrap();
        assert_eq!(normalized.metric, "total_visitors");
        assert_eq!(
            (normalized.mart.as_str(), normalized.measure.as_str()),
            ("", "")
        );
        assert_eq!(normalized.threshold, 100_000.0);
        for bad in ["bad name", "x;DROP", "1abc"] {
            let err = normalize_input(&AlertRuleInput {
                metric: Some(bad.to_owned()),
                ..valid_alert_input()
            })
            .unwrap_err();
            assert_eq!(err.to_string(), "invalid metric.", "{bad}");
        }
        // A blank metric is a rule on a column, as before.
        let plain = normalize_input(&AlertRuleInput {
            metric: Some("  ".to_owned()),
            ..valid_alert_input()
        })
        .unwrap();
        assert_eq!(plain.metric, "");
    }

    fn metric_rule(metric: &str, op: AlertOp, threshold: f64) -> AlertRule {
        AlertRule {
            id: "al_metric".to_owned(),
            name: "watch".to_owned(),
            kind: AlertKind::Alert,
            mart: None,
            measure: None,
            agg: "sum".to_owned(),
            op,
            threshold,
            board: None,
            channel: AlertChannel::Webhook,
            target: "http://hooks.test/x".to_owned(),
            enabled: true,
            created_at: None,
            severity: None,
            pipeline: None,
            metric: Some(metric.to_owned()),
            connector: None,
            asset: None,
            chart: None,
            series: None,
            filters: Vec::new(),
            condition: None,
            owner_id: None,
        }
    }

    struct EverythingSilenced;
    #[async_trait::async_trait]
    impl SilenceSource for EverythingSilenced {
        async fn is_silenced(&self, _rule_id: &str) -> bool {
            true
        }
    }

    /// A metric that does not exist (or whose store cannot be read) makes the
    /// rule's evaluation fail with a fixed reason: it is neither fired nor
    /// reported as a value that did not breach.
    #[tokio::test]
    async fn a_metric_that_is_gone_makes_the_rule_skip_with_a_fixed_reason_and_never_fire() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [], "data": [], "rows": 0,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = WebhookSender::new(std::sync::Arc::new(RefuseAll));
        let email = no_smtp_email_sender();
        let rule = metric_rule("gone", AlertOp::Gt, 0.0);
        let result = run_one(&ch, &http, &email, None, (None, None), &RewriteGate, &rule).await;
        assert!(!result.fired);
        assert_eq!(result.value, None);
        assert_eq!(
            result.skipped.as_deref(),
            Some("the metric no longer exists.")
        );
        // A store that cannot be read says so, without any engine text.
        let down = ChClient::new(
            "http://127.0.0.1:1".to_owned(),
            "default".to_owned(),
            String::new(),
        );
        let result = run_one(
            &down,
            &http,
            &email,
            None,
            (None, None),
            &RewriteGate,
            &rule,
        )
        .await;
        assert!(!result.fired);
        assert_eq!(
            result.skipped.as_deref(),
            Some("the metric could not be read.")
        );
    }

    struct PassGate;
    #[async_trait::async_trait]
    impl SqlGate for PassGate {
        async fn gate(&self, sql: &str) -> Result<String, String> {
            Ok(sql.to_owned())
        }
        async fn board_restricted(&self, _board_id: &str) -> bool {
            false
        }
    }

    /// `BI-19` B1 review R6: answers that every board is restricted.
    struct RestrictedGate;
    #[async_trait::async_trait]
    impl SqlGate for RestrictedGate {
        async fn gate(&self, sql: &str) -> Result<String, String> {
            Ok(sql.to_owned())
        }
        async fn board_restricted(&self, _board_id: &str) -> bool {
            true
        }
    }

    /// Against the dev engine: the metric `total_visitors_demo` (`Sum([visitors])`
    /// on `serving.mart_demo_map_points`, 278,955) must already exist in
    /// `console.bi_metric`. Read-only; the rule is silenced so nothing is
    /// delivered. Run by hand with `LAYER_ENGINE_URL`, `LAYER_ENGINE_USER`,
    /// `LAYER_ENGINE_PASSWORD` and `-- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs the dev engine and the demo metric; see the doc comment"]
    async fn a_rule_on_the_demo_metric_fires_above_its_value_and_not_below_it() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let ch = ChClient::new(
            var("LAYER_ENGINE_URL"),
            var("LAYER_ENGINE_USER"),
            var("LAYER_ENGINE_PASSWORD"),
        );
        let http = WebhookSender::new(std::sync::Arc::new(RefuseAll));
        let email = no_smtp_email_sender();
        let silence = EverythingSilenced;
        for (op, threshold, expect_fired) in [
            (AlertOp::Gt, 100_000.0, true),
            (AlertOp::Lt, 100_000.0, false),
            (AlertOp::Ge, 278_955.0, true),
            (AlertOp::Gt, 278_955.0, false),
        ] {
            let rule = metric_rule("total_visitors_demo", op, threshold);
            let r = run_one(
                &ch,
                &http,
                &email,
                None,
                (Some(&silence), None),
                &PassGate,
                &rule,
            )
            .await;
            eprintln!(
                "RULE total_visitors_demo {} {threshold} -> fired {} value {:?} skipped {:?}",
                op.as_str(),
                r.fired,
                r.value,
                r.skipped
            );
            assert_eq!(r.skipped, None);
            assert_eq!(r.value, Some(278_955.0));
            assert_eq!(r.fired, expect_fired);
        }
        let gone = metric_rule("no_such_metric_here", AlertOp::Gt, 0.0);
        let r = run_one(
            &ch,
            &http,
            &email,
            None,
            (Some(&silence), None),
            &PassGate,
            &gone,
        )
        .await;
        eprintln!(
            "RULE no_such_metric_here -> fired {} skipped {:?}",
            r.fired, r.skipped
        );
        assert!(!r.fired && r.skipped.as_deref() == Some("the metric no longer exists."));
    }

    // ── SRC-7: connector and upload kinds ───────────────────────────────

    fn connector_input(kind: &str, connector: Option<&str>) -> AlertRuleInput {
        AlertRuleInput {
            name: Some("Load failed".to_owned()),
            kind: Some(kind.to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            connector: connector.map(str::to_owned),
            ..AlertRuleInput::default()
        }
    }

    #[test]
    fn src7_kinds_have_snake_case_strings_in_as_str_and_on_the_wire() {
        for (kind, wire) in [
            (AlertKind::ConnectorFailure, "connector_failure"),
            (
                AlertKind::ConnectorRepeatedFailure,
                "connector_repeated_failure",
            ),
            (AlertKind::ConnectorDisabled, "connector_disabled"),
            (AlertKind::ConnectorSchemaChange, "connector_schema_change"),
            (AlertKind::ConnectorSuccess, "connector_success"),
            (AlertKind::UploadFailure, "upload_failure"),
        ] {
            assert_eq!(kind.as_str(), wire);
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(wire));
            assert_eq!(AlertKind::parse_connector_kind(wire), Some(kind));
            assert!(kind.is_connector_scoped());
        }
        assert!(!AlertKind::PipelineFailure.is_connector_scoped());
    }

    #[test]
    fn normalize_input_accepts_connector_kinds_with_an_id_or_star() {
        for kind in [
            "connector_failure",
            "connector_repeated_failure",
            "connector_disabled",
            "connector_schema_change",
        ] {
            let one = normalize_input(&connector_input(kind, Some("conn-pg-oms"))).unwrap();
            assert_eq!(one.connector, "conn-pg-oms");
            assert_eq!(one.pipeline, "");
            let all = normalize_input(&connector_input(kind, Some("*"))).unwrap();
            assert_eq!(all.connector, "*");
        }
        let success =
            normalize_input(&connector_input("connector_success", Some("conn-a"))).unwrap();
        assert_eq!(success.kind, AlertKind::ConnectorSuccess);
    }

    #[test]
    fn normalize_input_refuses_a_connector_rule_without_a_valid_connector() {
        let missing = normalize_input(&connector_input("connector_failure", None)).unwrap_err();
        assert!(
            missing.to_string().contains("requires a connector id"),
            "{missing}"
        );
        let blank = normalize_input(&connector_input("connector_failure", Some("  "))).unwrap_err();
        assert!(
            blank.to_string().contains("requires a connector id"),
            "{blank}"
        );
        for bad in ["conn a", "conn;drop", "conn\n1", "con'n", "é"] {
            let err =
                normalize_input(&connector_input("connector_failure", Some(bad))).unwrap_err();
            assert_eq!(
                err.to_string(),
                "connector_failure connector is not a valid connector id."
            );
        }
        let long = "a".repeat(MAX_CONNECTOR_ID_LEN + 1);
        assert!(normalize_input(&connector_input("connector_failure", Some(&long))).is_err());
    }

    /// Decision D4: a success rule names one connector, never `"*"`.
    #[test]
    fn normalize_input_refuses_a_connector_success_rule_for_star_with_a_fixed_message() {
        let err = normalize_input(&connector_input("connector_success", Some("*"))).unwrap_err();
        assert_eq!(
            err.to_string(),
            "connector_success must name one connector, not \"*\"."
        );
    }

    #[test]
    fn normalize_input_stores_star_for_an_upload_failure_rule_whatever_was_sent() {
        let a = normalize_input(&connector_input("upload_failure", None)).unwrap();
        assert_eq!(
            (a.kind, a.connector.as_str()),
            (AlertKind::UploadFailure, "*")
        );
        let b = normalize_input(&connector_input("upload_failure", Some("conn-x"))).unwrap();
        assert_eq!(b.connector, "*");
    }

    #[test]
    fn row_to_rule_maps_the_connector_column() {
        let mut row = Map::new();
        row.insert(
            "type".to_owned(),
            Value::String("connector_failure".to_owned()),
        );
        row.insert("connector".to_owned(), Value::String("conn-a".to_owned()));
        let rule = row_to_rule(&row);
        assert_eq!(rule.kind, AlertKind::ConnectorFailure);
        assert_eq!(rule.connector.as_deref(), Some("conn-a"));
        row.insert("connector".to_owned(), Value::String(String::new()));
        assert_eq!(row_to_rule(&row).connector, None);
    }

    #[tokio::test]
    async fn ensure_sends_an_additive_connector_column_statement() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("ADD COLUMN IF NOT EXISTS connector"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        ensure(&ch).await.unwrap();
    }

    /// The connector scope reaches `ClickHouse` as an escaped literal, never
    /// as raw interpolation: a quote in an (invalid) id is refused before
    /// SQL is built, and a valid id is sent quoted.
    #[tokio::test]
    async fn save_rule_sends_the_connector_scope_as_a_quoted_literal() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("INSERT INTO console.alert_rule"))
            .and(body_string_contains("'connector_failure'"))
            .and(body_string_contains("'conn-pg-oms'"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(connector_rules_json(&[(
                    "al_c1",
                    "connector_failure",
                    "conn-pg-oms",
                    "https://hooks.example.com/x",
                    true,
                )])),
            )
            .mount(&server)
            .await;
        // `SEC-10` merged with `SRC-7`: the rule's webhook target is a name, so
        // the sender needs a resolver that approves it (nothing is dialled).
        let webhooks = lakehouse_notify::MappedResolver::new()
            .with("hooks.example.com", "127.0.0.1:9".parse().unwrap())
            .into_sender();
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let saved = save_rule(
            &ch,
            &webhooks,
            &connector_input("connector_failure", Some("conn-pg-oms")),
            Some("al_c1"),
        )
        .await
        .unwrap();
        assert_eq!(saved.connector.as_deref(), Some("conn-pg-oms"));

        let err = save_rule(
            &ch,
            &webhooks,
            &connector_input("connector_failure", Some("x' OR 1=1 --")),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AlertError::Validation(_)));
    }

    /// `(id, kind, connector, target, enabled)` rows, every field
    /// `row_to_rule` reads populated.
    fn connector_rules_json(rows: &[(&str, &str, &str, &str, bool)]) -> serde_json::Value {
        let data: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, kind, connector, target, enabled)| {
                serde_json::json!({
                    "id": id, "name": format!("Rule {id}"), "type": kind,
                    "mart": "", "measure": "", "agg": "", "op": "", "threshold": "0",
                    "board": "", "channel": "webhook", "target": target,
                    "enabled": if *enabled { "1" } else { "0" },
                    "created_at": "", "severity": "", "pipeline": "",
                    "connector": connector,
                })
            })
            .collect();
        serde_json::json!({ "meta": [], "data": data, "rows": data.len() })
    }

    /// Starts a mock where `ClickHouse` answers every call with `rows` and
    /// each rule's webhook path `/hooks/<id>` is expected `expect` times.
    async fn connector_event_server(
        rows: &[(&str, &str, &str, bool)],
        expectations: &[(&str, u64)],
    ) -> (wiremock::MockServer, ChClient) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for (id, times) in expectations {
            Mock::given(method("POST"))
                .and(path(format!("/hooks/{id}")))
                .respond_with(ResponseTemplate::new(200))
                .expect(*times)
                .mount(&server)
                .await;
        }
        let full: Vec<(&str, &str, &str, String, bool)> = rows
            .iter()
            .map(|(id, kind, connector, enabled)| {
                (
                    *id,
                    *kind,
                    *connector,
                    format!("{}/hooks/{id}", server.uri()),
                    *enabled,
                )
            })
            .collect();
        let refs: Vec<(&str, &str, &str, &str, bool)> = full
            .iter()
            .map(|(id, kind, connector, target, enabled)| {
                (*id, *kind, *connector, target.as_str(), *enabled)
            })
            .collect();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(connector_rules_json(&refs)))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        (server, ch)
    }

    /// A rule scoped to one connector delivers for that connector only; a
    /// `*` rule delivers for every connector; a disabled rule and a rule of
    /// another kind never deliver. The message names the connector and run
    /// and links to the console (decision D8).
    #[tokio::test]
    async fn evaluate_connector_event_matches_its_connector_star_and_kind_only() {
        let (_server, ch) = connector_event_server(
            &[
                ("one", "connector_failure", "conn-a", true),
                ("other", "connector_failure", "conn-b", true),
                ("all", "connector_failure", "*", true),
                ("off", "connector_failure", "*", false),
                ("kind", "connector_success", "conn-a", true),
            ],
            &[
                ("one", 1),
                ("other", 0),
                ("all", 1),
                ("off", 0),
                ("kind", 0),
            ],
        )
        .await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered = evaluate_connector_event(
            &ch,
            &http,
            &email,
            AlertKind::ConnectorFailure,
            "conn-a",
            "Connector failure",
            "Connector Orders (conn-a) run run-1 failed. View: /connectors/conn-a",
            None,
        )
        .await
        .unwrap();
        let mut ids: Vec<&str> = delivered.iter().map(|d| d.rule.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["all", "one"]);
    }

    #[tokio::test]
    async fn evaluate_connector_event_delivers_the_message_text_unchanged() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/msg", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/msg"))
            .and(body_string_contains("conn-a"))
            .and(body_string_contains("run-1"))
            .and(body_string_contains("/connectors/conn-a"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(connector_rules_json(&[(
                    "al_m",
                    "connector_failure",
                    "conn-a",
                    &webhook,
                    true,
                )])),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered = evaluate_connector_event(
            &ch,
            &http,
            &email,
            AlertKind::ConnectorFailure,
            "conn-a",
            "Connector failure",
            "Connector Orders (conn-a) run run-1 failed. View: /connectors/conn-a",
            None,
        )
        .await
        .unwrap();
        assert_eq!(delivered.len(), 1);
    }

    #[tokio::test]
    async fn evaluate_connector_event_does_not_deliver_a_silenced_rule() {
        let (_server, ch) = connector_event_server(
            &[("quiet", "connector_failure", "conn-a", true)],
            &[("quiet", 0)],
        )
        .await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered = evaluate_connector_event(
            &ch,
            &http,
            &email,
            AlertKind::ConnectorFailure,
            "conn-a",
            "Connector failure",
            "text",
            Some(&AlwaysSilenced),
        )
        .await
        .unwrap();
        assert!(delivered.is_empty());
    }

    /// `SRC-11` has no caller yet (decision D5) and `SRC-8` calls from
    /// `routes::schema_changes`; the entry point delivers both kinds.
    #[tokio::test]
    async fn evaluate_connector_event_delivers_connector_disabled_and_schema_change_rules() {
        let (_server, ch) = connector_event_server(
            &[
                ("dis", "connector_disabled", "conn-a", true),
                ("sch", "connector_schema_change", "*", true),
            ],
            &[("dis", 1), ("sch", 1)],
        )
        .await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        for (kind, expected) in [
            (AlertKind::ConnectorDisabled, "dis"),
            (AlertKind::ConnectorSchemaChange, "sch"),
        ] {
            let delivered =
                evaluate_connector_event(&ch, &http, &email, kind, "conn-a", "Event", "text", None)
                    .await
                    .unwrap();
            assert_eq!(delivered.len(), 1);
            assert_eq!(delivered[0].rule.id, expected);
        }
    }

    /// `upload_failure` rules are stored `"*"`, so they match whatever id
    /// the caller passes (the upload id).
    #[tokio::test]
    async fn evaluate_connector_event_delivers_an_upload_failure_rule_for_any_upload() {
        let (_server, ch) =
            connector_event_server(&[("up", "upload_failure", "*", true)], &[("up", 1)]).await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered = evaluate_connector_event(
            &ch,
            &http,
            &email,
            AlertKind::UploadFailure,
            "upl-1",
            "Upload failure",
            "text",
            None,
        )
        .await
        .unwrap();
        assert_eq!(delivered.len(), 1);
    }

    /// `SRC-17` F1 (delivery review 2026-10-12, D-a): the entry point hands
    /// back what each delivery did. A webhook answering 500, a host nobody
    /// listens on, a non-`http` address and an e-mail rule on an installation
    /// without `SMTP` each give their own fixed reason, and none of those
    /// reasons contains anything of the other side (status code, address).
    #[tokio::test]
    async fn evaluate_connector_event_reports_each_deliverys_outcome_with_a_fixed_reason() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let closed_port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let server = MockServer::start().await;
        for (route, status) in [("err", 500), ("ok", 200)] {
            Mock::given(method("POST"))
                .and(path(format!("/hooks/{route}")))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
        }
        let down = format!("http://127.0.0.1:{closed_port}/hooks/down");
        let err = format!("{}/hooks/err", server.uri());
        let ok = format!("{}/hooks/ok", server.uri());
        let mut rules = connector_rules_json(&[
            ("r_ok", "connector_failure", "conn-a", &ok, true),
            ("r_500", "connector_failure", "conn-a", &err, true),
            ("r_down", "connector_failure", "conn-a", &down, true),
            (
                "r_bad",
                "connector_failure",
                "conn-a",
                "ftp://example.invalid/x",
                true,
            ),
            (
                "r_mail",
                "connector_failure",
                "conn-a",
                "ops@example.com",
                true,
            ),
        ]);
        rules["data"][4]["channel"] = serde_json::json!("email");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rules))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let attempted = evaluate_connector_event(
            &ch,
            &http,
            &email,
            AlertKind::ConnectorFailure,
            "conn-a",
            "Connector failure",
            "text",
            None,
        )
        .await
        .unwrap();
        assert_eq!(attempted.len(), 5);
        let by_id = |id: &str| {
            attempted
                .iter()
                .find(|d| d.rule.id == id)
                .unwrap_or_else(|| panic!("no delivery for {id}"))
        };
        assert!(by_id("r_ok").result.ok);
        assert_eq!(by_id("r_ok").result.error, None);
        for (id, reason) in [
            ("r_500", "the webhook server answered with an error"),
            ("r_down", "the webhook server could not be reached"),
            ("r_bad", "the webhook address was refused"),
            ("r_mail", "e-mail is not set up on this installation"),
        ] {
            let result = &by_id(id).result;
            assert!(!result.ok, "{id} must not read as delivered");
            assert_eq!(result.error.as_deref(), Some(reason), "{id}");
        }
        for d in &attempted {
            let text = d.result.error.clone().unwrap_or_default();
            assert!(
                !text.contains("500") && !text.contains("127.0.0.1") && !text.contains("ftp"),
                "upstream text leaked into {}: {text}",
                d.rule.id
            );
        }
    }

    /// A pipeline kind passed here is not a `SRC-7` kind: nothing is
    /// listed, nothing is delivered.
    #[tokio::test]
    async fn evaluate_connector_event_ignores_a_non_connector_kind() {
        let (_server, ch) = connector_event_server(&[], &[]).await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let delivered = evaluate_connector_event(
            &ch,
            &http,
            &email,
            AlertKind::PipelineFailure,
            "pl-x",
            "x",
            "text",
            None,
        )
        .await
        .unwrap();
        assert!(delivered.is_empty());
    }

    /// `BI-18` part C, part 2: a digest goes to the rule's recipients, not to a
    /// signed-in viewer, so it is read as nobody in particular. A dashboard in
    /// somebody's personal space is "not found" there (the statement carries the
    /// owner restriction, so the store never hands the row back), and so is one
    /// in the trash (`is_deleted = 0`); the digest says so and sends no numbers.
    /// `BI-19` B1 review R6 / owner decision Q-R12: a digest of a restricted
    /// dashboard is skipped with a reason, and nothing is read or sent.
    #[tokio::test]
    async fn a_digest_of_a_restricted_dashboard_is_skipped_with_a_reason() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [], "data": [], "rows": 0,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let mut rule = freshness_rule("t");
        rule.kind = AlertKind::Digest;
        rule.board = Some("b_restricted".to_owned());
        let result = run_one(
            &ch,
            &http,
            &email,
            None,
            (None, None),
            &RestrictedGate,
            &rule,
        )
        .await;
        assert!(!result.fired);
        assert!(
            result
                .skipped
                .as_deref()
                .is_some_and(|r| r.contains("restricted")),
            "{:?}",
            result.skipped
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_digest_of_a_private_or_deleted_dashboard_is_not_found() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let private = serde_json::json!({
            "id": "b_draft", "name": "Secret draft", "owner_id": "u_someone",
            "layout_json": "{}", "filters_json": "[]", "created_at": "2026-10-14 00:00:00"
        });
        // The row comes back only to a statement with no owner restriction.
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.bi_board FINAL"))
            .and(body_string_contains("owner_id = ''"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.bi_board FINAL"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "meta": [], "data": [private], "rows": 1 })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let text = digest_text(&ch, &PassGate, "b_draft").await.unwrap();
        assert_eq!(text, "Dashboard not found.");
        let sent = server.received_requests().await.unwrap();
        let reads: Vec<String> = sent
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|s| s.contains("FROM console.bi_board FINAL WHERE is_deleted = 0"))
            .collect();
        assert_ne!(reads, [] as [std::string::String; 0]);
        assert!(
            reads.iter().all(|r| r.contains("owner_id = ''")),
            "{reads:?}"
        );
        // No chart was listed for it.
        assert!(
            sent.iter()
                .all(|r| !String::from_utf8_lossy(&r.body).contains("FROM console.bi_chart"))
        );
    }

    // ── DATA-14: the quality_incident kind ──────────────────────────────

    fn quality_input(asset: Option<&str>) -> AlertRuleInput {
        AlertRuleInput {
            name: Some("Quality incident".to_owned()),
            kind: Some("quality_incident".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            asset: asset.map(str::to_owned),
            ..AlertRuleInput::default()
        }
    }

    #[test]
    fn the_quality_incident_kind_is_asset_scoped_and_nothing_else_is() {
        assert_eq!(AlertKind::QualityIncident.as_str(), "quality_incident");
        assert_eq!(
            AlertKind::parse_asset_kind("quality_incident"),
            Some(AlertKind::QualityIncident)
        );
        assert_eq!(AlertKind::parse_asset_kind("connector_failure"), None);
        assert!(AlertKind::QualityIncident.is_asset_scoped());
        assert!(!AlertKind::QualityIncident.is_connector_scoped());
        assert!(!AlertKind::ConnectorFailure.is_asset_scoped());
        assert_eq!(
            serde_json::to_string(&AlertKind::QualityIncident).unwrap(),
            "\"quality_incident\""
        );
    }

    #[test]
    fn normalize_input_accepts_a_table_or_star_for_a_quality_incident_rule() {
        let all = normalize_input(&quality_input(Some("*"))).unwrap();
        assert_eq!(all.kind, AlertKind::QualityIncident);
        assert_eq!(all.asset, "*");
        assert_eq!(all.connector, "");
        assert_eq!(all.pipeline, "");
        for (given, stored) in [
            ("silver.orders", "silver.orders"),
            ("  Serving.Mart_Sales ", "serving.mart_sales"),
            ("bronze.raw", "bronze.raw"),
        ] {
            assert_eq!(
                normalize_input(&quality_input(Some(given))).unwrap().asset,
                stored
            );
        }
    }

    #[test]
    fn normalize_input_refuses_a_quality_incident_rule_without_a_valid_table() {
        for asset in [
            None,
            Some(""),
            Some("  "),
            Some("orders"),
            Some("gold.orders"),
            Some("silver."),
            Some("silver.a b"),
            Some("silver.x'; DROP TABLE y"),
            Some("silver.`x`"),
        ] {
            let err = normalize_input(&quality_input(asset)).unwrap_err();
            assert!(matches!(err, AlertError::Validation(_)), "{asset:?}");
            // Fixed text: the input is never echoed.
            assert!(!err.to_string().contains("DROP"), "{err}");
        }
    }

    #[test]
    fn row_to_rule_reads_the_asset_and_an_empty_one_is_none() {
        let mut row = serde_json::Map::new();
        row.insert("id".to_owned(), Value::String("al_q".to_owned()));
        row.insert(
            "type".to_owned(),
            Value::String("quality_incident".to_owned()),
        );
        row.insert(
            "asset".to_owned(),
            Value::String("silver.orders".to_owned()),
        );
        let rule = row_to_rule(&row);
        assert_eq!(rule.kind, AlertKind::QualityIncident);
        assert_eq!(rule.asset.as_deref(), Some("silver.orders"));
        row.insert("asset".to_owned(), Value::String(String::new()));
        assert_eq!(row_to_rule(&row).asset, None);
    }

    #[tokio::test]
    async fn ensure_sends_an_additive_asset_column_statement() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("ADD COLUMN IF NOT EXISTS asset"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        ensure(&ch).await.unwrap();
    }

    /// `(id, asset, target, enabled)` quality incident rules, plus one rule of
    /// another kind, every field `row_to_rule` reads populated.
    fn quality_rules_json(rows: &[(&str, &str, &str, bool)]) -> serde_json::Value {
        let mut data: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, asset, target, enabled)| {
                serde_json::json!({
                    "id": id, "name": format!("Rule {id}"), "type": "quality_incident",
                    "mart": "", "measure": "", "agg": "", "op": "", "threshold": "0",
                    "board": "", "channel": "webhook", "target": target,
                    "enabled": if *enabled { "1" } else { "0" },
                    "created_at": "", "severity": "", "pipeline": "",
                    "connector": "", "asset": asset,
                })
            })
            .collect();
        data.push(serde_json::json!({
            "id": "other-kind", "name": "Rule other", "type": "connector_failure",
            "mart": "", "measure": "", "agg": "", "op": "", "threshold": "0",
            "board": "", "channel": "webhook", "target": "http://127.0.0.1:1/x",
            "enabled": "1", "created_at": "", "severity": "", "pipeline": "",
            "connector": "*", "asset": "*",
        }));
        serde_json::json!({ "meta": [], "data": data, "rows": data.len() })
    }

    /// A rule for one table delivers for that table (whatever the case the
    /// caller names it in); `*` delivers for every table; another table, a
    /// disabled rule and another kind never deliver.
    #[tokio::test]
    async fn deliver_quality_incident_matches_its_table_star_and_kind_only() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for (id, times) in [("one", 1_u64), ("other", 0), ("all", 1), ("off", 0)] {
            Mock::given(method("POST"))
                .and(path(format!("/hooks/{id}")))
                .respond_with(ResponseTemplate::new(200))
                .expect(times)
                .mount(&server)
                .await;
        }
        let rows = [
            ("one", "silver.orders", true),
            ("other", "silver.customers", true),
            ("all", "*", true),
            ("off", "silver.orders", false),
        ]
        .map(|(id, asset, enabled)| (id, asset, format!("{}/hooks/{id}", server.uri()), enabled));
        let refs: Vec<(&str, &str, &str, bool)> = rows
            .iter()
            .map(|(id, asset, target, enabled)| (*id, *asset, target.as_str(), *enabled))
            .collect();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(quality_rules_json(&refs)))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let rules = load_rules(&ch).await.unwrap();

        let delivered = deliver_quality_incident_to(
            &rules,
            &test_webhook_sender(),
            &no_smtp_email_sender(),
            "Silver.Orders",
            "Quality incident",
            "silver.orders: email_filled — 75.0% not null (threshold: email not null >= 90%)",
            None,
        )
        .await;
        let mut ids: Vec<&str> = delivered.iter().map(|d| d.rule.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["all", "one"]);
        assert!(delivered.iter().all(|d| d.result.ok));
    }

    /// The scope reaches `ClickHouse` as an escaped literal.
    #[tokio::test]
    async fn save_rule_sends_the_asset_scope_as_a_quoted_literal() {
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("INSERT INTO console.alert_rule"))
            .and(body_string_contains("'quality_incident'"))
            .and(body_string_contains("'silver.orders'"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(quality_rules_json(&[(
                    "al_q1",
                    "silver.orders",
                    "https://hooks.example.com/x",
                    true,
                )])),
            )
            .mount(&server)
            .await;
        // `DATA-14` (CI failure): the target is a name, so the sender needs a
        // resolver that approves it without dialling; the bare sender resolved
        // the real host in CI.
        let webhooks = lakehouse_notify::MappedResolver::new()
            .with("hooks.example.com", "127.0.0.1:9".parse().unwrap())
            .into_sender();
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let saved = save_rule(
            &ch,
            &webhooks,
            &quality_input(Some("silver.orders")),
            Some("al_q1"),
        )
        .await
        .unwrap();
        assert_eq!(saved.kind, AlertKind::QualityIncident);
    }

    // ── AlertKind::PipelineBehindTarget (BLD-2 plan D9) ──────────────────
    // BLD-2 review R3 BLOCKER 1 / decision R14: alert includes reason when provided
    #[tokio::test]
    async fn evaluate_pipeline_behind_target_delivers_a_message_naming_reason_when_provided() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/behind-reason", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/behind-reason"))
            .and(body_string_contains(
                "Pipeline pl-orders is behind its target lag (lag: 30s, target: 600s, reason: the last 2 runs for this change failed; the next input change starts again)",
            ))
            .and(body_string_contains("View: /pipelines/pl-orders"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let rule_json = serde_json::json!({
            "meta": [],
            "data": [{
                "id": "al_pbt_r",
                "name": "Orders lag alert with reason",
                "type": "pipeline_behind_target",
                "mart": "",
                "measure": "",
                "agg": "",
                "op": "",
                "threshold": "0",
                "board": "",
                "channel": "webhook",
                "target": webhook,
                "enabled": "1",
                "created_at": "",
                "severity": "",
                "pipeline": "pl-orders",
                "connector": ""
            }]
        });

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_json))
            .mount(&server)
            .await;

        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_behind_target(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(30),
            Some(600),
            Some("the last 2 runs for this change failed; the next input change starts again"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);
    }

    #[test]
    fn pipeline_behind_target_kind_as_str_round_trips() {
        assert_eq!(
            AlertKind::PipelineBehindTarget.as_str(),
            "pipeline_behind_target"
        );
    }

    #[test]
    fn row_to_rule_maps_pipeline_behind_target_type_and_pipeline_column() {
        let mut row = Map::new();
        row.insert("id".to_owned(), Value::String("al_pbt1".to_owned()));
        row.insert("name".to_owned(), Value::String("Behind target".to_owned()));
        row.insert(
            "type".to_owned(),
            Value::String("pipeline_behind_target".to_owned()),
        );
        row.insert("mart".to_owned(), Value::String(String::new()));
        row.insert("measure".to_owned(), Value::String(String::new()));
        row.insert("agg".to_owned(), Value::String(String::new()));
        row.insert("op".to_owned(), Value::String(String::new()));
        row.insert("threshold".to_owned(), Value::String("0".to_owned()));
        row.insert("board".to_owned(), Value::String(String::new()));
        row.insert("channel".to_owned(), Value::String("webhook".to_owned()));
        row.insert("target".to_owned(), Value::String("t".to_owned()));
        row.insert("enabled".to_owned(), Value::String("1".to_owned()));
        row.insert("created_at".to_owned(), Value::String(String::new()));
        row.insert("severity".to_owned(), Value::String(String::new()));
        row.insert("pipeline".to_owned(), Value::String("pl-orders".to_owned()));

        let rule = row_to_rule(&row);
        assert_eq!(rule.kind, AlertKind::PipelineBehindTarget);
        assert_eq!(rule.pipeline.as_deref(), Some("pl-orders"));
    }

    #[test]
    fn normalize_input_accepts_a_pipeline_behind_target_rule_with_star() {
        let input = AlertRuleInput {
            name: Some("Any pipeline behind target".to_owned()),
            kind: Some("pipeline_behind_target".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: Some("*".to_owned()),
            ..AlertRuleInput::default()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::PipelineBehindTarget);
        assert_eq!(normalized.pipeline, "*");
    }

    #[test]
    fn normalize_input_accepts_a_pipeline_behind_target_rule_with_pipeline_id() {
        let input = AlertRuleInput {
            name: Some("Orders behind target".to_owned()),
            kind: Some("pipeline_behind_target".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: Some("pl-orders".to_owned()),
            ..AlertRuleInput::default()
        };
        let normalized = normalize_input(&input).unwrap();
        assert_eq!(normalized.kind, AlertKind::PipelineBehindTarget);
        assert_eq!(normalized.pipeline, "pl-orders");
    }

    #[test]
    fn normalize_input_rejects_a_pipeline_behind_target_rule_with_no_pipeline() {
        let input = AlertRuleInput {
            name: Some("Orders behind target".to_owned()),
            kind: Some("pipeline_behind_target".to_owned()),
            target: Some("https://hooks.example.com/x".to_owned()),
            channel: Some("webhook".to_owned()),
            pipeline: None,
            ..AlertRuleInput::default()
        };
        let err = normalize_input(&input).unwrap_err();
        assert!(
            err.to_string()
                .contains("pipeline_behind_target requires a pipeline id or"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn evaluate_pipeline_behind_target_delivers_a_message_naming_pipeline_and_lag() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/behind", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/behind"))
            .and(body_string_contains(
                "Pipeline pl-orders is behind its target lag (lag: 300s, target: 120s)",
            ))
            .and(body_string_contains("View: /pipelines/pl-orders"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let rule_json = serde_json::json!({
            "meta": [],
            "data": [{
                "id": "al_pbt1",
                "name": "Orders lag alert",
                "type": "pipeline_behind_target",
                "mart": "",
                "measure": "",
                "agg": "",
                "op": "",
                "threshold": "0",
                "board": "",
                "channel": "webhook",
                "target": webhook,
                "enabled": "1",
                "created_at": "",
                "severity": "",
                "pipeline": "pl-orders",
                "connector": ""
            }]
        });

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_json))
            .mount(&server)
            .await;

        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_behind_target(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(300),
            Some(120),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(matched, 1);
    }

    #[tokio::test]
    async fn evaluate_pipeline_behind_target_does_not_deliver_a_silenced_rule() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let webhook = format!("{}/hooks/behind_silenced", server.uri());
        Mock::given(method("POST"))
            .and(path("/hooks/behind_silenced"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let rule_json = serde_json::json!({
            "meta": [],
            "data": [{
                "id": "al_pbt_quiet",
                "name": "Silenced lag alert",
                "type": "pipeline_behind_target",
                "mart": "",
                "measure": "",
                "agg": "",
                "op": "",
                "threshold": "0",
                "board": "",
                "channel": "webhook",
                "target": webhook,
                "enabled": "1",
                "created_at": "",
                "severity": "",
                "pipeline": "pl-orders",
                "connector": ""
            }]
        });

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_json))
            .mount(&server)
            .await;

        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let matched = evaluate_pipeline_behind_target(
            &ch,
            &http,
            &email,
            "pl-orders",
            Some(300),
            Some(120),
            None,
            Some(&AlwaysSilenced),
        )
        .await
        .unwrap();
        assert_eq!(matched, 0, "silenced rule must not be delivered");
    }

    // ── BI-19 part C, T11: alerts that fire once (D11) ──────────────────

    /// The whole transition table of D11 / feature C2.
    #[test]
    fn the_transition_table_sends_once_on_crossing_and_once_on_recovery() {
        use AlertState::{Firing, Ok, Stopped};
        use Sending::{Crossed, Nothing, Recovered};
        for (before, crossed, after, sends) in [
            (Ok, true, Firing, Crossed),
            (Ok, false, Ok, Nothing),
            (Firing, true, Firing, Nothing),
            (Firing, false, Ok, Recovered),
            (Stopped, true, Firing, Crossed),
            (Stopped, false, Ok, Nothing),
        ] {
            assert_eq!(
                transition(before, crossed),
                (after, sends),
                "{before:?} with crossed={crossed}"
            );
        }
    }

    #[test]
    fn a_stored_state_word_that_is_not_known_reads_as_ok_and_never_as_silence() {
        for s in [AlertState::Ok, AlertState::Firing, AlertState::Stopped] {
            assert_eq!(AlertState::parse(s.as_str()), s);
        }
        assert_eq!(AlertState::parse(""), AlertState::Ok);
        assert_eq!(AlertState::parse("garbage"), AlertState::Ok);
    }

    /// A state source in memory that counts its writes.
    #[derive(Default)]
    struct FakeState {
        states: std::sync::Mutex<std::collections::HashMap<String, AlertState>>,
        failed: std::sync::Mutex<Vec<String>>,
        unreadable: bool,
        /// When set, `state_of` answers this whatever is stored: a runner that
        /// read the state before another runner claimed the transition.
        stale: Option<AlertState>,
    }

    #[async_trait::async_trait]
    impl StateSource for FakeState {
        async fn state_of(&self, rule_id: &str) -> Result<AlertState, String> {
            if self.unreadable {
                return Err("the alert's state could not be read.".to_owned());
            }
            if let Some(stale) = self.stale {
                return Ok(stale);
            }
            Ok(self
                .states
                .lock()
                .unwrap()
                .get(rule_id)
                .copied()
                .unwrap_or(AlertState::Ok))
        }
        async fn record(
            &self,
            rule_id: &str,
            state: AlertState,
            _value: Option<f64>,
            _reason: Option<&str>,
        ) {
            self.states
                .lock()
                .unwrap()
                .insert(rule_id.to_owned(), state);
        }
        async fn record_failed(&self, _rule_id: &str, reason: &str) {
            self.failed.lock().unwrap().push(reason.to_owned());
        }
        async fn claim(
            &self,
            rule_id: &str,
            previous: AlertState,
            next: AlertState,
            _value: Option<f64>,
        ) -> Result<bool, String> {
            let mut states = self.states.lock().unwrap();
            let current = states.get(rule_id).copied().unwrap_or(AlertState::Ok);
            if current != previous {
                return Ok(false);
            }
            states.insert(rule_id.to_owned(), next);
            Ok(true)
        }
        async fn release(&self, rule_id: &str, claimed: AlertState, previous: AlertState) {
            let mut states = self.states.lock().unwrap();
            if states.get(rule_id).copied() == Some(claimed) {
                states.insert(rule_id.to_owned(), previous);
            }
        }
    }

    /// A mart rule `sum(amount) > 100` whose webhook target is `hook`.
    fn legacy_rule(hook: &str) -> AlertRule {
        let mut rule = freshness_rule("t");
        rule.kind = AlertKind::Alert;
        rule.mart = Some("mart_x".to_owned());
        rule.measure = Some("amount".to_owned());
        rule.op = AlertOp::Gt;
        rule.threshold = 100.0;
        rule.channel = AlertChannel::Webhook;
        rule.target = format!("{hook}/hook");
        rule
    }

    async fn engine_answering(value: f64) -> wiremock::MockServer {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "v", "type": "Float64"}],
                "data": [{"v": value}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        server
    }

    async fn hook_receiving() -> wiremock::MockServer {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let hook = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&hook)
            .await;
        hook
    }

    /// `BI-19` part C, T11: a rule that was over the line on three checks in a
    /// row sends once, not three times (the spec's "re-fires every check").
    #[tokio::test]
    async fn a_legacy_rule_over_the_line_on_three_checks_sends_once() {
        let engine = engine_answering(150.0).await;
        let hook = hook_receiving().await;
        let ch = ChClient::new(engine.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let state = FakeState::default();
        let rule = legacy_rule(&hook.uri());
        let mut fired = 0;
        for _ in 0..3 {
            let r = run_one(
                &ch,
                &http,
                &email,
                None,
                (None, Some(&state)),
                &PassGate,
                &rule,
            )
            .await;
            assert_eq!(r.skipped, None);
            assert_eq!(r.value, Some(150.0));
            fired += usize::from(r.fired);
        }
        assert_eq!(fired, 1);
        assert_eq!(hook.received_requests().await.unwrap().len(), 1);
        assert_eq!(state.states.lock().unwrap()["al_1"], AlertState::Firing);
    }

    /// And once more when it is back: firing, firing, back, back is two messages.
    #[tokio::test]
    async fn the_value_coming_back_sends_one_recovery_and_then_nothing() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let hook = hook_receiving().await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let state = FakeState::default();
        let rule = legacy_rule(&hook.uri());
        let mut recoveries = 0;
        for value in [150.0, 160.0, 40.0, 30.0] {
            let engine = MockServer::start().await;
            Mock::given(wiremock::matchers::any())
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "meta": [{"name": "v", "type": "Float64"}],
                    "data": [{"v": value}],
                    "rows": 1,
                })))
                .mount(&engine)
                .await;
            let ch = ChClient::new(engine.uri(), "default".to_owned(), String::new());
            let r = run_one(
                &ch,
                &http,
                &email,
                None,
                (None, Some(&state)),
                &PassGate,
                &rule,
            )
            .await;
            recoveries += usize::from(r.recovered);
            assert!(!(r.fired && r.recovered));
        }
        assert_eq!(recoveries, 1);
        let bodies: Vec<String> = hook
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        assert!(bodies[0].contains("Alert:"), "{}", bodies[0]);
        assert!(bodies[1].contains("Back to normal"), "{}", bodies[1]);
        assert_eq!(state.states.lock().unwrap()["al_1"], AlertState::Ok);
    }

    /// A check that errors changes nothing: the rule stays `firing`, so the check
    /// after it does not send the crossing a second time.
    #[tokio::test]
    async fn a_check_that_errors_records_itself_and_changes_nothing() {
        let hook = hook_receiving().await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let state = FakeState::default();
        state
            .states
            .lock()
            .unwrap()
            .insert("al_1".to_owned(), AlertState::Firing);
        let down = ChClient::new(
            "http://127.0.0.1:1".to_owned(),
            "default".to_owned(),
            String::new(),
        );
        let rule = legacy_rule(&hook.uri());
        let r = run_one(
            &down,
            &http,
            &email,
            None,
            (None, Some(&state)),
            &PassGate,
            &rule,
        )
        .await;
        assert_eq!(r.skipped.as_deref(), Some("the value could not be read."));
        assert!(!r.fired && !r.recovered);
        assert_eq!(state.states.lock().unwrap()["al_1"], AlertState::Firing);
        assert_eq!(
            *state.failed.lock().unwrap(),
            ["the value could not be read."]
        );
        assert!(hook.received_requests().await.unwrap().is_empty());
        // Still over the line afterwards: nothing is sent, it was already told.
        let engine = engine_answering(150.0).await;
        let ch = ChClient::new(engine.uri(), "default".to_owned(), String::new());
        let r = run_one(
            &ch,
            &http,
            &email,
            None,
            (None, Some(&state)),
            &PassGate,
            &rule,
        )
        .await;
        assert!(!r.fired);
        assert!(hook.received_requests().await.unwrap().is_empty());
    }

    /// Fail closed: with no way to read the state the rule is not checked, since
    /// it would send on every check.
    #[tokio::test]
    async fn a_threshold_rule_with_no_readable_state_is_skipped_not_sent() {
        let engine = engine_answering(150.0).await;
        let hook = hook_receiving().await;
        let ch = ChClient::new(engine.uri(), "default".to_owned(), String::new());
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let rule = legacy_rule(&hook.uri());
        let none = run_one(&ch, &http, &email, None, (None, None), &PassGate, &rule).await;
        assert_eq!(none.skipped.as_deref(), Some(STATE_UNAVAILABLE));
        let broken = FakeState {
            unreadable: true,
            ..FakeState::default()
        };
        let r = run_one(
            &ch,
            &http,
            &email,
            None,
            (None, Some(&broken)),
            &PassGate,
            &rule,
        )
        .await;
        assert_eq!(
            r.skipped.as_deref(),
            Some("the alert's state could not be read.")
        );
        assert!(hook.received_requests().await.unwrap().is_empty());
    }

    /// A crossing the webhook refused is not lost: the state stays `ok`, so the
    /// next check sends it again.
    #[tokio::test]
    async fn a_crossing_that_could_not_be_delivered_is_sent_again_at_the_next_check() {
        let engine = engine_answering(150.0).await;
        let ch = ChClient::new(engine.uri(), "default".to_owned(), String::new());
        let http = WebhookSender::new(std::sync::Arc::new(RefuseAll));
        let email = no_smtp_email_sender();
        let state = FakeState::default();
        let rule = legacy_rule("http://hooks.test");
        let r = run_one(
            &ch,
            &http,
            &email,
            None,
            (None, Some(&state)),
            &PassGate,
            &rule,
        )
        .await;
        assert!(r.fired);
        assert!(r.delivered.is_some_and(|d| !d.ok));
        // BI-19 C review SF-1: the claim was given back, so the rule reads as
        // before (`ok`) and the next check sends again.
        assert_eq!(
            state.states.lock().unwrap().get("al_1").copied(),
            Some(AlertState::Ok)
        );
        assert_eq!(*state.failed.lock().unwrap(), [NOT_DELIVERED_RETRY]);
    }

    /// `BI-19` C review SF-1: two runners that read the same state before either
    /// wrote it send once. The second one read `ok` after the first had already
    /// claimed `firing`, so its claim loses and it sends nothing.
    #[tokio::test]
    async fn two_overlapping_checks_of_one_crossing_send_once() {
        let hook = hook_receiving().await;
        let http = test_webhook_sender();
        let email = no_smtp_email_sender();
        let rule = legacy_rule(&hook.uri());
        let reading = || Reading::against_threshold(&rule, "x", 150.0);
        let state = FakeState::default();
        let first = settle((&http, &email), None, Some(&state), &rule, Ok(reading())).await;
        assert!(first.fired, "{first:?}");
        let late_reader = FakeState {
            stale: Some(AlertState::Ok),
            states: std::sync::Mutex::new(state.states.lock().unwrap().clone()),
            ..FakeState::default()
        };
        let second = settle(
            (&http, &email),
            None,
            Some(&late_reader),
            &rule,
            Ok(reading()),
        )
        .await;
        assert!(!second.fired && second.delivered.is_none(), "{second:?}");
        assert_eq!(second.skipped.as_deref(), Some(CLAIMED_ELSEWHERE));
        assert_eq!(hook.received_requests().await.unwrap().len(), 1);
    }

    /// `BI-19` C review SF-3: only a change of what is watched drops the state.
    #[test]
    fn editing_what_a_rule_watches_differs_and_editing_its_name_does_not() {
        let old = legacy_rule("http://hooks.test");
        let mut renamed = old.clone();
        renamed.name = "other".to_owned();
        renamed.target = "http://hooks.test/other".to_owned();
        renamed.enabled = !old.enabled;
        assert!(!watches_differ(&old, &renamed));
        for change in [
            |r: &mut AlertRule| r.threshold = 5.0,
            |r: &mut AlertRule| r.op = AlertOp::Lt,
            |r: &mut AlertRule| r.measure = Some("other".to_owned()),
            |r: &mut AlertRule| r.chart = Some("c_1".to_owned()),
            |r: &mut AlertRule| r.condition = Some("any_rows".to_owned()),
            |r: &mut AlertRule| r.owner_id = Some("u".to_owned()),
        ] {
            let mut changed = old.clone();
            change(&mut changed);
            assert!(watches_differ(&old, &changed));
        }
    }

    /// `ensure` widens the table with additive statements only (D11): every new
    /// column is `ADD COLUMN IF NOT EXISTS`, and nothing is dropped or modified.
    #[tokio::test]
    async fn ensure_sends_only_additive_alters_for_the_part_c_columns() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        ensure(&ch).await.unwrap();
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        for column in ["chart", "series", "filters_json", "condition", "owner_id"] {
            assert!(
                sent.iter().any(|q| q.contains(&format!(
                    "ALTER TABLE console.alert_rule ADD COLUMN IF NOT EXISTS {column} String DEFAULT ''"
                ))),
                "no additive statement for {column}"
            );
        }
        for q in sent.iter().filter(|q| q.contains("ALTER")) {
            assert!(q.contains("ADD COLUMN IF NOT EXISTS"), "{q}");
            assert!(!q.contains("DROP") && !q.contains("MODIFY"), "{q}");
        }
    }

    // ── BI-19 part C, T12: chart alerts (D12) ───────────────────────────

    fn chart_input(condition: &str) -> AlertRuleInput {
        AlertRuleInput {
            name: Some("Visitors".to_owned()),
            target: Some("ops@example.com".to_owned()),
            channel: Some("email".to_owned()),
            chart: Some("c_one".to_owned()),
            condition: Some(condition.to_owned()),
            threshold: Some(100.0),
            ..AlertRuleInput::default()
        }
    }

    #[test]
    fn a_chart_rule_needs_a_known_condition_and_derives_its_operator() {
        let above = normalize_input(&chart_input("above")).unwrap();
        assert_eq!((above.op, above.threshold), (AlertOp::Gt, 100.0));
        assert!(above.mart.is_empty() && above.metric.is_empty());
        let below = normalize_input(&chart_input("below")).unwrap();
        assert_eq!(below.op, AlertOp::Lt);
        let any = normalize_input(&chart_input("any_rows")).unwrap();
        assert_eq!(any.threshold, 0.0);
        for bad in ["", "equal", "ABOVE"] {
            let err = normalize_input(&chart_input(bad)).unwrap_err();
            assert_eq!(
                err.to_string(),
                "condition must be above, below or any_rows."
            );
        }
    }

    #[test]
    fn a_chart_rule_refuses_a_chart_id_series_threshold_or_filter_that_is_malformed() {
        let mut input = chart_input("above");
        input.chart = Some("c' OR 1=1".to_owned());
        assert_eq!(
            normalize_input(&input).unwrap_err().to_string(),
            "invalid chart."
        );
        let mut input = chart_input("above");
        input.threshold = Some(f64::NAN);
        assert_eq!(
            normalize_input(&input).unwrap_err().to_string(),
            "invalid threshold."
        );
        let mut input = chart_input("above");
        input.series = Some("a\nb".to_owned());
        assert_eq!(
            normalize_input(&input).unwrap_err().to_string(),
            "invalid series."
        );
        let mut input = chart_input("above");
        input.filters = Some(vec![lakehouse_bi::filters::FilterDef {
            column: String::new(),
            ..serde_json::from_str(r#"{"column":"x","values":["a"]}"#).unwrap()
        }]);
        assert!(normalize_input(&input).is_err());
    }

    #[test]
    fn extras_keep_a_chart_rules_fields_and_only_the_owner_of_any_other_rule() {
        let mut input = chart_input("above");
        input.series = Some("visitors".to_owned());
        input.owner_id = Some("u1".to_owned());
        input.filters = Some(vec![
            serde_json::from_str(r#"{"column":"region","values":["Bali"]}"#).unwrap(),
        ]);
        let extras = RuleExtras::of(&input, AlertKind::Alert);
        assert_eq!(
            (
                extras.chart.as_str(),
                extras.series.as_str(),
                extras.condition.as_str()
            ),
            ("c_one", "visitors", "above")
        );
        assert!(extras.filters_json.contains("Bali"));
        let other = RuleExtras::of(&input, AlertKind::Digest);
        assert!(other.chart.is_empty() && other.filters_json.is_empty());
        assert_eq!(other.owner_id, "u1");
        input.condition = Some("any_rows".to_owned());
        assert_eq!(RuleExtras::of(&input, AlertKind::Alert).series, "");
    }

    #[test]
    fn a_chart_number_is_the_first_value_of_a_kpi_and_the_latest_point_of_a_series() {
        use serde_json::json;
        let cols = vec!["month".to_owned(), "visitors".to_owned()];
        let rows = vec![
            json!({"month": "2026-01", "visitors": 10}),
            json!({"month": "2026-02", "visitors": "25.5"}),
        ];
        assert_eq!(
            chart_number(NumberShape::Series, Some("visitors"), &cols, &rows),
            Ok(25.5)
        );
        assert_eq!(
            chart_number(NumberShape::Series, None, &cols, &rows),
            Err("the series is not in the chart's result.".to_owned())
        );
        let kpi = vec![json!({"total": 7})];
        assert_eq!(
            chart_number(NumberShape::Single, None, &["total".to_owned()], &kpi),
            Ok(7.0)
        );
        assert_eq!(
            chart_number(NumberShape::Single, None, &["total".to_owned()], &[]),
            Err("the chart returned no value.".to_owned())
        );
        let text = vec![json!({"total": "n/a"}), json!({"total": null})];
        for row in &text {
            assert_eq!(
                chart_number(
                    NumberShape::Single,
                    None,
                    &["total".to_owned()],
                    std::slice::from_ref(row)
                ),
                Err("the chart's value is not a number.".to_owned())
            );
        }
        assert!(chart_number(NumberShape::NoNumber, None, &cols, &rows).is_err());
    }

    #[test]
    fn only_kpi_gauge_and_ordered_series_kinds_give_a_number() {
        use lakehouse_bi::specs::ChartKind;
        assert_eq!(number_shape(ChartKind::Kpi), NumberShape::Single);
        assert_eq!(number_shape(ChartKind::Gauge), NumberShape::Single);
        assert_eq!(number_shape(ChartKind::Line), NumberShape::Series);
        assert_eq!(number_shape(ChartKind::Pie), NumberShape::NoNumber);
        assert_eq!(number_shape(ChartKind::Heatmap), NumberShape::NoNumber);
    }

    fn charts_json(kind: &str, order: &str) -> serde_json::Value {
        let spec = serde_json::json!({
            "id": "c_one", "title": "T", "kind": kind, "mart": "mart_x", "sql": "SELECT 1",
            "x": "month", "y": "visitors"
        });
        let def = serde_json::json!({
            "title": "T", "mart": "mart_x", "kind": kind, "dimension": "month",
            "measures": ["visitors"], "order": order
        });
        serde_json::json!({
            "meta": [], "rows": 1,
            "data": [{
                "id": "c_one", "board": "b1", "created_by": "ui",
                "created_at": "2026-01-01 00:00:00",
                "spec_json": serde_json::json!({"spec": spec, "def": def, "hasYear": false}).to_string()
            }]
        })
    }

    async fn save_chart_rule(
        kind: &str,
        order: &str,
        condition: &str,
        series: Option<&str>,
    ) -> Result<AlertRule, AlertError> {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::body_string_contains(
            "FROM console.bi_chart",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(charts_json(kind, order)))
        .mount(&server)
        .await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [], "data": [], "rows": 0
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut input = chart_input(condition);
        input.series = series.map(str::to_owned);
        save_rule(&ch, &test_webhook_sender(), &input, None).await
    }

    /// D12: a chart with no single number refuses "above" and keeps "`any_rows`".
    #[tokio::test]
    async fn a_chart_with_no_number_refuses_above_and_allows_any_rows() {
        let err = save_chart_rule("pie", "none", "above", None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("can only be 'returns any rows'"),
            "{err}"
        );
        // The stored row is empty in this fake, so the save fails after the check
        // with the "could not be found" sentence; what matters is the check passed.
        let ok = save_chart_rule("pie", "none", "any_rows", None).await;
        assert!(
            ok.as_ref()
                .err()
                .is_none_or(|e| e.to_string().contains("could not be found")),
            "{ok:?}"
        );
    }

    #[tokio::test]
    async fn a_series_chart_needs_a_series_and_an_order_by_its_dimension() {
        let err = save_chart_rule("line", "none", "above", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("choose the series"), "{err}");
        let err = save_chart_rule("line", "desc", "above", Some("visitors"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("sorted by value"), "{err}");
        let kpi = save_chart_rule("kpi", "none", "above", None).await;
        assert!(
            kpi.as_ref()
                .err()
                .is_none_or(|e| e.to_string().contains("could not be found")),
            "{kpi:?}"
        );
    }

    #[tokio::test]
    async fn a_chart_that_does_not_exist_is_refused() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [], "data": [], "rows": 0
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let err = save_rule(&ch, &test_webhook_sender(), &chart_input("above"), None)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "no chart has that id.");
    }

    /// D12: `run_rules` leaves a chart rule to the API, with the reason.
    #[tokio::test]
    async fn run_one_skips_a_chart_rule_and_says_why() {
        let state = FakeState::default();
        let ch = ChClient::new(
            "http://127.0.0.1:1".to_owned(),
            "default".to_owned(),
            String::new(),
        );
        let mut rule = legacy_rule("http://127.0.0.1:9");
        rule.chart = Some("c_one".to_owned());
        let r = run_one(
            &ch,
            &test_webhook_sender(),
            &no_smtp_email_sender(),
            None,
            (None, Some(&state)),
            &PassGate,
            &rule,
        )
        .await;
        assert_eq!(r.skipped.as_deref(), Some(CHART_SKIPPED));
        assert!(!r.fired);
    }

    /// C4: an owner who lost access stops the rule; nothing is sent, the reason
    /// is recorded, and the next healthy check judges it afresh.
    #[tokio::test]
    async fn a_stopped_rule_records_its_reason_and_is_judged_afresh_when_it_runs_again() {
        let state = FakeState::default();
        let rule = legacy_rule("http://127.0.0.1:9");
        let r = stop(
            Some(&state),
            &rule,
            "The person who set this alert is gone.",
        )
        .await;
        assert_eq!(state.states.lock().unwrap()["al_1"], AlertState::Stopped);
        assert_eq!(
            r.skipped.as_deref(),
            Some("The person who set this alert is gone.")
        );
        assert_eq!(
            transition(AlertState::Stopped, true),
            (AlertState::Firing, Sending::Crossed)
        );
    }
}
