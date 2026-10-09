//! `GET/POST/PUT/DELETE /api/alerts`, `GET/POST /api/alerts/run` — threshold
//! alerts & scheduled digests.
//!
//! Ports `src/app/api/alerts/route.ts` and
//! `src/app/api/alerts/run/route.ts`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_alerts::{
    AlertKind, AlertRule, AlertRuleInput, FreshnessSource, LateSource, SilenceSource, SqlGate,
};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_iceberg::IcebergClient;
use lakehouse_notify::{EmailSender, SmtpConfig};
use lakehouse_store::PgPool;
use lakehouse_store::overview::{self, FiredRule};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::config::Config;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// `GET /api/alerts` — list the alert & digest rules the caller may see.
///
/// The body is `{"rules": [...], "runEventsConfigured": bool}`. Rules of
/// the pre-`SRC-7` kinds are returned exactly as before; rules of the six
/// `SRC-7` kinds are filtered by [`visible_rules`] (`SRC-7` review BLOCKER 1:
/// the list is one for the installation, and such a rule names a connector
/// and carries a webhook target). The body also carries
/// `runEventsConfigured` (`SRC-7` D9), true when this API has
/// `PIPELINE_RUN_TOKEN` set.
///
/// The `TypeScript` handler's `catch` returns a 500 with `e.message`
/// (`alerts/route.ts`'s `GET`), unlike `POST`/`PUT` which return 400 for
/// the same kind of failure — the status code here depends on which route
/// caught the error, not on the error's own type, so it is chosen at each
/// call site rather than baked into a single `From` conversion.
///
/// # Errors
///
/// Returns a 500 [`ApiError::Internal`] on a `ClickHouse` failure; for the
/// `SRC-7` kinds, whatever [`visible_rules`] returns.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let rules = lakehouse_alerts::list_rules(&state.clickhouse)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let rules = visible_rules(&state, principal.as_ref().map(|Extension(p)| p), rules).await?;
    Ok(ApiJson(json!({
        "rules": rules,
        "runEventsConfigured": state.config.pipeline_run_token.is_some(),
    })))
}

/// The rules of `rules` that `principal` may see (`SRC-7` review BLOCKER 1).
///
/// Rules of the pre-`SRC-7` kinds pass untouched. For the six new kinds an
/// unrestricted caller ([`crate::routes::catalog::is_unrestricted`]) sees
/// all; anyone else sees only the rules scoped to a connector of their
/// tenants. A `*` rule, an `upload_failure` rule and a rule whose connector
/// no longer exists (or has no tenant) are hidden from them, as the save
/// check refuses the same scopes. Membership is decided for all connectors
/// in one query, not one per rule.
///
/// # Errors
///
/// 401 with no principal when a rule of the new kinds exists; 503 if no
/// pool is configured; 500 on a database failure.
pub(crate) async fn visible_rules(
    state: &AppState,
    principal: Option<&Principal>,
    rules: Vec<AlertRule>,
) -> Result<Vec<AlertRule>, ApiError> {
    if !rules.iter().any(|rule| rule.kind.is_connector_scoped()) {
        return Ok(rules);
    }
    let Some(principal) = principal else {
        return Err(ApiError::unauthorized());
    };
    if crate::routes::catalog::is_unrestricted(principal) {
        return Ok(rules);
    }
    let mut ids: Vec<String> = rules
        .iter()
        .filter(|rule| rule.kind.is_connector_scoped())
        .filter_map(|rule| rule.connector.clone())
        .filter(|connector| connector != "*")
        .collect();
    ids.sort();
    ids.dedup();
    let own: std::collections::HashSet<String> = if ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        let pool = state
            .pg
            .as_deref()
            .ok_or_else(|| ApiError::Unavailable("connector store unavailable".to_owned()))?;
        lakehouse_store::connectors::connector_ids_in_tenants(pool, &ids, &principal.tenant_ids)
            .await?
            .into_iter()
            .collect()
    };
    Ok(rules
        .into_iter()
        .filter(|rule| {
            !rule.kind.is_connector_scoped()
                || rule
                    .connector
                    .as_deref()
                    .is_some_and(|connector| own.contains(connector))
        })
        .collect())
}

/// Parse the raw request body as JSON into an [`AlertRuleInput`].
///
/// The `TypeScript` handler's `catch` around `await req.json()` swallows
/// *any* failure from that expression — a genuinely malformed body, but
/// also (irrelevantly here) a body read error — and reports it at 400 with
/// `e.message`. Bun's JSON parser produces a distinctive message (e.g.
/// `JSON Parse error: Unexpected identifier "not"`) that `serde_json`
/// cannot reproduce; see `rust/tests/parity/README.md`'s "Known
/// non-deterministic captures" section, which already documents this exact
/// pair of routes (`alerts-create-bad-body`,
/// `dashboard-boards-create-bad-body`) as un-portable runtime error text
/// and normalizes the `error` field for them in the parity harness. This
/// produces a sensible equivalent — a 400 naming the parse failure — rather
/// than contorting the parser to chase Bun's exact wording.
fn parse_body(body: &Bytes) -> Result<AlertRuleInput, ApiError> {
    serde_json::from_slice(body)
        .map_err(|err| ApiError::BadRequest(format!("JSON is invalid: {err}")))
}

/// The refusal for a rule scoped to every connector or to uploads, by
/// someone who does not see every tenant (`SRC-7` D7).
const ALL_SCOPE_DENIED: &str =
    "only an administrator who sees every tenant can save a rule for all connectors or for uploads";

/// Who may save a rule of the `SRC-7` kinds (decision D7, finding F7). Rules
/// are one list for the installation, so a rule for all connectors, or for
/// uploads, would carry every tenant's connector and file names:
///
/// - scope `"*"` (and every `upload_failure`, which is always `"*"`) needs
///   [`crate::routes::catalog::is_unrestricted`];
/// - a connector id must pass
///   [`crate::routes::connectors::ensure_connector_in_tenants`], which
///   answers a connector outside the caller's tenants exactly as an unknown
///   one (404), so the answer is no oracle for which ids exist.
///
/// Any other kind is not checked here (unchanged behaviour). A missing
/// connector is left to `save_rule`'s own validation (400). With no
/// principal the `SRC-7` kinds fail closed (401); the HTTP routes always
/// have one, and the copilot's alert tools pass theirs.
///
/// # Errors
///
/// 401, 403 or 404 as above; 503 if no pool is configured; 500 on a
/// database failure.
pub(crate) async fn authorise_rule_scope(
    state: &AppState,
    principal: Option<&Principal>,
    kind: Option<&str>,
    connector: Option<&str>,
) -> Result<(), ApiError> {
    let Some(kind) = kind.and_then(AlertKind::parse_connector_kind) else {
        return Ok(());
    };
    let Some(principal) = principal else {
        return Err(ApiError::unauthorized());
    };
    let scope = if kind == AlertKind::UploadFailure {
        "*"
    } else {
        connector.map_or("", str::trim)
    };
    if scope.is_empty() {
        return Ok(());
    }
    if scope == "*" {
        return if crate::routes::catalog::is_unrestricted(principal) {
            Ok(())
        } else {
            Err(ApiError::PermissionDenied(ALL_SCOPE_DENIED.to_owned()))
        };
    }
    crate::routes::connectors::ensure_connector_in_tenants(state, Some(principal), scope).await
}

/// [`authorise_rule_scope`] for `input`, and for the rule `id` it replaces:
/// an update must not let a caller take over, or move, a rule they could not
/// have created (`SRC-7` D7). An unrestricted caller may do both, so the
/// stored rule is not even read for them.
///
/// # Errors
///
/// As [`authorise_rule_scope`]; 503 with a fixed message when the stored
/// rule cannot be read.
pub(crate) async fn authorise_rule_update(
    state: &AppState,
    principal: Option<&Principal>,
    id: &str,
    input: &AlertRuleInput,
) -> Result<(), ApiError> {
    authorise_rule_scope(
        state,
        principal,
        input.kind.as_deref(),
        input.connector.as_deref(),
    )
    .await?;
    if principal.is_some_and(crate::routes::catalog::is_unrestricted) {
        return Ok(());
    }
    let existing = lakehouse_alerts::get_rule(&state.clickhouse, id)
        .await
        .map_err(|err| {
            tracing::warn!(%err, "an alert rule could not be read before an update");
            ApiError::Unavailable("the alert rules could not be read".to_owned())
        })?;
    match existing {
        Some(rule) => {
            authorise_rule_scope(
                state,
                principal,
                Some(rule.kind.as_str()),
                rule.connector.as_deref(),
            )
            .await
        }
        None => Ok(()),
    }
}

/// `POST /api/alerts` — create a rule.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body or a
/// validation failure (see `lakehouse_alerts::save_rule`) — matching the
/// `TypeScript`'s single `catch` around both. For the `SRC-7` rule kinds, a
/// 401, 403 or 404 from [`authorise_rule_scope`].
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let input = parse_body(&body)?;
    authorise_rule_scope(
        &state,
        principal.as_ref().map(|Extension(p)| p),
        input.kind.as_deref(),
        input.connector.as_deref(),
    )
    .await?;
    let rule = lakehouse_alerts::save_rule(&state.clickhouse, &input, None)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "rule": rule })))
}

/// `PUT /api/alerts` — update a rule (body must include `id`).
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body, a missing
/// `id`, or a validation failure — matching the `TypeScript`. For the
/// `SRC-7` rule kinds, a 401, 403 or 404 from [`authorise_rule_update`].
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let input = parse_body(&body)?;
    let Some(id) = input.id.clone() else {
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
    authorise_rule_update(
        &state,
        principal.as_ref().map(|Extension(p)| p),
        &id,
        &input,
    )
    .await?;
    let rule = lakehouse_alerts::save_rule(&state.clickhouse, &input, Some(&id))
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "rule": rule })))
}

/// Query parameters for `DELETE /api/alerts` (`?id=`).
#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    id: Option<String>,
}

/// `DELETE /api/alerts?id=` — soft-delete a rule.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] when `id` is missing, or a 500
/// [`ApiError::Internal`] on a `ClickHouse` failure — matching the
/// `TypeScript`'s pre-`try` `id` check (400) vs. its `catch` (500).
pub async fn delete(
    State(state): State<AppState>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<ApiJson<Value>> {
    let Some(id) = query.id else {
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
    lakehouse_alerts::delete_rule(&state.clickhouse, &id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// Query parameters for `GET`/`POST /api/alerts/run`.
#[derive(Debug, Deserialize, Default)]
pub struct RunQuery {
    /// Run only this rule id, when given.
    id: Option<String>,
    /// Shared token, as a query-string fallback to the `x-run-token`
    /// header.
    token: Option<String>,
}

/// The `/api/alerts/run` guard, split out of the handler so it can be
/// exercised without a live `ClickHouse` connection or ever calling
/// `run_rules`. Was a straight port of `alerts/run/route.ts:16-20`; is now
/// (D4) fail-closed instead of fail-open — see the module-level "D4" doc
/// comment on [`run`] for the full rationale.
///
/// # Security property
///
/// * `configured: Some(token)` — unchanged from the `TypeScript`: the
///   caller's header/query token must match, or this is a 401.
/// * `configured: None` (`ALERTS_RUN_TOKEN` unset) — the `TypeScript`
///   (and this handler, pre-D4) skipped the check entirely, making the
///   route completely unauthenticated: any caller could trigger a real
///   evaluation run that sends real webhooks/emails. D4 closes that: with
///   no shared token configured, only a **service-identity** principal
///   (`PrincipalId::Service` — the cron/scheduler's own credential, not a
///   logged-in human's session) is allowed through; anyone/anything else
///   is refused with 503. A human session principal is deliberately NOT
///   sufficient here even though the router's `Policy::RequiresAuth`
///   already requires ONE — this guard's whole job is to stop an ordinary
///   authenticated user (any signed-up account, not just an operator) from
///   firing every registered alert channel just by hitting this URL.
fn check_run_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<(), ApiError> {
    if let Some(need) = configured {
        return if header_token.or(query_token) == Some(need) {
            Ok(())
        } else {
            Err(ApiError::unauthorized())
        };
    }
    match principal {
        Some(p) if matches!(p.id, PrincipalId::Service(_)) => Ok(()),
        _ => Err(ApiError::Unavailable(
            "alerts run is not configured: set ALERTS_RUN_TOKEN, or call with \
             service-identity credentials (not a human user session)"
                .to_owned(),
        )),
    }
}

/// `pub(in crate::routes)`: reused by `routes::ai::tools::alerts::run_alert_rule`
/// (T1.1 of the copilot-operations-handover plan) so the copilot's
/// `run_alert_rule` tool delivers through the exact same `SmtpConfig` the
/// `/api/alerts/run` route builds, rather than a second, possibly-drifting
/// copy of this six-field mapping.
pub(in crate::routes) fn smtp_config(config: &Config) -> SmtpConfig {
    SmtpConfig {
        host: config.smtp_host.clone(),
        port: config.smtp_port,
        secure: config.smtp_secure,
        user: config.smtp_user.clone(),
        pass: config.smtp_pass.clone(),
        from: config.smtp_from.clone(),
    }
}

/// [`FreshnessSource`] backed by real Postgres `dataset_sla` rows and
/// WS2's Iceberg REST surface — the in-process path, not an HTTP call to
/// this crate's own `/api/lakehouse/tables/{ns}/{table}` route (which
/// returns `TableDetail`, a struct with no top-level `last_updated_ms`
/// field at all). `lakehouse_iceberg::rest::load_table_summary` returns
/// `TableSummary`, which does carry `last_updated_ms`, so this calls that
/// function directly, in the same process, rather than looping back
/// through HTTP. WS5 item C1.
pub(in crate::routes) struct ApiFreshnessSource<'a> {
    pub(in crate::routes) pg: &'a PgPool,
    pub(in crate::routes) iceberg: Arc<IcebergClient>,
}

#[async_trait::async_trait]
impl FreshnessSource for ApiFreshnessSource<'_> {
    async fn expected_interval_minutes(&self, table_name: &str) -> Result<Option<i64>, String> {
        // `StoreError`'s own `Display` renders the fixed string "database
        // error" (`lakehouse-store/src/error.rs`), never an upstream
        // detail, so `.to_string()` here is not a leak — unlike
        // `ChError`/`DgError`'s `Server` variant elsewhere in this crate.
        lakehouse_store::governance::expected_interval_minutes_for(self.pg, table_name)
            .await
            .map(|opt| opt.map(i64::from))
            .map_err(|err| err.to_string())
    }

    async fn last_snapshot_ms(&self, table_name: &str) -> Option<i64> {
        let (ns, table) = table_name.split_once('.')?;
        let ident = TableIdent::new(NamespaceIdent::new(ns.to_owned()), table.to_owned());
        lakehouse_iceberg::rest::load_table_summary(&self.iceberg, &ident)
            .await
            .ok()
            .and_then(|summary| summary.last_updated_ms)
    }
}

/// [`SilenceSource`] backed by real Postgres `alert_instance.silenced_until`
/// (`lakehouse_store::overview::is_rule_silenced`). WS5 item C1, WS5 plan
/// review U6.
pub(in crate::routes) struct ApiSilenceSource<'a> {
    pub(in crate::routes) pg: &'a PgPool,
}

#[async_trait::async_trait]
impl SilenceSource for ApiSilenceSource<'_> {
    async fn is_silenced(&self, rule_id: &str) -> bool {
        overview::is_rule_silenced(self.pg, rule_id, OffsetDateTime::now_utc())
            .await
            // A Postgres error here must not turn a legitimate alert into
            // a silent no-op that also fails to fire — degrade to "not
            // silenced," never fail the whole run.
            .unwrap_or(false)
    }
}

/// [`SqlGate`] over the shared policy engine
/// (`policy_engine::rewrite_sql_for_roles`, the path tiles, embeds and
/// Query Studio use).
///
/// Governed as the least-privileged "Dashboard Viewer" role, the same
/// mapping `embed.rs` documents for public/embedded dashboards: an alert
/// value or a digest is delivered outside the console (webhook, email) to
/// whoever the rule targets, so no rule author's or caller's own grants
/// apply to what the recipient sees. A refusal is the engine's fixed,
/// non-leaking message.
pub(in crate::routes) struct ApiSqlGate<'a> {
    pub(in crate::routes) pg: Option<&'a PgPool>,
    pub(in crate::routes) ch: &'a lakehouse_clickhouse::ChClient,
}

/// Plan 1f: [`LateSource`] backed by Postgres (for the SLA row) and a
/// pre-fetched map of last-success epoch seconds per pipeline id
/// (populated by the route handler from the Dagster client, so this
/// crate stays free of the Dagster SDK). When Postgres is not
/// configured, or when either query fails, the error message is
/// classified ("database error") before returning — never an upstream
/// detail, matching AGENTS.md rule 4.
pub(in crate::routes) struct ApiLateSource<'a> {
    pub(in crate::routes) pg: &'a PgPool,
    /// `pipeline_id → last successful run's epoch seconds`. A *missing*
    /// key means "no SUCCESS run on record yet" — the [`late`] helper
    /// treats that as "definitely late, threshold set." The run id is
    /// stored alongside by [`evaluate_late_pass`] for the
    /// `pipeline_run_event` dedupe; this source only carries the epoch
    /// seconds because [`LateSource::late_inputs`] is the only method
    /// the alerts crate's evaluator calls.
    pub(in crate::routes) last_success: HashMap<String, Option<LastSuccess>>,
}

#[async_trait::async_trait]
impl LateSource for ApiLateSource<'_> {
    async fn late_inputs(&self, pipeline_id: &str) -> Result<Option<(i32, Option<f64>)>, String> {
        // SLA row first: without a threshold the answer is "config gap",
        // not "not late". `get_pipeline_sla` already classifies its
        // error → `Ok(None)` for missing rows, `Err("database error")`
        // for transport failures.
        let sla = match lakehouse_store::pipelines::get_pipeline_sla(self.pg, pipeline_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return Ok(None),
            Err(_) => return Err("database error".to_owned()),
        };
        let threshold = match sla.late_after_seconds {
            Some(t) if t > 0 => t,
            // `late_after_seconds = NULL` (only `max_duration_seconds`
            // set) — there is no "late" claim to make for this
            // pipeline.
            _ => return Ok(None),
        };
        // The route handler pre-fetches last-success epoch seconds
        // per pipeline. A missing key means the Dagster round-trip
        // didn't include this pipeline id — treat it as "no SUCCESS
        // run on record," not as an error: that's the same posture a
        // missing key would have if Dagster had no runs for the job.
        let last = self
            .last_success
            .get(pipeline_id)
            .and_then(|o| o.as_ref())
            .map(|s| s.epoch_seconds);
        Ok(Some((threshold, last)))
    }
}

#[async_trait::async_trait]
impl SqlGate for ApiSqlGate<'_> {
    async fn gate(&self, sql: &str) -> Result<String, String> {
        let obligations = crate::policy_engine::PolicyEngineObligations::new(self.pg, self.ch);
        crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::ClickHouseDialect {},
            &[crate::routes::embed::EMBED_VIEWER_ROLE.to_owned()],
            &crate::sql_rewrite::PlaceholderValues::none(),
            &obligations,
        )
        .await
        .map_err(|err| crate::policy_engine::enforcement_error_message(&err).to_owned())
    }
}

/// A fixed, kind-derived `AlertItem.source` label for a fired rule's
/// persisted instance — never invented per-rule text, just which engine
/// produced it.
fn fired_source(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::Alert => "Alert rules",
        AlertKind::Freshness => "Freshness monitoring",
        AlertKind::Digest => "Digest", // never reached: callers filter Digest out before this
        AlertKind::PipelineFailure => "Pipeline failures", // never reached: callers filter PipelineFailure out before this
        AlertKind::PipelineSlow => "Pipeline SLA: duration",
        AlertKind::PipelineLate => "Pipeline SLA: late",
        AlertKind::PipelineVolumeDrop => "Pipeline SLA: volume drop",
        // `SRC-7`: connector and upload rules fire from the run event
        // routes (`routes::pipelines`), which persist their own instance
        // with the source label; `run_rules` skips them, so these labels
        // are only a fallback for an exhaustive match.
        AlertKind::ConnectorFailure
        | AlertKind::ConnectorRepeatedFailure
        | AlertKind::ConnectorDisabled
        | AlertKind::ConnectorSchemaChange
        | AlertKind::ConnectorSuccess => "Connector runs",
        AlertKind::UploadFailure => "File uploads",
    }
}

/// A human-readable description of a fired rule's breach, built from
/// [`lakehouse_alerts::RunResult::value`] and the rule's own config —
/// never fabricated beyond what both already carry.
fn fired_detail(rule: &AlertRule, value: Option<f64>) -> String {
    let value = value.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.2}"));
    match rule.kind {
        AlertKind::Freshness => format!(
            "{} is stale: observed age {value} min",
            rule.mart.as_deref().unwrap_or("")
        ),
        AlertKind::Alert | AlertKind::Digest => format!(
            "{}({}) on {} {} {} — observed {value}",
            rule.agg,
            rule.measure.as_deref().unwrap_or(""),
            rule.mart.as_deref().unwrap_or(""),
            rule.op.as_str(),
            rule.threshold
        ),
        // PipelineFailure / PipelineSlow / PipelineVolumeDrop rules fire
        // from the event routes, never `run_rules`; the detail isn't
        // used for them.
        AlertKind::PipelineFailure => "pipeline failure".to_owned(),
        AlertKind::PipelineSlow => "pipeline run exceeded its duration SLA".to_owned(),
        AlertKind::PipelineLate => {
            "pipeline is late (no successful run within the SLA window)".to_owned()
        }
        AlertKind::PipelineVolumeDrop => {
            "pipeline run processed fewer than half the median rows of prior runs".to_owned()
        }
        // `SRC-7`: never reached from `run_rules` (it skips these kinds);
        // the event routes write their own detail.
        AlertKind::ConnectorFailure
        | AlertKind::ConnectorRepeatedFailure
        | AlertKind::ConnectorDisabled
        | AlertKind::ConnectorSchemaChange
        | AlertKind::ConnectorSuccess
        | AlertKind::UploadFailure => "connector or upload event".to_owned(),
    }
}

/// Persist every fired, non-`Digest` result from `results` as an
/// `alert_instance` row, best-effort — mirrors `routes::query.rs:171-198`'s
/// history write: a Postgres outage (or a `ClickHouse` re-fetch failure)
/// must never turn an otherwise-successful `/api/alerts/run` into an error
/// response, since the webhook/email already went out. Skips (never
/// inserts) a rule the caller's own [`ApiSilenceSource`] reports as
/// currently silenced — the route layer's decision, not
/// `insert_from_fired_rule`'s (that primitive has no silence awareness by
/// design; see `lakehouse_store::overview`'s module doc comment). WS5 item
/// C1, WS5 plan review U6.
async fn persist_fired_results(
    pg: &PgPool,
    ch: &lakehouse_clickhouse::ChClient,
    results: &[lakehouse_alerts::RunResult],
) {
    if !results
        .iter()
        .any(|r| r.fired && r.kind != AlertKind::Digest)
    {
        return;
    }
    // A second `list_rules` round trip, deliberately: `RunResult` carries
    // only id/name/kind/fired/value, never the rule's own `severity`/
    // `mart` this insert needs, and `run_rules` does not return its
    // internal rule list.
    let rules = match lakehouse_alerts::list_rules(ch).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::warn!(%err, "failed to re-fetch alert rules for alert_instance persistence (results were still delivered)");
            return;
        }
    };
    let now = OffsetDateTime::now_utc();
    for result in results {
        if !result.fired || result.kind == AlertKind::Digest {
            continue;
        }
        let Some(rule) = rules.iter().find(|r| r.id == result.id) else {
            continue;
        };
        let silenced = ApiSilenceSource { pg }.is_silenced(&rule.id).await;
        if silenced {
            continue;
        }
        let detail = fired_detail(rule, result.value);
        let fired = FiredRule {
            rule_id: &rule.id,
            title: &rule.name,
            severity: rule.severity.as_deref(),
            source: fired_source(rule.kind),
            affected: rule.mart.as_deref().unwrap_or(""),
            detail: &detail,
        };
        if let Err(err) = overview::insert_from_fired_rule(pg, &fired, now).await {
            tracing::warn!(%err, rule_id = %rule.id, "failed to persist fired alert instance (delivery already happened)");
        }
    }
}

/// `GET`/`POST /api/alerts/run` — evaluate rules and deliver alerts/
/// digests that fire.
///
/// # Warning
///
/// This is a live, side-effecting endpoint: on success it queries
/// `serving.*` marts and, for every rule that fires, sends a real webhook
/// `POST` or a real `SMTP` email. It is NOT captured in the parity corpus
/// for exactly this reason — see `rust/tests/parity/README.md`'s
/// "Deliberate omissions" section. Do not call this against production
/// infrastructure while testing; only the rejection paths (wrong/missing
/// token with `ALERTS_RUN_TOKEN` set; no token configured and no
/// service-identity principal) are safe to exercise.
///
/// # D4: fail closed when `ALERTS_RUN_TOKEN` is unset
///
/// The `TypeScript` original — and this handler, before this fix — skipped
/// its shared-token check entirely when `ALERTS_RUN_TOKEN` was unset,
/// which is precisely the state of this environment: the guard was a
/// no-op, and any caller (any signed-up user, once auth landed; literally
/// anyone, before it) could trigger every alert rule and fire real
/// webhooks/emails. [`check_run_token`] now fails closed: with no token
/// configured, only a `PrincipalId::Service` principal — a
/// `service_identity` credential meant for the cron/scheduler that runs
/// this on a timer, not a human's browser session — is let through;
/// everyone else gets a 503. The 503 (not 401) is deliberate: this is a
/// missing-configuration state ("nobody set up how this route should be
/// called"), the same idiom `routes::identity::pool` uses for "no
/// `DATABASE_URL`" — not a bad-credential state, which is what 401 means
/// everywhere else in this crate. The token path (`ALERTS_RUN_TOKEN` set)
/// is unchanged, so an existing cron/scheduler integration keeps working
/// exactly as before.
///
/// # Errors
///
/// Returns a 401 [`ApiError::unauthorized`] when `ALERTS_RUN_TOKEN` is set
/// and the caller's token doesn't match; a 503 [`ApiError::Unavailable`]
/// when it is unset and the caller is not an authenticated service-identity
/// principal; or a 500 [`ApiError::Internal`] on a `ClickHouse` failure
/// while listing rules.
pub async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_run_token(
        state.config.alerts_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let http = reqwest::Client::new();
    let email = EmailSender::new(smtp_config(&state.config));
    // `freshness`/`silence` sources are real, Postgres/Iceberg-backed
    // implementations when both dependencies are configured for this
    // request; otherwise `None` — every `Freshness` rule then reports
    // `skipped` and no rule's delivery is ever silence-suppressed, an
    // honest degrade rather than a placeholder failure. `AppState::iceberg`
    // is read once, through the lock, with the guard dropped immediately
    // (never held across an `.await`) — this route only uses whatever
    // client is already cached; it does not itself trigger a connect.
    let cached_iceberg = state.iceberg.read().await.clone();
    let freshness_source = match (state.pg.as_deref(), cached_iceberg) {
        (Some(pg), Some(iceberg)) => Some(ApiFreshnessSource { pg, iceberg }),
        _ => None,
    };
    let silence_source = state.pg.as_deref().map(|pg| ApiSilenceSource { pg });
    let gate = ApiSqlGate {
        pg: state.pg.as_deref(),
        ch: &state.clickhouse,
    };

    let results = lakehouse_alerts::run_rules(
        &state.clickhouse,
        &http,
        &email,
        query.id.as_deref(),
        freshness_source.as_ref().map(|s| s as &dyn FreshnessSource),
        silence_source.as_ref().map(|s| s as &dyn SilenceSource),
        &gate,
    )
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;

    // Persist every fired, non-Digest result as an `alert_instance` row,
    // best-effort (WS5 item C1) — see `persist_fired_results`'s doc
    // comment.
    if let Some(pg) = state.pg.as_deref() {
        persist_fired_results(pg, &state.clickhouse, &results).await;
    }

    // Plan 1f: `pipeline_late` rules fire from this op, not from the
    // orchestrator event path. A "late" claim depends on the clock at
    // evaluation time, not at run time. The route handler iterates
    // every enabled `pipeline_late` rule whose `pipeline` names a
    // specific pipeline id (the `*` wildcard is not yet wired for
    // pipeline_late — it would require enumerating every runnable
    // pipeline here, which is the plan's next-step scope, not this
    // commit's) and calls [`evaluate_pipeline_late`] per pipeline.
    // The matching `run_rules` entries (each a `skipped` with the
    // reason "pipeline_late rules are evaluated by /api/alerts/run...")
    // stay in the response, so the user sees both the rule list and
    // the per-pipeline deliveries.
    if let Some(pg) = state.pg.as_deref() {
        let late_rule_pipelines = list_late_rule_pipelines(&state.clickhouse).await?;
        if !late_rule_pipelines.is_empty() {
            let last_success =
                fetch_last_success_per_pipeline(&state.dagster, &late_rule_pipelines).await;
            // Plan 1f (reviewer fix #4): the clock helper returns `None`
            // when the system clock is unset. Pass that `None` straight
            // through to `evaluate_late_pass` — it skips every pipeline
            // (a "late" claim is undecidable without a clock) rather
            // than silently fabricating a `Some(0.0)` "late since 1970"
            // for each one. The helper log line is the visible audit
            // trail for the skip.
            let now_seconds = crate::routes::pipelines::now_unix_seconds();
            evaluate_late_pass(
                pg,
                &state.clickhouse,
                &http,
                &email,
                late_rule_pipelines,
                &last_success,
                silence_source.as_ref().map(|s| s as &dyn SilenceSource),
                now_seconds,
            )
            .await?;
        }
    }

    // The same tick keeps quality verdicts current: every runnable rule
    // whose verdict is over an hour old is run again, in the background —
    // a table scan must not hold up this response (the scheduler waits 30
    // seconds for it). Not on a single-rule run (`?id=`), which is someone
    // testing one alert, not the tick.
    let quality_started =
        query.id.is_none() && crate::routes::quality::spawn_scheduled_pass(&state);

    // And records the schema a Silver or Gold table has now when it differs
    // from the last one recorded (ADR 0015), in the background for the same
    // reason, and not on a single-rule run either.
    let schema_started = query.id.is_none() && crate::routes::schema_versions::spawn_pass(&state);

    // The semantic layer's drafting pass (AI-16): asks the deployment's
    // model for a description of each table that has none, up to ten per
    // tick, in the background for the same reason. The response does not
    // report it: it changes no verdict, and a pass that starts nothing
    // (switch off, no key, one already running) is not a fault to show.
    if query.id.is_none() {
        crate::routes::ai::semantic::spawn_pass(&state);
    }

    Ok(ApiJson(json!({
        "ran": results.len(),
        "results": results,
        "qualityPassStarted": quality_started,
        "schemaPassStarted": schema_started,
    })))
}

/// Plan 1f: list every distinct `pipeline_id` referenced by an enabled
/// `pipeline_late` rule with a non-`None`, non-`"*"` `pipeline` field.
/// Used by [`run`] to drive [`evaluate_pipeline_late`]. Filters
/// in-process from `list_rules` to avoid a second `ClickHouse`
/// round-trip and a second schema surface
/// (`pipeline_late_rule_pipelines`) — the underlying query is the same
/// `console.alert_rule` SELECT.
///
/// # Errors
///
/// Returns `Err("database error")` on a `ClickHouse` failure — never an
/// upstream detail, matching AGENTS.md rule 4.
async fn list_late_rule_pipelines(
    ch: &lakehouse_clickhouse::ChClient,
) -> Result<Vec<String>, ApiError> {
    let rules = lakehouse_alerts::list_rules(ch)
        .await
        .map_err(|_| ApiError::Internal("database error".to_owned()))?;
    let mut out: Vec<String> = rules
        .into_iter()
        .filter(|r| {
            r.enabled
                && r.kind == lakehouse_alerts::AlertKind::PipelineLate
                && r.pipeline
                    .as_deref()
                    .is_some_and(|p| !p.is_empty() && p != "*")
        })
        .filter_map(|r| r.pipeline)
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// Plan 1f: pull the last SUCCESS run (epoch seconds + run id) for every
/// pipeline id in `pipeline_ids` from the Dagster client (one round-trip
/// per id, the same call [`routes::pipelines`] uses for the runs/detail
/// rows). A per-pipeline failure (e.g. Dagster temporarily unreachable)
/// is captured as `None` for that id, matching the same posture
/// `routes::pipelines::last_success_for` takes for a missing run — a
/// missing last-success is then treated by [`late`] as "definitely
/// late, threshold set," which is the honest answer when a pipeline's
/// orchestrator is silent.
///
/// The run id is the dedupe key for the `pipeline_run_event` row written
/// per late episode (reviewer fix #2); without it, two passes for the
/// same "no new success since the threshold" state would re-deliver.
async fn fetch_last_success_per_pipeline(
    dagster: &lakehouse_dagster::DgClient,
    pipeline_ids: &[String],
) -> HashMap<String, Option<LastSuccess>> {
    let mut out = HashMap::new();
    for id in pipeline_ids {
        let job_name = job_name_for_pipeline_id(id);
        let last = dagster
            .list_runs_for_job(&job_name, 30)
            .await
            .ok()
            .and_then(|runs| last_success_for(&runs, &job_name));
        out.insert(id.clone(), last);
    }
    out
}

/// The most recent SUCCESS run's `start_time` epoch seconds and its run
/// id, for the deduplicated `pipeline_run_event` key the late pass uses
/// (reviewer fix #2). `None` when no SUCCESS run is on record yet.
#[derive(Debug, Clone)]
pub(in crate::routes) struct LastSuccess {
    pub(in crate::routes) epoch_seconds: f64,
    pub(in crate::routes) run_id: String,
}

/// Plan 1f: the Dagster job name for `pipeline_id`. Mirrors
/// `routes::pipelines::list_body`'s own `authored__<id>` mapping —
/// `evaluate_pipeline_late` runs against the same `authored__*` jobs
/// the orchestrator launches. `*` is not supported here (see
/// [`list_late_rule_pipelines`]).
fn job_name_for_pipeline_id(pipeline_id: &str) -> String {
    format!("authored__{pipeline_id}")
}

/// Plan 1f: the most recent SUCCESS run for `job_name` in `runs` —
/// `(epoch_seconds, run_id)` of the newest SUCCESS by `start_time`.
/// `None` when no SUCCESS run is present. Mirrors the same pattern as
/// `routes::pipelines::last_success_for` but returns both the epoch
/// seconds (for [`late`]) and the run id (for the dedupe key).
fn last_success_for(runs: &[lakehouse_dagster::DgRun], job_name: &str) -> Option<LastSuccess> {
    runs.iter()
        .filter(|r| r.job_name == job_name && r.status == "SUCCESS")
        .filter_map(|r| r.start_time.map(|t| (t, r.run_id.clone())))
        .fold(None, |best, (t, id)| {
            Some(match best {
                None => LastSuccess {
                    epoch_seconds: t,
                    run_id: id,
                },
                Some(prev) if t > prev.epoch_seconds => LastSuccess {
                    epoch_seconds: t,
                    run_id: id,
                },
                Some(prev) => prev,
            })
        })
}

/// Plan 1f reviewer fix #2: the `pipeline_run_event` row key for a
/// `pipeline_late` episode. The spec says "the last success's run id as
/// the key." When there is no last success (pipeline has never
/// succeeded), this falls back to a per-pipeline sentinel so each
/// pipeline's "no success yet" state still has a unique dedupe key —
/// without it, two pipelines without a success would collide on a
/// `None` run id and the second pipeline's first late episode would
/// silently dedupe away.
fn late_episode_key(pipeline_id: &str, last: Option<LastSuccess>) -> String {
    last.map_or_else(|| format!("{pipeline_id}::no_success"), |s| s.run_id)
}

/// Plan 1f reviewer fix #4 + #2 orchestration: run the late-evaluation
/// pass (one pass per `/api/alerts/run` invocation). Skips entirely when
/// `now_seconds` is `None` — a "late" claim is undecidable without a
/// clock, and the alternative (`Some(0.0)` "late since 1970") is the
/// exact bug the single crate-wide `Option<f64>` clock helper prevents.
/// For each `pipeline_id`, computes the late decision from
/// `LateSource`; when it is `Some(true)`, records the dedupe row
/// (`kind="late"`, `run_id = late_episode_key(...)`) BEFORE delivery
/// (mirrors the run-failed route's pattern at routes/pipelines.rs:1302).
/// `first_seen == false` short-circuits without delivery — that is the
/// sensor-retry / repeat-pass dedupe the spec requires.
///
/// Returned `usize` is the number of pipelines for which delivery was
/// attempted (one per `first_seen` true). Used by the route-level
/// integration test as a "did we deliver at all" signal.
///
/// # Errors
///
/// Returns a 500 [`ApiError::Internal`] when the late source propagates a
/// classified error or a delivery fails; a failed dedupe insert maps
/// through `From<StoreError> for ApiError`.
async fn evaluate_late_pass(
    pg: &PgPool,
    ch: &lakehouse_clickhouse::ChClient,
    http: &reqwest::Client,
    email: &EmailSender,
    late_rule_pipelines: Vec<String>,
    last_success: &HashMap<String, Option<LastSuccess>>,
    silence_source: Option<&dyn SilenceSource>,
    now_seconds: Option<f64>,
) -> Result<usize, ApiError> {
    // Reviewer fix #4: when the system clock is unset (an unreachable
    // NTP source on some build hosts), `now_unix_seconds` returns
    // `None`. Without a clock we cannot decide whether any pipeline is
    // currently late, and "late since 1970" would be a worse answer
    // than honest silence. Skip the whole pass with a single
    // `warn!` so an operator can see why their `/api/alerts/run`
    // invocation did not raise anything.
    let Some(now) = now_seconds else {
        tracing::warn!(
            "pipeline_late pass skipped: system clock is unset (SystemTime::now() \
             returned a value before the UNIX epoch)"
        );
        return Ok(0);
    };
    let late_source = ApiLateSource {
        pg,
        last_success: last_success.clone(),
    };
    let mut delivered_pipelines = 0_usize;
    for pipeline_id in &late_rule_pipelines {
        // Reviewer fix #2: dedupe happens BEFORE delivery, the same
        // order `run_failed_event` uses (routes/pipelines.rs:1302).
        // `late_decision` returns the plan 1f `late` helper's result
        // without firing any rule.
        let decision = lakehouse_alerts::late_decision(
            Some(&late_source as &dyn lakehouse_alerts::LateSource),
            pipeline_id,
            Some(now),
        )
        .await
        .map_err(|err| ApiError::Internal(format!("{err}")))?;
        if decision != Some(true) {
            continue;
        }
        let dedupe_key = late_episode_key(
            pipeline_id,
            // `get` yields `Option<&Option<_>>`; clone + flatten lifts
            // the inner `Option<LastSuccess>` out in one step.
            last_success.get(pipeline_id).cloned().flatten(),
        );
        // Write the dedupe row FIRST. A `false` return means the same
        // `(run_id, kind)` pair was already inserted by a previous
        // pass for the same episode — short-circuit so this late
        // episode does not re-deliver.
        let first_seen = lakehouse_store::pipelines::record_pipeline_run_event(
            pg,
            &dedupe_key,
            pipeline_id,
            "late",
        )
        .await
        .map_err(ApiError::from)?;
        if !first_seen {
            continue;
        }
        // Delivery — only happens on a fresh insert.
        lakehouse_alerts::evaluate_pipeline_late(
            ch,
            http,
            email,
            pipeline_id,
            Some(&late_source as &dyn lakehouse_alerts::LateSource),
            silence_source,
            Some(now),
        )
        .await
        .map_err(|err| ApiError::Internal(format!("{err}")))?;
        delivered_pipelines += 1;
    }
    Ok(delivered_pipelines)
}

// Plan 1f note: `now_seconds` for the late-evaluation pass comes from
// [`crate::routes::pipelines::now_unix_seconds`] — the single crate-wide
// `Option<f64>` clock helper. A `None` from that helper skips the whole
// pass (a broken clock means we cannot make a "late" claim, and the
// alternative — `Some(0.0)` "late since 1970" — is exactly the bug this
// helper exists to prevent).

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;

    use super::*;

    fn service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "alerts-cron".to_owned(),
            permissions: PermissionSet::default(),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("*:*"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[test]
    fn run_endpoint_rejects_wrong_token_with_401() {
        let err = check_run_token(Some("secret"), None, Some("wrong"), None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
        assert_eq!(err.status(), 401);
    }

    #[test]
    fn run_endpoint_rejects_missing_token_with_401() {
        let err = check_run_token(Some("secret"), None, None, None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
    }

    #[test]
    fn run_endpoint_accepts_matching_header_token() {
        assert!(check_run_token(Some("secret"), Some("secret"), None, None).is_ok());
    }

    #[test]
    fn run_endpoint_accepts_matching_query_token_as_fallback() {
        assert!(check_run_token(Some("secret"), None, Some("secret"), None).is_ok());
    }

    #[test]
    fn run_endpoint_prefers_header_token_over_query_token() {
        // `req.headers.get("x-run-token") || url.searchParams.get("token")`
        // — the header wins when both are present, matching JS `||`.
        assert!(check_run_token(Some("secret"), Some("secret"), Some("wrong"), None).is_ok());
    }

    /// A token, once configured, is still checked even when the caller
    /// happens to also be a service-identity principal — the token path and
    /// the no-token-configured fallback are mutually exclusive branches,
    /// not additive.
    #[test]
    fn configured_token_still_required_even_for_a_service_principal() {
        let service = service_principal();
        let err = check_run_token(Some("secret"), None, Some("wrong"), Some(&service)).unwrap_err();
        assert_eq!(err.status(), 401);
    }

    /// D4 regression: with `ALERTS_RUN_TOKEN` unset, the guard used to pass
    /// unconditionally (see git history / the pre-D4 doc comment) —
    /// including for a caller presenting no token at all and no principal.
    /// It must now fail closed: 503, not a silent pass.
    #[test]
    fn run_endpoint_fails_closed_when_token_unset_and_no_principal() {
        let err = check_run_token(None, None, None, None).unwrap_err();
        assert_eq!(err.status(), 503);
        let err_with_wrong_token = check_run_token(None, Some("anything"), None, None).unwrap_err();
        assert_eq!(err_with_wrong_token.status(), 503);
    }

    /// D4: a human session principal (however highly privileged — even
    /// `*:*`) is still refused when no token is configured. This guard's
    /// whole point is that "some authenticated user" is not enough for a
    /// route that fires real webhooks/emails; only the dedicated
    /// service-identity door is.
    #[test]
    fn run_endpoint_fails_closed_for_a_human_principal_even_with_wildcard_permissions() {
        let user = user_principal();
        let err = check_run_token(None, None, None, Some(&user)).unwrap_err();
        assert_eq!(err.status(), 503);
    }

    /// D4: the new, intended long-term door — a service-identity principal
    /// (e.g. the cron/scheduler's own credential) is let through even with
    /// no shared token configured.
    #[test]
    fn run_endpoint_allows_a_service_identity_principal_when_token_unset() {
        let service = service_principal();
        assert!(check_run_token(None, None, None, Some(&service)).is_ok());
    }

    /// WS5 item C1, WS5 plan review U6: an end-to-end assertion that a
    /// silence suppresses both delivery AND a fresh `alert_instance` row —
    /// not just `lakehouse_store::overview`'s primitive-level boundary
    /// (`overview::tests::a_silenced_rule_produces_no_new_instance_after_its_dedup_window_lapses`
    /// documents that `insert_from_fired_rule` alone has no silence
    /// awareness; this test proves the route layer's decision not to call
    /// it while silenced).
    mod silence_end_to_end {
        use std::collections::HashMap;

        use axum::extract::{Query, State};
        use lakehouse_store::overview::{self, FiredRule};
        use lakehouse_test_support as _;
        use time::OffsetDateTime;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use super::service_principal;
        use crate::config::Config;

        /// One enabled `Alert` rule, `ClickHouse`-row shaped exactly like
        /// `row_to_rule` (`lakehouse-alerts/src/lib.rs`) expects — string
        /// values throughout, matching `FORMAT JSON`'s stringified-integer
        /// convention the rest of this codebase's `ClickHouse` fixtures
        /// already use (see `row_to_rule_maps_freshness_type_column`).
        fn rule_row_json() -> Value {
            json!({
                "id": "al-route-test-1",
                "name": "Route test rule",
                "type": "alert",
                "mart": "route_test_mart",
                "measure": "v",
                "agg": "sum",
                "op": ">",
                "threshold": "10",
                "board": "",
                "channel": "webhook",
                "target": "http://127.0.0.1:1/never-called",
                "enabled": "1",
                "created_at": "",
                "severity": "high",
            })
        }

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database")
            )
        }

        fn state_for(pool: &sqlx::PgPool, ch_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_silenced_rule_is_not_re_delivered_and_produces_no_new_row(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            // `list_rules`' `SELECT ... FROM console.alert_rule` — one
            // enabled Alert rule.
            Mock::given(method("POST"))
                .and(body_string_contains("FROM console.alert_rule"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [rule_row_json()], "rows": 1
                })))
                .mount(&server)
                .await;
            // `current_value`'s `SELECT round(sum(v)) FROM serving.<mart>`
            // — a value far over the rule's threshold of 10, so it fires.
            Mock::given(method("POST"))
                .and(body_string_contains("FROM serving."))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [{ "v": "999" }], "rows": 1
                })))
                .mount(&server)
                .await;
            // `ensure()`'s DDL statements (CREATE DATABASE/TABLE, ADD
            // COLUMN) — a blank 200 for anything else.
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;

            let state = state_for(&pool, &server.uri());
            let pg = state.pg.as_deref().expect("DATABASE_URL was set above");

            // Fixture: the rule already fired once, 20 minutes ago (past
            // the 15-minute dedup window), and is silenced 60 minutes out
            // from now.
            let first = overview::insert_from_fired_rule(
                pg,
                &FiredRule {
                    rule_id: "al-route-test-1",
                    title: "Route test rule",
                    severity: Some("high"),
                    source: "Alert rules",
                    affected: "route_test_mart",
                    detail: "seed",
                },
                OffsetDateTime::now_utc() - time::Duration::minutes(20),
            )
            .await
            .unwrap()
            .unwrap();
            overview::silence_alert(
                pg,
                &first.id,
                OffsetDateTime::now_utc() + time::Duration::minutes(60),
            )
            .await
            .unwrap();

            let response = run(
                State(state),
                HeaderMap::new(),
                Query(RunQuery {
                    id: Some("al-route-test-1".to_owned()),
                    token: None,
                }),
                Some(Extension(service_principal())),
            )
            .await
            .expect("a service-identity principal must pass the run-token guard");

            let body = response.0;
            let result = &body["results"][0];
            assert_eq!(
                result["fired"], true,
                "the rule is still genuinely over threshold"
            );
            assert!(
                result.get("delivered").is_none(),
                "delivery must be suppressed during the silence"
            );

            let row_count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM alert_instance WHERE rule_id = $1")
                    .bind("al-route-test-1")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                row_count, 1,
                "still only the original silenced row — no fresh instance while silenced"
            );
        }
    }

    /// ADR 0015 — the tick records the schema versions of Silver and Gold
    /// tables beside the quality pass; a single-rule run (`?id=`), which is
    /// someone testing one alert and not the tick, does not.
    mod schema_pass {
        use std::collections::HashMap;
        use std::time::Duration;

        use axum::extract::{Query, State};
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use super::service_principal;
        use crate::config::Config;

        /// A state with no `PostgreSQL` pool (a malformed `DATABASE_URL`,
        /// the idiom `routes::gold`'s tests use) and `ClickHouse` at `ch_url`.
        fn state_for(ch_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        async fn call_run(state: &AppState, id: Option<&str>) -> Value {
            run(
                State(state.clone()),
                HeaderMap::new(),
                Query(RunQuery {
                    id: id.map(str::to_owned),
                    token: None,
                }),
                Some(Extension(service_principal())),
            )
            .await
            .expect("a service-identity principal must pass the run-token guard")
            .0
        }

        async fn reads_of_system_columns(server: &MockServer) -> usize {
            server
                .received_requests()
                .await
                .expect("recorded")
                .iter()
                .filter(|r| String::from_utf8_lossy(&r.body).contains("FROM system.columns"))
                .count()
        }

        #[tokio::test]
        async fn a_single_rule_run_does_not_start_a_schema_pass() {
            let server = MockServer::start().await;
            // Every statement answers `200` with no rows: no rules, no tables.
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            let state = state_for(&server.uri());

            let body = call_run(&state, Some("al-no-such-rule")).await;

            assert_eq!(body["schemaPassStarted"], json!(false));
            assert_eq!(body["qualityPassStarted"], json!(false));
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(reads_of_system_columns(&server).await, 0);
        }

        #[tokio::test]
        async fn the_tick_starts_a_schema_pass_that_reads_the_engine() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            let state = state_for(&server.uri());

            // One pass runs at a time in the whole process, and a pass that
            // another test started (a run-finished report starts one too) may
            // still be running when this tick comes: that answers `false`,
            // so ask again until a pass of ours starts.
            let mut started = false;
            for _ in 0..100 {
                if call_run(&state, None).await["schemaPassStarted"] == json!(true) {
                    started = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert!(started, "a tick must start the schema pass");

            // The pass is in the background: wait for it to reach the engine.
            for _ in 0..200 {
                if reads_of_system_columns(&server).await > 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("the schema pass the tick started never read the engine's columns");
        }
    }

    /// Plan 1f reviewer fix #2 + #4 — the `pipeline_late` pass inside
    /// `/api/alerts/run`: the clock contract (`None` skips the whole
    /// pass without touching Postgres or `ClickHouse`) and the
    /// per-episode dedupe (a second pass for the same last-success
    /// run id must not re-deliver, via `pipeline_run_event`).
    mod late_pass {
        use lakehouse_clickhouse::ChClient;
        use lakehouse_notify::EmailSender;
        use wiremock::MockServer;
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};

        use super::super::{LastSuccess, evaluate_late_pass};
        use super::*;

        fn no_smtp_email_sender() -> EmailSender {
            EmailSender::new(SmtpConfig {
                host: None,
                port: 587,
                secure: false,
                user: None,
                pass: String::new(),
                from: String::new(),
            })
        }

        /// A `ClickHouse` client pointed at `server`, whose only mock
        /// answers `list_rules`' SELECT with zero rows — enough for the
        /// pass to run to completion (with no rules, nothing is
        /// delivered) while still recording every request it receives,
        /// so a test can assert nobody dialed `ClickHouse` at all.
        async fn ch_for(server: &MockServer) -> ChClient {
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [], "rows": 0
                })))
                .mount(server)
                .await;
            ChClient::new(server.uri(), "default".to_owned(), String::new())
        }

        /// Reviewer fix #4: `now_unix_seconds()` returning `None` (no
        /// usable system clock) must skip the ENTIRE pass — no SLA
        /// lookup, no rule listing, no delivery attempt. The wiremock
        /// request log is the proof: an empty log means `ClickHouse`
        /// was never dialed, so nothing downstream of the clock check
        /// ran.
        ///
        /// The fixture is deliberately load-bearing: the last success
        /// sits at a negative epoch so the pipeline WOULD be decided
        /// late if the pass ran under the old `Some(0.0)` fallback
        /// (0.0 − (−3600) > 60). With a realistic "an hour ago" success
        /// the old fallback would evaluate to not-late and this test
        /// would pass either way — proving nothing. A negative epoch is
        /// absurd as real data; it is exactly the value that separates
        /// "skipped" from "evaluated", which is the contract under
        /// test.
        #[sqlx::test(migrations = "../../migrations")]
        async fn late_pass_skips_entirely_when_the_clock_is_unset(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            let ch = ch_for(&server).await;
            let http = reqwest::Client::new();
            let email = no_smtp_email_sender();
            // Threshold 60s, so under the old fallback the run at
            // epoch −3600 is 3600 seconds late.
            lakehouse_store::pipelines::upsert_pipeline_sla(
                &pool,
                "pl-clock-test",
                None,
                Some(60),
                uuid::Uuid::nil(),
            )
            .await
            .unwrap();
            let mut last_success = HashMap::new();
            last_success.insert(
                "pl-clock-test".to_owned(),
                Some(LastSuccess {
                    epoch_seconds: -3600.0,
                    run_id: "run-clock".to_owned(),
                }),
            );

            let delivered = evaluate_late_pass(
                &pool,
                &ch,
                &http,
                &email,
                vec!["pl-clock-test".to_owned()],
                &last_success,
                None,
                None, // the broken-clock case under test
            )
            .await
            .expect("a skipped pass is not an error");

            assert_eq!(delivered, 0, "nothing is delivered without a clock");
            let dialed = server.received_requests().await.unwrap();
            assert!(
                dialed.is_empty(),
                "the pass must not touch ClickHouse when the clock is unset, got {} requests",
                dialed.len()
            );
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_run_event")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(events, 0, "no dedupe row is written without a clock");
        }

        /// Reviewer fix #2: the late pass dedupes per episode through
        /// `pipeline_run_event` keyed by the last success's run id. The
        /// first pass for an episode inserts the row and delivers (the
        /// return counts it); a second pass for the SAME episode must
        /// short-circuit on the dedupe row — return 0, and still
        /// exactly one `pipeline_run_event` row.
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_second_late_pass_for_the_same_episode_does_not_re_deliver(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            let ch = ch_for(&server).await;
            let http = reqwest::Client::new();
            let email = no_smtp_email_sender();
            // The pipeline is late: threshold 60s, last success an hour
            // before `now`.
            lakehouse_store::pipelines::upsert_pipeline_sla(
                &pool,
                "pl-dedupe-test",
                None,
                Some(60),
                uuid::Uuid::nil(),
            )
            .await
            .unwrap();
            let now = 1_800_000_000.0;
            let mut last_success = HashMap::new();
            last_success.insert(
                "pl-dedupe-test".to_owned(),
                Some(LastSuccess {
                    epoch_seconds: now - 3600.0,
                    run_id: "run-abc".to_owned(),
                }),
            );

            let first = evaluate_late_pass(
                &pool,
                &ch,
                &http,
                &email,
                vec!["pl-dedupe-test".to_owned()],
                &last_success,
                None,
                Some(now),
            )
            .await
            .expect("the first pass must succeed");
            assert_eq!(first, 1, "the first pass delivers the episode once");

            let second = evaluate_late_pass(
                &pool,
                &ch,
                &http,
                &email,
                vec!["pl-dedupe-test".to_owned()],
                &last_success,
                None,
                Some(now),
            )
            .await
            .expect("a repeat pass must not error");
            assert_eq!(
                second, 0,
                "the second pass for the same episode is deduped away"
            );

            // Exactly one dedupe row, keyed by the last success's run
            // id — the spec's key choice, pinned here so a drift back
            // to an unspecified key fails loudly.
            let rows: Vec<(String, String, String)> =
                sqlx::query_as("SELECT run_id, pipeline_id, kind FROM pipeline_run_event")
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                rows,
                vec![(
                    "run-abc".to_owned(),
                    "pl-dedupe-test".to_owned(),
                    "late".to_owned()
                )],
                "one row, keyed by the last success's run id"
            );
        }

        /// The dedupe key for a pipeline that has never succeeded is
        /// the per-pipeline sentinel, not an empty string or `None`:
        /// two pipelines without a success must not collide on one key
        /// (the second one's first late episode would silently dedupe
        /// away).
        #[test]
        fn the_no_success_episode_key_is_per_pipeline() {
            assert_eq!(
                super::super::late_episode_key("pl-a", None),
                "pl-a::no_success"
            );
            assert_eq!(
                super::super::late_episode_key(
                    "pl-b",
                    Some(LastSuccess {
                        epoch_seconds: 1.0,
                        run_id: "r1".to_owned(),
                    })
                ),
                "r1"
            );
        }
    }
}

#[cfg(test)]
mod sql_gate_enforcement {
    //! Alerts/digests must be governed like the dashboards they summarize:
    //! same two-harness shape as `support::run_spec_sql_enforcement` (a real
    //! ephemeral Postgres policy row + a wiremock `ClickHouse`).

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_alerts::SqlGate;
    use lakehouse_clickhouse::ChClient;
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::{self, CreatePolicyInput};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::ApiSqlGate;

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_digest_or_alert_statement_is_masked_for_the_dashboard_viewer_role(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "alert-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Dashboard Viewer".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Dashboard Viewer"],"table":"serving.mart_x","mask":["email"]}"#
                        .to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "amount", "default_kind": "", "default_expression": ""},
                ],
                "rows": 2,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let gate = ApiSqlGate {
            pg: Some(&pool),
            ch: &ch,
        };

        let governed = gate
            .gate("SELECT email, sum(amount) AS v FROM serving.mart_x GROUP BY email")
            .await
            .expect("a governed table is rewritten, not refused");

        assert!(
            governed.contains("replaceRegexpOne(toString(`email`)"),
            "the masked column must be rewritten: {governed}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_statement_the_engine_refuses_is_refused_with_a_fixed_message(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let gate = ApiSqlGate {
            pg: Some(&pool),
            ch: &ch,
        };

        let err = gate
            .gate("SELECT * FROM url('http://example.com/x.csv', 'CSV')")
            .await
            .expect_err("a table function is refused");

        assert!(!err.contains("example.com"), "{err}");
        assert!(server.received_requests().await.unwrap().is_empty());
        Ok(())
    }
}

#[cfg(test)]
mod rule_scope_authorisation {
    //! `SRC-7` D7/F7: who may save a rule of the connector and upload kinds.
    //! A real ephemeral Postgres (connector tenants) and a wiremock
    //! `ClickHouse` that answers every rule read with one configured row.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::http::StatusCode;
    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const OWN_TENANT: &str = "11111111-1111-4111-8111-000000000001";
    const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
    const OWN_CONNECTOR: &str = "conn-pg-lakehouse";
    const OTHER_CONNECTOR: &str = "conn-s3-warehouse";

    /// A Data Engineer: `alert:write`, member of [`OWN_TENANT`] only.
    fn engineer() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(7)),
            tenant_ids: vec![OWN_TENANT.parse().unwrap()],
            display_name: "Data Engineer".to_owned(),
            permissions: PermissionSet::parse("alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// An administrator who sees every tenant (`*:*`).
    fn admin() -> Principal {
        Principal {
            permissions: PermissionSet::parse("*:*"),
            ..engineer()
        }
    }

    /// `existing` is the `(type, connector)` of the one rule the rule store
    /// answers every read with.
    async fn setup(pool: &sqlx::PgPool, existing: (&str, &str)) -> (AppState, MockServer) {
        for (id, tenant) in [(OWN_CONNECTOR, OWN_TENANT), (OTHER_CONNECTOR, OTHER_TENANT)] {
            sqlx::query("UPDATE connector SET tenant_id = $2 WHERE id = $1")
                .bind(id)
                .bind(Uuid::parse_str(tenant).unwrap())
                .execute(pool)
                .await
                .unwrap();
        }
        let ch = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [], "rows": 1,
                "data": [{
                    "id": "al_x", "name": "Rule", "type": existing.0,
                    "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0",
                    "board": "", "channel": "webhook", "target": "https://hooks.example.com/x",
                    "enabled": "1", "created_at": "", "severity": "", "pipeline": "",
                    "connector": existing.1,
                }]
            })))
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ch)
            .await;
        let options = pool.connect_options();
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options.get_database().expect("a named test database"),
            ),
        );
        env.insert("CH_URL".to_owned(), ch.uri());
        (
            AppState::new(Config::from_map(&env).expect("a valid test Config")),
            ch,
        )
    }

    fn rule_body(kind: &str, connector: Option<&str>, id: Option<&str>) -> Bytes {
        Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "id": id, "name": "Load failed", "type": kind, "connector": connector,
                "channel": "webhook", "target": "https://hooks.example.com/x",
                "pipeline": "pl-x",
            }))
            .unwrap(),
        )
    }

    async fn create_as(
        state: &AppState,
        principal: Option<Principal>,
        kind: &str,
        connector: Option<&str>,
    ) -> Result<(), ApiError> {
        create(
            State(state.clone()),
            principal.map(Extension),
            rule_body(kind, connector, None),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
    }

    async fn update_as(
        state: &AppState,
        principal: Principal,
        kind: &str,
        connector: Option<&str>,
    ) -> Result<(), ApiError> {
        update(
            State(state.clone()),
            Some(Extension(principal)),
            rule_body(kind, connector, Some("al_x")),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_user_may_save_a_rule_for_a_connector_of_their_own_tenant(pool: sqlx::PgPool) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        for kind in [
            "connector_failure",
            "connector_repeated_failure",
            "connector_success",
        ] {
            create_as(&state, Some(engineer()), kind, Some(OWN_CONNECTOR))
                .await
                .unwrap();
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_user_who_does_not_see_every_tenant_cannot_save_a_rule_for_all_or_for_uploads(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        for (kind, connector) in [
            ("connector_failure", Some("*")),
            ("connector_disabled", Some("*")),
            ("upload_failure", None),
            ("upload_failure", Some(OWN_CONNECTOR)),
        ] {
            let err = create_as(&state, Some(engineer()), kind, connector)
                .await
                .unwrap_err();
            assert_eq!(err.status(), 403, "{kind} {connector:?}");
            assert_eq!(
                err.to_string(),
                format!("permission_denied: {ALL_SCOPE_DENIED}")
            );
        }
    }

    /// A connector of another tenant is answered exactly as an unknown one
    /// (same status, same text but for the id the caller sent): no oracle.
    #[sqlx::test(migrations = "../../migrations")]
    async fn another_tenants_connector_is_refused_like_an_unknown_one(pool: sqlx::PgPool) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let other = create_as(
            &state,
            Some(engineer()),
            "connector_failure",
            Some(OTHER_CONNECTOR),
        )
        .await
        .unwrap_err();
        let unknown = create_as(
            &state,
            Some(engineer()),
            "connector_failure",
            Some("conn-ghost"),
        )
        .await
        .unwrap_err();
        assert_eq!(other.status(), StatusCode::NOT_FOUND.as_u16());
        assert_eq!(other.status(), unknown.status());
        assert_eq!(
            other.to_string().replace(OTHER_CONNECTOR, "ID"),
            unknown.to_string().replace("conn-ghost", "ID")
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_administrator_may_save_a_rule_for_all_connectors_and_for_uploads(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("connector_failure", "*")).await;
        create_as(&state, Some(admin()), "connector_failure", Some("*"))
            .await
            .unwrap();
        create_as(&state, Some(admin()), "upload_failure", None)
            .await
            .unwrap();
        create_as(
            &state,
            Some(admin()),
            "connector_failure",
            Some(OTHER_CONNECTOR),
        )
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_new_kinds_fail_closed_with_no_principal_and_other_kinds_are_unchanged(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let err = create_as(&state, None, "connector_failure", Some(OWN_CONNECTOR))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 401);
        // A pipeline kind is not checked: the engineer saves it as before.
        create_as(&state, Some(engineer()), "pipeline_failure", None)
            .await
            .unwrap();
    }

    /// An update may not take over or move a rule the caller could not have
    /// created: not to `*`, not off a `*` rule, not onto another tenant's
    /// connector.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_update_cannot_move_a_rule_the_caller_could_not_have_created(pool: sqlx::PgPool) {
        // The stored rule is for all connectors.
        let (state, _ch) = setup(&pool, ("connector_failure", "*")).await;
        let err = update_as(&state, engineer(), "connector_failure", Some(OWN_CONNECTOR))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403, "taking over an all-connectors rule");
        // The stored rule is for another tenant's connector.
        let (state, _ch) = setup(&pool, ("connector_failure", OTHER_CONNECTOR)).await;
        let err = update_as(&state, engineer(), "connector_failure", Some(OWN_CONNECTOR))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 404, "taking over another tenant's rule");
        // The stored rule is the caller's own; moving it to `*` or to another
        // tenant's connector is refused, moving it within their tenant is not.
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let err = update_as(&state, engineer(), "connector_failure", Some("*"))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403);
        let err = update_as(
            &state,
            engineer(),
            "connector_failure",
            Some(OTHER_CONNECTOR),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status(), 404);
        update_as(
            &state,
            engineer(),
            "connector_repeated_failure",
            Some(OWN_CONNECTOR),
        )
        .await
        .unwrap();
        // Turning a connector rule into a pipeline kind does not escape the
        // check on what it was.
        let (state, _ch) = setup(&pool, ("upload_failure", "*")).await;
        let err = update_as(&state, engineer(), "pipeline_failure", None)
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403);
        // An administrator may do all of it.
        update_as(&state, admin(), "connector_failure", Some("*"))
            .await
            .unwrap();
    }

    async fn listed_as(state: &AppState, principal: Option<Principal>) -> Result<Value, ApiError> {
        list(State(state.clone()), principal.map(Extension))
            .await
            .map(|json| json.0)
            .map_err(|rejection| rejection.0)
    }

    /// `SRC-7` review BLOCKER 1: the list shows a non-administrator only the
    /// connector rules of their tenants.
    #[sqlx::test(migrations = "../../migrations")]
    async fn list_shows_a_caller_only_the_connector_rules_of_their_tenants(pool: sqlx::PgPool) {
        // The rule store answers every read with this one rule.
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let body = listed_as(&state, Some(engineer())).await.unwrap();
        assert_eq!(body["rules"].as_array().unwrap().len(), 1);
        assert_eq!(body["rules"][0]["connector"], OWN_CONNECTOR);

        let (state, _ch) = setup(&pool, ("connector_failure", OTHER_CONNECTOR)).await;
        let body = listed_as(&state, Some(engineer())).await.unwrap();
        assert_eq!(body["rules"], serde_json::json!([]));

        // A connector that no longer exists is hidden from a non-admin.
        let (state, _ch) = setup(&pool, ("connector_failure", "conn-gone")).await;
        let body = listed_as(&state, Some(engineer())).await.unwrap();
        assert_eq!(body["rules"], serde_json::json!([]));

        // `*` and upload rules are an unrestricted caller's alone.
        for existing in [("connector_failure", "*"), ("upload_failure", "*")] {
            let (state, _ch) = setup(&pool, existing).await;
            let body = listed_as(&state, Some(engineer())).await.unwrap();
            assert_eq!(body["rules"], serde_json::json!([]));
            let body = listed_as(&state, Some(admin())).await.unwrap();
            assert_eq!(body["rules"].as_array().unwrap().len(), 1);
        }
    }

    /// Other kinds are returned as before, to anyone and with no principal;
    /// the new kinds with no principal are a 401.
    #[sqlx::test(migrations = "../../migrations")]
    async fn list_returns_other_kinds_untouched_and_refuses_new_kinds_without_a_principal(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("pipeline_failure", "")).await;
        for principal in [None, Some(engineer())] {
            let body = listed_as(&state, principal).await.unwrap();
            assert_eq!(body["rules"].as_array().unwrap().len(), 1);
        }
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        assert_eq!(listed_as(&state, None).await.unwrap_err().status(), 401);
    }
}

#[cfg(test)]
mod run_events_configured {
    //! `SRC-7` D9: `GET /api/alerts` says whether run reports can arrive.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn listed(token: Option<&str>) -> Value {
        let ch = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "rows": 0, "data": [] })),
            )
            .mount(&ch)
            .await;
        let mut env = HashMap::new();
        env.insert("CH_URL".to_owned(), ch.uri());
        if let Some(token) = token {
            env.insert("PIPELINE_RUN_TOKEN".to_owned(), token.to_owned());
        }
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        list(State(state), None).await.expect("listed").0
    }

    #[tokio::test]
    async fn the_flag_is_true_with_a_token_and_false_without_or_with_an_empty_one() {
        let set = listed(Some("a-token")).await;
        assert_eq!(set["runEventsConfigured"], true);
        assert_eq!(set["rules"], json!([]));
        assert_eq!(listed(None).await["runEventsConfigured"], false);
        assert_eq!(listed(Some("")).await["runEventsConfigured"], false);
    }
}
