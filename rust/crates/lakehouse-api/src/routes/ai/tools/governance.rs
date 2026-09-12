//! Governance tools (T2.1 + T2.5 of the copilot-operations-handover plan):
//! `get_audit_history`, `list_classification_rules`, `list_quality_rules`,
//! `get_cdc_health`, `get_maintenance_metrics`, `draft_policy`,
//! `draft_classification_rule`, `draft_quality_rule`.
//!
//! Every read here calls `routes::governance::get` (the SAME `{kind}`
//! dispatch `GET /api/governance/{kind}` uses) via [`super::response_to_value`]
//! — including its existing degrade-cleanly behaviour: `get_cdc_health`
//! (`kind = "replication"`) and `get_maintenance_metrics`
//! (`kind = "maintenance"`) read `ClickHouse` tables that only exist once
//! their Dagster job has run at least once (see `routes::governance.rs`
//! doc comments on `maintenance`/`replication`); on a deployment where
//! that hasn't happened yet, the query fails and `get` already turns that
//! into a 503 `{"error": ...}` body, which `response_to_value` reads back
//! as a normal tool-result `{"error": ...}` — no special-casing needed
//! here.
//!
//! The three draft tools call the real `routes::governance::create_policy`/
//! `create_quality_rule`/`create_classification_rule` handlers (via
//! [`super::api_result_to_value`]), never a second copy of the insert
//! logic — see the module doc comment on `registry::draft_policy_schema`
//! (and its siblings) for why each of the three genuinely can only ever
//! create a non-active record with today's schema.

use axum::extract::{Path, State};
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str, response_to_value};
use crate::state::AppState;

async fn governance_kind(state: &AppState, kind: &str) -> Value {
    response_to_value(
        crate::routes::governance::get(State(state.clone()), Path(kind.to_owned())).await,
    )
    .await
}

pub(super) async fn get_audit_history(state: &AppState) -> Value {
    governance_kind(state, "audit").await
}

pub(super) async fn list_classification_rules(state: &AppState) -> Value {
    governance_kind(state, "classification").await
}

pub(super) async fn list_quality_rules(state: &AppState) -> Value {
    governance_kind(state, "quality").await
}

pub(super) async fn get_cdc_health(state: &AppState) -> Value {
    governance_kind(state, "replication").await
}

pub(super) async fn get_maintenance_metrics(state: &AppState) -> Value {
    governance_kind(state, "maintenance").await
}

/// Builds the `POST /api/governance/policies` body from the tool's args,
/// forcing `activate: false` regardless of anything the model passed —
/// `draft_policy` must NEVER be able to create an already-`"ready"`
/// policy; only a human, in the console, activates one.
pub(super) async fn draft_policy(state: &AppState, args: &Map<String, Value>) -> Value {
    let mut body = args.clone();
    body.insert("activate".to_owned(), Value::Bool(false));
    let bytes = match serde_json::to_vec(&Value::Object(body)) {
        Ok(bytes) => bytes,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    api_result_to_value(
        crate::routes::governance::create_policy(
            State(state.clone()),
            axum::body::Bytes::from(bytes),
        )
        .await,
    )
    .await
}

pub(super) async fn draft_classification_rule(
    state: &AppState,
    args: &Map<String, Value>,
) -> Value {
    let asset = arg_str(args, "asset");
    if asset.is_empty() {
        return json!({ "error": "asset wajib diisi" });
    }
    let bytes = match serde_json::to_vec(&Value::Object(args.clone())) {
        Ok(bytes) => bytes,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    api_result_to_value(
        crate::routes::governance::create_classification_rule(
            State(state.clone()),
            axum::body::Bytes::from(bytes),
        )
        .await,
    )
    .await
}

pub(super) async fn draft_quality_rule(state: &AppState, args: &Map<String, Value>) -> Value {
    let name = arg_str(args, "name");
    if name.is_empty() {
        return json!({ "error": "name wajib diisi" });
    }
    let bytes = match serde_json::to_vec(&Value::Object(args.clone())) {
        Ok(bytes) => bytes,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    api_result_to_value(
        crate::routes::governance::create_quality_rule(
            State(state.clone()),
            axum::body::Bytes::from(bytes),
        )
        .await,
    )
    .await
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

    /// A `DATABASE_URL` that cannot possibly be a live Postgres — unlike
    /// [`state`], which defaults to the local dev compose Postgres and
    /// WILL actually write a row if a test reaches an insert (see
    /// `draft_policy_forces_draft_even_when_activate_is_requested` below,
    /// which needs the opposite: a real pool, in the isolated integration
    /// test instead — `tests/tier2_governance_drafts.rs`).
    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    #[tokio::test]
    async fn draft_classification_rule_and_draft_quality_rule_require_their_field() {
        let s = state();
        assert_eq!(
            draft_classification_rule(&s, &Map::new()).await,
            json!({ "error": "asset wajib diisi" })
        );
        assert_eq!(
            draft_quality_rule(&s, &Map::new()).await,
            json!({ "error": "name wajib diisi" })
        );
    }

    /// With no Postgres pool reachable at all, `draft_policy` fails
    /// cleanly (an error `Value`) rather than panicking — the real proof
    /// that it forces `status = "draft"` even when `activate: true` is
    /// requested lives in `tests/tier2_governance_drafts.rs`
    /// (`draft_policy_always_creates_a_draft_status_row`), which runs
    /// against a real, isolated-per-test Postgres database and reads the
    /// stored row back.
    #[tokio::test]
    async fn draft_policy_without_a_pool_fails_rather_than_silently_activating() {
        let s = state_without_pool();
        let mut args = Map::new();
        args.insert("name".to_owned(), json!("n"));
        args.insert("kind".to_owned(), json!("Row filter"));
        args.insert("subjects".to_owned(), json!("All analysts"));
        args.insert("resources".to_owned(), json!("tenant-scoped tables"));
        args.insert("effect".to_owned(), json!("Permit with obligation"));
        args.insert("activate".to_owned(), json!(true));
        let result = draft_policy(&s, &args).await;
        assert!(result.get("error").is_some(), "{result}");
    }
}
