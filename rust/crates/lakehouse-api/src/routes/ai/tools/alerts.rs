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
use lakehouse_clickhouse::ChClient;
use lakehouse_notify::{EmailSender, WebhookSender};
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
    serde_json::from_value(Value::Object(args.clone())).unwrap_or_default()
}

pub(super) async fn list_alert_rules(ch: &ChClient) -> Value {
    match lakehouse_alerts::list_rules(ch).await {
        Ok(rules) => json!({ "rules": rules }),
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn create_alert_rule(
    ch: &ChClient,
    webhooks: &WebhookSender,
    args: &Map<String, Value>,
) -> Value {
    let input = parse_input(args);
    // SEC-10: `save_rule` checks the webhook target with the same sender and
    // message as the console route.
    match lakehouse_alerts::save_rule(ch, webhooks, &input, None).await {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => upstream_error::alert(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn update_alert_rule(
    ch: &ChClient,
    webhooks: &WebhookSender,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let input = parse_input(args);
    match lakehouse_alerts::save_rule(ch, webhooks, &input, Some(&id)).await {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => upstream_error::alert(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn delete_alert_rule(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    match lakehouse_alerts::delete_rule(ch, &id).await {
        Ok(()) => json!({ "ok": true }),
        Err(err) => upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
    }
}

pub(super) async fn run_alert_rule(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // SEC-10: the sender checks every webhook target and never follows a redirect.
    let http = crate::webhook_guard::sender(&state.config);
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
        &crate::routes::alerts::ApiSqlGate {
            pg: state.pg.as_deref(),
            ch: &state.clickhouse,
        },
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
    async fn failing_clickhouse() -> (wiremock::MockServer, ChClient) {
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 60. DB::Exception: Table planted-marker-table-x doesn't exist (version 0.0.0)",
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        (server, ch)
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
        let (_server, ch) = failing_clickhouse().await;
        assert_fixed_failure(&list_alert_rules(&ch).await);
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("r1"));
        assert_fixed_failure(&delete_alert_rule(&ch, &args).await);
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
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let result = create_alert_rule(
            &ch,
            &crate::webhook_guard::sender(&state().config),
            &Map::new(),
        )
        .await;
        assert!(result["error"].is_string(), "{result}");
        assert!(result.get("errorId").is_none(), "{result}");
    }

    #[tokio::test]
    async fn update_delete_and_run_require_id() {
        let ch = &state().clickhouse;
        let webhooks = crate::webhook_guard::sender(&state().config);
        assert_eq!(
            update_alert_rule(ch, &webhooks, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        assert_eq!(
            delete_alert_rule(ch, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        let s = state();
        assert_eq!(
            run_alert_rule(&s, &Map::new()).await,
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
}
