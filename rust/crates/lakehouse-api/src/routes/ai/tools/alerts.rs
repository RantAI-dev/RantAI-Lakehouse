//! Alert-rule tools (T1.1 of the copilot-operations-handover plan):
//! `list_alert_rules`, `create_alert_rule`, `update_alert_rule`,
//! `delete_alert_rule`, `run_alert_rule`.
//!
//! Every function here calls straight into the `lakehouse-alerts` crate —
//! the SAME `list_rules`/`save_rule`/`delete_rule`/`run_rules` functions
//! `routes::alerts` calls — rather than re-implementing rule storage or
//! evaluation. `run_alert_rule` also reuses `routes::alerts::smtp_config`
//! so a fired alert is delivered through the exact same `SmtpConfig` the
//! console's `/api/alerts/run` builds, and `routes::alerts::ApiFreshnessSource`/
//! `ApiSilenceSource` (WS5 item C1) so a copilot-triggered run evaluates
//! `Freshness` rules and suppresses delivery for a silenced rule exactly
//! like the HTTP route does — a rule silenced from the console must not
//! page someone again just because the copilot ran it instead.
//!
//! `run_alert_rule` deliberately does NOT go through
//! `routes::alerts::run`/`check_run_token`: that guard's whole point (see
//! its doc comment) is restricting the HTTP route to a shared cron token or
//! a service-identity principal, which is the wrong shape for "a logged-in
//! human, authorized by holding `alert:write`, asks the copilot to run one
//! rule now" — the copilot's OWN gate (`spec.permission = "alert:write"`,
//! enforced by `super::super::gate::decide`) is the intended authorization
//! check for this call, not the cron door.

use lakehouse_alerts::AlertRuleInput;
use lakehouse_auth::Principal;
use lakehouse_notify::EmailSender;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::state::AppState;
use crate::upstream_error;

/// Deserializes `args` directly into [`AlertRuleInput`] — its fields
/// (`name`, `type`, `mart`, `measure`, `agg`, `op`, `threshold`, `board`,
/// `channel`, `target`, `enabled`) already match the tool schema's
/// property names one-for-one, so no field-by-field mapping is needed.
/// Malformed/missing fields simply become `None`/defaults, exactly like
/// the JSON body `routes::alerts::create`/`update` parse — [`save_rule`]
/// itself does the real validation.
fn parse_input(args: &Map<String, Value>) -> AlertRuleInput {
    let mut input: AlertRuleInput =
        serde_json::from_value(Value::Object(args.clone())).unwrap_or_default();
    // BI-19 part C (planner decision 4, 2026-10-26): chart alerts are not an
    // assistant tool yet (`AI-11`). This path saves through `save_rule` without
    // the console route's C3 checks (who may view the chart, the self-only
    // target, restricted dashboards), so the chart fields are dropped here
    // rather than honoured; the rule then fails as a plain threshold alert.
    input.chart = None;
    input.series = None;
    input.filters = None;
    input.condition = None;
    input
}

pub(super) async fn list_alert_rules(state: &AppState, principal: Option<&Principal>) -> Value {
    match lakehouse_alerts::list_rules(&state.clickhouse).await {
        // `SRC-7` review BLOCKER 1: the copilot lists through the same
        // filter as `GET /api/alerts`, else it is a way around it.
        Ok(rules) => match crate::routes::alerts::visible_rules(state, principal, rules).await {
            Ok(rules) => json!({ "rules": rules }),
            Err(err) => json!({ "error": err.to_string() }),
        },
        // SEC-11: the database's own text goes to the log under a reference id.
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn create_alert_rule(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let input = parse_input(args);
    // `SRC-7` D7: the copilot saves through `save_rule` directly, so it must
    // pass the same scope check the HTTP route does; otherwise it would be a
    // way around it.
    // BI-19 B1 review R6 (assistant path): the same create guard, which now also
    // refuses a digest of a board the caller cannot see or that is restricted.
    // `SEC-50` slice 2 D7a (M7): the same guard as the console route, tenant
    // gate included. No `X-Tenant` here: a tool call carries the model's
    // arguments, not the chat's request headers, so the person's first tenant
    // acts (the same fallback as everywhere else).
    let headers = axum::http::HeaderMap::new();
    if let Err(err) =
        crate::routes::alerts::authorise_rule_create(state, principal, &headers, &input).await
    {
        return json!({ "error": err.to_string() });
    }
    // SEC-10: `save_rule` checks the webhook target with the same sender and
    // message as the console route.
    let webhooks = state.webhook_sender();
    match lakehouse_alerts::save_rule(&state.clickhouse, &webhooks, &input, None).await {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => upstream_error::alert(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn update_alert_rule(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let input = parse_input(args);
    // `SRC-7` D7: as in `create_alert_rule`.
    // `SEC-50` slice 2 D7a (M7): the same guard, tenant gate included.
    let headers = axum::http::HeaderMap::new();
    if let Err(err) =
        crate::routes::alerts::authorise_rule_update(state, principal, &headers, &id, &input).await
    {
        return json!({ "error": err.to_string() });
    }
    // SEC-10: as in `create_alert_rule`.
    let webhooks = state.webhook_sender();
    match lakehouse_alerts::save_rule(&state.clickhouse, &webhooks, &input, Some(&id)).await {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => upstream_error::alert(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn delete_alert_rule(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // `SRC-7` review BLOCKER 2: as `DELETE /api/alerts`; the copilot would
    // otherwise delete another tenant's connector rule.
    if let Err(err) = crate::routes::alerts::authorise_existing_rule(state, principal, &id).await {
        return json!({ "error": err.to_string() });
    }
    match lakehouse_alerts::delete_rule(&state.clickhouse, &id).await {
        Ok(()) => {
            // BI-19 C review SF-7: the same state clean-up as `DELETE /api/alerts`.
            crate::routes::alerts::forget_state(state, &id).await;
            json!({ "ok": true })
        }
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn run_alert_rule(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // SEC-10: the sender checks every webhook target and never follows a redirect.
    let http = state.webhook_sender();
    let email = EmailSender::new(crate::routes::alerts::smtp_config(&state.config));
    // Same real `FreshnessSource`/`SilenceSource` wiring as
    // `routes::alerts::run` (WS5 item C1) — reused here, not
    // reimplemented, so a copilot-triggered run also evaluates `Freshness`
    // rules and never pages someone about an incident they already
    // silenced. `AppState::iceberg` is read once, through the lock, guard
    // dropped immediately.
    let cached_iceberg = state.iceberg.read().await.clone();
    let freshness_source = match (state.pg.as_deref(), cached_iceberg) {
        (Some(pg), Some(iceberg)) => {
            Some(crate::routes::alerts::ApiFreshnessSource { pg, iceberg })
        }
        _ => None,
    };
    let silence_source = state
        .pg
        .as_deref()
        .map(|pg| crate::routes::alerts::ApiSilenceSource { pg });
    let state_source = state
        .pg
        .as_deref()
        .map(|pg| crate::routes::alerts::ApiStateSource { pg });
    // `SEC-50` slice 2 T7 (M6): a person's call is gated as the person, even
    // when it arrives through the assistant (`SEC-50` R4). No `X-Tenant`
    // here: a tool call carries the model's arguments, not the chat's
    // request headers, so the person's FIRST tenant acts — the same fallback
    // `tenant_scope::resolve` uses when no header is sent, and the same
    // mapping `export_gold_mart` uses for its own tool call. A service
    // identity or a call with no principal is a job with no user, and acts
    // as the catalog's tenant (D3).
    let headers = axum::http::HeaderMap::new();
    let actor = crate::routes::gold::gold_export_gate_actor(principal, &headers);
    match lakehouse_alerts::run_rules(
        &state.clickhouse,
        &http,
        &email,
        Some(&id),
        freshness_source
            .as_ref()
            .map(|s| s as &dyn lakehouse_alerts::FreshnessSource),
        silence_source
            .as_ref()
            .map(|s| s as &dyn lakehouse_alerts::SilenceSource),
        state_source
            .as_ref()
            .map(|s| s as &dyn lakehouse_alerts::StateSource),
        &crate::routes::alerts::ApiSqlGate { state, actor },
    )
    .await
    {
        Ok(results) => json!({ "ran": results.len(), "results": results }),
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    /// SEC-11: a `ClickHouse` that fails every request with a planted marker.
    async fn failing_clickhouse() -> (wiremock::MockServer, AppState) {
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 60. DB::Exception: Table planted-marker-table-x doesn't exist (version 0.0.0)",
            ))
            .mount(&server)
            .await;
        let mut state = state();
        state.clickhouse = std::sync::Arc::new(lakehouse_clickhouse::ChClient::new(
            server.uri(),
            "default".to_owned(),
            String::new(),
        ));
        (server, state)
    }

    fn assert_fixed_failure(result: &Value) {
        let text = result.to_string();
        assert!(
            !text.contains("planted-marker"),
            "database text leaked: {text}"
        );
        assert!(!text.contains("version 0.0.0"), "{text}");
        assert!(result["errorId"].is_string(), "no reference id: {text}");
        assert_eq!(result["error"], "The database request failed.");
    }

    #[tokio::test]
    async fn a_failing_alert_tool_reports_a_fixed_message_and_a_reference() {
        let (_server, state) = failing_clickhouse().await;
        assert_fixed_failure(&list_alert_rules(&state, None).await);
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("r1"));
        // `SRC-7`: without an unrestricted principal the delete first reads
        // the rule for the scope check; that failure has its own fixed
        // message (no database text), so only the leak is asserted here.
        let deleted = delete_alert_rule(&state, None, &args).await;
        assert!(
            !deleted.to_string().contains("planted-marker"),
            "database text leaked: {deleted}"
        );
    }

    /// Validation text in `AlertError` is ours and stays as written.
    #[tokio::test]
    async fn an_alert_validation_failure_keeps_its_own_message() {
        // `save_rule` ensures its table first, then validates; a healthy
        // `ClickHouse` gets us to the validation message.
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;
        let mut state = state();
        state.clickhouse = std::sync::Arc::new(lakehouse_clickhouse::ChClient::new(
            server.uri(),
            "default".to_owned(),
            String::new(),
        ));
        let result = create_alert_rule(&state, None, &Map::new()).await;
        assert!(result["error"].is_string(), "{result}");
        assert!(result.get("errorId").is_none(), "{result}");
    }

    #[tokio::test]
    async fn update_delete_and_run_require_id() {
        // SRC-7 task 6: `update_alert_rule` takes the state and the caller's
        // principal now (the same scope check as the HTTP route); the
        // assertion is unchanged.
        assert_eq!(
            update_alert_rule(&state(), None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        // `SRC-7` review BLOCKER 2: `delete_alert_rule` takes the state and
        // the principal for the same check as the route; expected value
        // unchanged.
        assert_eq!(
            delete_alert_rule(&state(), None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        let s = state();
        assert_eq!(
            run_alert_rule(&s, None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
    }

    /// `parse_input`'s field names round-trip a tool call's args straight
    /// into `AlertRuleInput` with no manual mapping.
    #[test]
    fn parse_input_reads_every_field_by_name() {
        let mut args = Map::new();
        args.insert("name".to_owned(), json!("Kunjungan turun"));
        args.insert("type".to_owned(), json!("alert"));
        args.insert("mart".to_owned(), json!("mart_wisman"));
        args.insert("measure".to_owned(), json!("kunjungan"));
        args.insert("agg".to_owned(), json!("sum"));
        args.insert("op".to_owned(), json!("<"));
        args.insert("threshold".to_owned(), json!(100.0));
        args.insert("channel".to_owned(), json!("webhook"));
        args.insert("target".to_owned(), json!("https://example.com/hook"));
        let input = parse_input(&args);
        assert_eq!(input.name.as_deref(), Some("Kunjungan turun"));
        assert_eq!(input.kind.as_deref(), Some("alert"));
        assert_eq!(input.mart.as_deref(), Some("mart_wisman"));
        assert_eq!(input.op.as_deref(), Some("<"));
        assert_eq!(input.threshold, Some(100.0));
    }

    /// An [`AppState`] over `pool` and a `ClickHouse` at `ch_url`, naming the
    /// catalog's own tenant (`SEC-50` slice 2 M11), built through `Config`
    /// (the production path).
    fn state_in(pool: &sqlx::PgPool, ch_url: &str) -> AppState {
        let options = pool.connect_options();
        let url = format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database"),
        );
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), url);
        env.insert("CH_URL".to_owned(), ch_url.to_owned());
        env.insert(
            "CATALOG_TENANT_ID".to_owned(),
            "11111111-1111-4111-8111-000000000001".to_owned(),
        );
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    /// `SEC-50` slice 2 T7 (M6, R4): a person's `run_alert_rule` is gated as
    /// that person. A member of a tenant that does not own the shared
    /// catalog is refused with `SEC-20`'s own sentence, and the mart is never
    /// read — the copilot is not a way around the gate.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_alert_rule_reads_as_the_person_who_asked(pool: sqlx::PgPool) -> sqlx::Result<()> {
        use lakehouse_auth::{PermissionSet, PrincipalId};
        use uuid::Uuid;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// The tenant the seeded deployment holds besides the catalog's.
        const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
        /// `SEC-20`'s refusal of `serving.mart_x`, byte for byte.
        const REFUSAL: &str =
            "Table not found or not available to your organisation: serving.mart_x";

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [{
                    "id": "al-mine",
                    "name": "Mine",
                    "type": "alert",
                    "mart": "mart_x",
                    "measure": "v",
                    "agg": "sum",
                    "op": ">",
                    "threshold": "10",
                    "board": "",
                    "channel": "email",
                    "target": "ops@example.com",
                    "enabled": "1",
                    "created_at": "",
                    "severity": "high",
                }],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [{ "v": "999" }], "rows": 1
            })))
            .mount(&server)
            .await;

        let mut args = Map::new();
        args.insert("id".to_owned(), json!("al-mine"));
        let state = state_in(&pool, &server.uri());
        let person = Principal {
            id: PrincipalId::User(Uuid::from_u128(9)),
            tenant_ids: vec![OTHER_TENANT.parse().expect("a seeded tenant id")],
            display_name: "Other tenant".to_owned(),
            permissions: PermissionSet::parse("alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };

        let result = run_alert_rule(&state, Some(&person), &args).await;

        assert_eq!(result["results"][0]["skipped"], REFUSAL, "{result}");
        assert_eq!(result["results"][0]["fired"], false, "{result}");
        let asked = server
            .received_requests()
            .await
            .expect("mock records requests");
        assert!(
            !asked
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body).contains("FROM serving.")),
            "the refusal precedes the engine: the mart must not be read"
        );
        Ok(())
    }
}
