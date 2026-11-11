//! Alert-rule tools (T1.1 of the copilot-operations-handover plan):
//! `list_alert_rules`, `create_alert_rule`, `update_alert_rule`,
//! `delete_alert_rule`, `run_alert_rule`.
//!
//! Every function here calls straight into the `lakehouse-alerts` crate —
//! the SAME `list_rules`/`save_rule`/`delete_rule`/`run_rules` functions
//! `routes::alerts` calls — rather than re-implementing rule storage or
//! evaluation. `run_alert_rule` calls `routes::alerts::run_one` (`BI-40`
//! D4), the one place a rule's run is wired, so a copilot-triggered run
//! reads a chart rule as its owner, saves a fired result as an
//! `alert_instance`, resolves a recovery, evaluates a `Freshness` rule and
//! suppresses delivery for a silenced rule exactly as the HTTP route does —
//! a rule silenced from the console must not page someone again just because
//! the copilot ran it instead.
//!
//! `run_alert_rule` deliberately does NOT go through
//! `routes::alerts::run`/`check_run_token`: that guard's whole point (see
//! its doc comment) is restricting the HTTP route to a shared cron token or
//! a service-identity principal, which is the wrong shape for "a logged-in
//! human, authorized by holding `alert:write`, asks the copilot to run one
//! rule now" — the copilot's OWN gate (`spec.permission = "alert:write"`,
//! enforced by `super::super::gate::decide`) is the intended authorization
//! check for this call, not the cron door.
//!
//! # Saving goes through the route's one path (`BI-37` D1)
//!
//! `create_alert_rule` and `update_alert_rule` do not save on their own: both
//! call [`crate::routes::alerts::save_rule_with_checks`], the function the two
//! HTTP routes call, so the scope guard, the chart-alert save guard (who may
//! watch the chart, the self-only target, restricted dashboards, the owner) and
//! the state reset are the route's. `parse_input` therefore keeps the chart
//! fields (`BI-37` D3): "alert me when this chart drops below 1,000" is a chart
//! alert here too, not a plain mart alert that fails validation.
//!
//! Without `alert:write` the route's rule is the tool's: a chart alert to the
//! caller's **own** email address. With no target the address is read from the
//! caller's account on the server (`BI-37` D3), so "alert me" needs no address
//! from the model.

use lakehouse_alerts::AlertRuleInput;
use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::state::AppState;
use crate::upstream_error;

/// What the model is told when the call came without a signed-in person and
/// needs one (a chart alert with no target: the address would be whose?).
const SIGN_IN: &str = "Sign in to save an alert.";

/// The refusal when a chart alert names no target and the caller's own address
/// cannot be read (`BI-37` D3): the address is never guessed.
const NO_ADDRESS: &str = "Your account's email address could not be read, so this alert cannot be sent to you. Give a target address instead.";

/// Deserializes `args` into [`AlertRuleInput`] — its fields (`name`, `type`,
/// `mart`, `measure`, `agg`, `op`, `threshold`, `metric`, `board`, `chart`,
/// `series`, `filters`, `condition`, `channel`, `target`, `enabled`) already
/// match the tool schema's property names one-for-one, so no field-by-field
/// mapping is needed. Malformed/missing fields simply become `None`/defaults,
/// exactly like the JSON body `routes::alerts::create`/`update` parse —
/// [`save_rule_with_checks`] and `save_rule` do the real validation.
fn parse_input(args: &Map<String, Value>) -> AlertRuleInput {
    serde_json::from_value(Value::Object(args.clone())).unwrap_or_default()
}

/// `BI-37` D3: a chart alert with no target goes to the caller's own email
/// address. The address is read from their account on the server, never
/// guessed, and the channel becomes `email` so the save guard's self-only test
/// (`routes::chart_alerts::authorise_save`) can see it. Any other call is left
/// exactly as the model sent it: a mart, metric or digest rule still needs a
/// `target`, and a chart alert that names one keeps it (the guard then judges
/// it).
///
/// # Errors
///
/// The tool's answer: [`SIGN_IN`] for a chart alert with no target and no
/// caller, [`NO_ADDRESS`] when the account's address cannot be read.
async fn fill_own_address(
    state: &AppState,
    principal: Option<&Principal>,
    input: &mut AlertRuleInput,
) -> Result<(), Value> {
    let is_chart = input
        .chart
        .as_deref()
        .is_some_and(|chart| !chart.trim().is_empty());
    let no_target = input
        .target
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty);
    if !is_chart || !no_target {
        return Ok(());
    }
    let Some(principal) = principal else {
        return Err(json!({ "error": SIGN_IN }));
    };
    let me = principal.id.uuid().to_string();
    let Some(address) = crate::routes::chart_alerts::address_of(state, &me).await else {
        return Err(json!({ "error": NO_ADDRESS }));
    };
    input.channel = Some("email".to_owned());
    input.target = Some(address);
    Ok(())
}

pub(super) async fn list_alert_rules(state: &AppState, principal: Option<&Principal>) -> Value {
    match lakehouse_alerts::list_rules(&state.clickhouse).await {
        // `SRC-7` review BLOCKER 1: the copilot lists through the same
        // filter as `GET /api/alerts`, else it is a way around it. `BI-39` D9:
        // through the same helper too, so a chart rule arrives with its chart's
        // name (`chartTitle`, `dashboardTitle`) and the assistant can say which
        // chart an alert is on.
        Ok(rules) => match crate::routes::alerts::listed_rules(state, principal, rules).await {
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
    let mut input = parse_input(args);
    if let Err(error) = fill_own_address(state, principal, &mut input).await {
        return error;
    }
    // `BI-37` D1: the one save path of `POST /api/alerts`, so the chart guard
    // (`SRC-7` D7's scope check included) is the route's. `SEC-50` slice 2 D7a
    // (M7): that path's create guard carries the tenant gate, so the copilot
    // asks the same question as the console. No `X-Tenant` here: a tool call
    // carries the model's arguments, not the chat's request headers, so the
    // person's first tenant acts (the same fallback as everywhere else).
    let headers = axum::http::HeaderMap::new();
    match crate::routes::alerts::save_rule_with_checks(state, principal, &headers, None, &mut input)
        .await
    {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => json!({ "error": err.to_string() }),
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
    let mut input = parse_input(args);
    if let Err(error) = fill_own_address(state, principal, &mut input).await {
        return error;
    }
    // `BI-37` D1: as `create_alert_rule`; the state reset for a rule that now
    // watches something else happens inside the same function.
    // `SEC-50` slice 2 D7a (M7): the update guard inside it carries the tenant
    // gate, as in `create_alert_rule`.
    let headers = axum::http::HeaderMap::new();
    match crate::routes::alerts::save_rule_with_checks(
        state,
        principal,
        &headers,
        Some(&id),
        &mut input,
    )
    .await
    {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => json!({ "error": err.to_string() }),
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
    // `BI-40` D2/D7: the assistant runs through the same guard as the person's
    // test route. The tool's `alert:write` (`registry.rs`) says it may run *an*
    // alert; this says it may run *this* one — the rule is in what the caller's
    // list shows, the `SRC-7` D7 scope check passes, and it is theirs to manage.
    if let Err(err) = crate::routes::alerts::authorise_rule_test(state, principal, &id).await {
        return json!({ "error": err.to_string() });
    }
    // `BI-40` D4: one helper for one rule, shared with `/api/alerts/run` and the
    // person's test route. The copilot therefore reads a chart rule as its owner
    // and saves its fired instance as the route does — the two gaps this tool
    // had when it carried its own copy of the run wiring.
    // `SEC-50` slice 2 T7 (M6): a person's call is gated as the person, even
    // when it arrives through the assistant (`SEC-50` R4). No `X-Tenant`
    // here: a tool call carries the model's arguments, not the chat's request
    // headers, so the person's FIRST tenant acts — the same fallback the
    // person's test route takes with no header, and the same mapping
    // `export_gold_mart` uses for its own tool call. A service identity or a
    // call with no principal is a job with no user and acts as the catalog's
    // tenant (D3), through the same one helper.
    let headers = axum::http::HeaderMap::new();
    let actor = crate::routes::gold::gold_export_gate_actor(principal, &headers);
    match crate::routes::alerts::run_one(state, Some(&id), actor).await {
        Ok(results) => json!({ "ran": results.len(), "results": results }),
        Err(err) => json!({ "error": err.to_string() }),
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

    /// `BI-40` D4: `run_alert_rule` goes through `routes::alerts::run_one`, which
    /// reads a chart rule as its owner. Before that, the tool carried its own
    /// copy of the run wiring and answered a chart rule with
    /// `run_rules`' `CHART_SKIPPED` sentence — the rule was never read.
    #[tokio::test]
    async fn running_a_chart_rule_from_the_assistant_does_not_answer_chart_skipped() {
        use lakehouse_auth::{PermissionSet, PrincipalId};
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        // The tool's own gate is `alert:write`; the caller owns the rule, so
        // D2.1 shows it to them (the same C3 pair the console's guard uses).
        let owner = uuid::Uuid::new_v4();
        let caller = Principal {
            id: PrincipalId::User(owner),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: PermissionSet::parse("alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        let server = wiremock::MockServer::start().await;
        let rule = json!({
            "id": "al_chart", "name": "Visitors in Bali", "type": "alert",
            "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "100",
            "board": "", "channel": "email", "target": "owner@test.invalid",
            "enabled": "1", "created_at": "2026-11-02 00:00:00", "severity": "",
            "pipeline": "", "metric": "", "connector": "", "asset": "", "chart": "c_kpi",
            "series": "", "condition": "above", "owner_id": owner.to_string(),
            "filters_json": "[]",
        });
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [rule], "rows": 1 })),
            )
            .mount(&server)
            .await;
        let mut state = state();
        state.clickhouse = std::sync::Arc::new(lakehouse_clickhouse::ChClient::new(
            server.uri(),
            "default".to_owned(),
            String::new(),
        ));
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("al_chart"));
        let result = run_alert_rule(&state, Some(&caller), &args).await;
        let text = result.to_string();
        assert!(
            !text.contains("chart alerts are evaluated by /api/alerts/run"),
            "a chart rule is read now, not skipped as before: {text}"
        );
        assert_eq!(result["ran"], json!(1), "{text}");
        assert!(result["results"][0]["skipped"].is_string(), "{text}");
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
    /// into `AlertRuleInput` with no manual mapping, chart fields included
    /// (`BI-37` D3: they are no longer dropped here).
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
        args.insert("chart".to_owned(), json!("c_kpi"));
        args.insert("series".to_owned(), json!("visitors"));
        args.insert("condition".to_owned(), json!("below"));
        args.insert(
            "filters".to_owned(),
            json!([{ "column": "region", "values": ["Bali"] }]),
        );
        let input = parse_input(&args);
        assert_eq!(input.name.as_deref(), Some("Kunjungan turun"));
        assert_eq!(input.kind.as_deref(), Some("alert"));
        assert_eq!(input.mart.as_deref(), Some("mart_wisman"));
        assert_eq!(input.op.as_deref(), Some("<"));
        assert_eq!(input.threshold, Some(100.0));
        // `BI-37` D3: kept, not dropped.
        assert_eq!(input.chart.as_deref(), Some("c_kpi"));
        assert_eq!(input.series.as_deref(), Some("visitors"));
        assert_eq!(input.condition.as_deref(), Some("below"));
        assert_eq!(
            input.filters.as_ref().map(Vec::len),
            Some(1),
            "the page's filters ride along, as `export_chart` takes them"
        );
    }

    /// `BI-37` D3: a chart alert with no target is filled from the caller's
    /// own account; a mart alert (no chart) is left as sent, so its save still
    /// asks for a target.
    #[tokio::test]
    async fn only_a_chart_alert_with_no_target_is_given_the_callers_address() {
        let state = state();
        let mut chart = AlertRuleInput {
            chart: Some("c_kpi".to_owned()),
            ..AlertRuleInput::default()
        };
        // No caller: the address would be whose? The call is refused.
        let refused = fill_own_address(&state, None, &mut chart).await;
        assert_eq!(refused, Err(json!({ "error": SIGN_IN })));

        let mut mart = AlertRuleInput {
            mart: Some("mart_wisman".to_owned()),
            target: None,
            ..AlertRuleInput::default()
        };
        assert!(fill_own_address(&state, None, &mut mart).await.is_ok());
        assert_eq!(mart.target, None, "a mart alert still needs its own target");

        // A chart alert that names a target keeps it: the save guard judges it.
        let mut named = AlertRuleInput {
            chart: Some("c_kpi".to_owned()),
            target: Some("someone@example.test".to_owned()),
            ..AlertRuleInput::default()
        };
        assert!(fill_own_address(&state, None, &mut named).await.is_ok());
        assert_eq!(
            named.target.as_deref(),
            Some("someone@example.test"),
            "a named target is never replaced"
        );
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

    /// `BI-39` D2/D3: saving through the assistant goes through the route's
    /// save guard (`BI-37` D1), so the caller owns the rule it saves — before
    /// `BI-39` a mart rule reached the store with no owner at all.
    #[sqlx::test(migrations = "../../migrations")]
    async fn create_alert_rule_stores_the_caller_as_the_owner(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_auth::{PermissionSet, PrincipalId};
        use uuid::Uuid;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// The catalog's own tenant, so the save's tenant gate admits the
        /// statement (`SEC-50` slice 2 M11).
        const CATALOG_TENANT: &str = "11111111-1111-4111-8111-000000000001";

        let server = MockServer::start().await;
        // Every write succeeds; a read of the rule answers the row just saved.
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [{
                    "id": "al_new",
                    "name": "Visitors over the line",
                    "type": "alert",
                    "mart": "mart_x",
                    "measure": "visitors",
                    "agg": "sum",
                    "op": ">",
                    "threshold": "100",
                    "board": "",
                    "channel": "email",
                    "target": "rina@test.invalid",
                    "enabled": "1",
                    "created_at": "",
                    "severity": "",
                    "owner_id": Uuid::from_u128(11).to_string(),
                }],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok.\n"))
            .mount(&server)
            .await;

        let role = Uuid::new_v4();
        // Role names are unique and the seeded deployment has its own "Data
        // Engineer", whose permissions this test is not about.
        let role_name = format!("Data Engineer {}", Uuid::new_v4());
        sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, 'dashboard:read, alert:write', 'test')")
            .bind(role)
            .bind(&role_name)
            .execute(&pool)
            .await?;
        let caller_id = Uuid::from_u128(11);
        sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Rina', 'rina', 'rina@test.invalid', 'active')")
            .bind(caller_id)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
            .bind(caller_id)
            .bind(role)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
            .bind(caller_id)
            .bind(CATALOG_TENANT.parse::<Uuid>().expect("a seeded tenant id"))
            .execute(&pool)
            .await?;
        let person = Principal {
            id: PrincipalId::User(caller_id),
            tenant_ids: vec![CATALOG_TENANT.parse().expect("a seeded tenant id")],
            display_name: "Rina".to_owned(),
            permissions: PermissionSet::parse("dashboard:read, alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: vec!["Data Engineer".to_owned()],
        };

        let mut args = Map::new();
        args.insert("name".to_owned(), json!("Visitors over the line"));
        args.insert("type".to_owned(), json!("alert"));
        args.insert("mart".to_owned(), json!("mart_x"));
        args.insert("measure".to_owned(), json!("visitors"));
        args.insert("agg".to_owned(), json!("sum"));
        args.insert("op".to_owned(), json!(">"));
        args.insert("threshold".to_owned(), json!(100.0));
        args.insert("channel".to_owned(), json!("email"));
        args.insert("target".to_owned(), json!("rina@test.invalid"));

        let state = state_in(&pool, &server.uri());
        let result = create_alert_rule(&state, Some(&person), &args).await;

        assert_eq!(result["ok"], json!(true), "{result}");
        let inserts: Vec<String> = server
            .received_requests()
            .await
            .expect("mock records requests")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|body| body.contains("INSERT INTO console.alert_rule"))
            .collect();
        assert_eq!(inserts.len(), 1, "{inserts:?}");
        assert!(
            inserts[0].contains(&caller_id.to_string()),
            "the caller is the stored owner: {}",
            inserts[0]
        );
        Ok(())
    }
}
