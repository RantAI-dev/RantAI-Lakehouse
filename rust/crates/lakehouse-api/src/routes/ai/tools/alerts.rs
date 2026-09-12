//! Alert-rule tools (T1.1 of the copilot-operations-handover plan):
//! `list_alert_rules`, `create_alert_rule`, `update_alert_rule`,
//! `delete_alert_rule`, `run_alert_rule`.
//!
//! Every function here calls straight into the `lakehouse-alerts` crate —
//! the SAME `list_rules`/`save_rule`/`delete_rule`/`run_rules` functions
//! `routes::alerts` calls — rather than re-implementing rule storage or
//! evaluation. `run_alert_rule` also reuses
//! `routes::alerts::smtp_config` so a fired alert is delivered through the
//! exact same `SmtpConfig` the console's `/api/alerts/run` builds.
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
use lakehouse_notify::EmailSender;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::state::AppState;

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
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn create_alert_rule(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let input = parse_input(args);
    match lakehouse_alerts::save_rule(ch, &input, None).await {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn update_alert_rule(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    let input = parse_input(args);
    match lakehouse_alerts::save_rule(ch, &input, Some(&id)).await {
        Ok(rule) => json!({ "ok": true, "rule": rule }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn delete_alert_rule(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    match lakehouse_alerts::delete_rule(ch, &id).await {
        Ok(()) => json!({ "ok": true }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn run_alert_rule(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    let http = reqwest::Client::new();
    let email = EmailSender::new(crate::routes::alerts::smtp_config(&state.config));
    match lakehouse_alerts::run_rules(&state.clickhouse, &http, &email, Some(&id)).await {
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

    #[tokio::test]
    async fn update_delete_and_run_require_id() {
        let ch = &state().clickhouse;
        assert_eq!(
            update_alert_rule(ch, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        assert_eq!(
            delete_alert_rule(ch, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        let s = state();
        assert_eq!(
            run_alert_rule(&s, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
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
