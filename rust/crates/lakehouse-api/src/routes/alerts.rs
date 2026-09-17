//! `GET/POST/PUT/DELETE /api/alerts`, `GET/POST /api/alerts/run` — threshold
//! alerts & scheduled digests.
//!
//! Ports `src/app/api/alerts/route.ts` and
//! `src/app/api/alerts/run/route.ts`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_alerts::{AlertKind, AlertRule, AlertRuleInput, FreshnessSource, SilenceSource};
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

/// `GET /api/alerts` — list every alert & digest rule.
///
/// The `TypeScript` handler's `catch` returns a 500 with `e.message`
/// (`alerts/route.ts`'s `GET`), unlike `POST`/`PUT` which return 400 for
/// the same kind of failure — the status code here depends on which route
/// caught the error, not on the error's own type, so it is chosen at each
/// call site rather than baked into a single `From` conversion.
///
/// # Errors
///
/// Returns a 500 [`ApiError::Internal`] on a `ClickHouse` failure.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let rules = lakehouse_alerts::list_rules(&state.clickhouse)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "rules": rules })))
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

/// `POST /api/alerts` — create a rule.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body or a
/// validation failure (see `lakehouse_alerts::save_rule`) — matching the
/// `TypeScript`'s single `catch` around both.
pub async fn create(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let input = parse_body(&body)?;
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
/// `id`, or a validation failure — matching the `TypeScript`.
pub async fn update(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let input = parse_body(&body)?;
    let Some(id) = input.id.clone() else {
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
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

/// A fixed, kind-derived `AlertItem.source` label for a fired rule's
/// persisted instance — never invented per-rule text, just which engine
/// produced it.
fn fired_source(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::Alert => "Alert rules",
        AlertKind::Freshness => "Freshness monitoring",
        AlertKind::Digest => "Digest", // never reached: callers filter Digest out before this
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

    let results = lakehouse_alerts::run_rules(
        &state.clickhouse,
        &http,
        &email,
        query.id.as_deref(),
        freshness_source.as_ref().map(|s| s as &dyn FreshnessSource),
        silence_source.as_ref().map(|s| s as &dyn SilenceSource),
    )
    .await
    .map_err(|err| ApiError::Internal(err.to_string()))?;

    // Persist every fired, non-Digest result as an `alert_instance` row,
    // best-effort (WS5 item C1) — see `persist_fired_results`'s doc
    // comment.
    if let Some(pg) = state.pg.as_deref() {
        persist_fired_results(pg, &state.clickhouse, &results).await;
    }

    Ok(ApiJson(json!({ "ran": results.len(), "results": results })))
}

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
}
