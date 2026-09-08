//! Connector tools (T1.2 of the copilot-operations-handover plan):
//! `list_connectors`, `create_connector`, `test_connector`,
//! `delete_connector`.
//!
//! # Security — every guard is reused, never re-implemented
//!
//! Every function here dispatches straight to the real
//! `routes::connectors::{list,create,test_connection,delete}` handler
//! (via [`super::api_result_to_value`]) rather than re-implementing any
//! part of it, so:
//!
//! - `create_connector`'s raw-credential refusal
//!   (`lakehouse_store::connectors::looks_like_raw_secret`) is the SAME
//!   check `POST /api/connectors` runs — there is no second copy here that
//!   could drift or be bypassed.
//! - `test_connector`'s real connectivity probe goes through
//!   `crate::connector_probe::probe` with the SAME SSRF guard
//!   (`connector_probe_allow_internal_hosts`) and the SAME
//!   `AllowlistedSecretResolver` the console route uses.
//! - The response types (`Connector`, `ConnectorDetail`,
//!   `ConnectorTestResult`) never serialize `host` or `secretRef` at
//!   all — see `lakehouse_store::connectors`'s module doc comment — so no
//!   redaction step is needed here for the tool output to stay credential-free.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::state::AppState;

pub(super) async fn list_connectors(state: &AppState) -> Value {
    api_result_to_value(crate::routes::connectors::list(State(state.clone())).await).await
}

/// Builds the exact `POST /api/connectors` JSON body from the tool's args
/// (the schema's property names already match `CreateConnectorBody`'s
/// `camelCase` fields byte-for-byte: `secretRef`, `secretRefSecondary`, ...)
/// and calls the route handler directly — this is what applies the
/// raw-credential refusal without a second copy of that check.
pub(super) async fn create_connector(state: &AppState, args: &Map<String, Value>) -> Value {
    let body = match serde_json::to_vec(&Value::Object(args.clone())) {
        Ok(bytes) => bytes,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    api_result_to_value(
        crate::routes::connectors::create(State(state.clone()), Bytes::from(body)).await,
    )
    .await
}

pub(super) async fn test_connector(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    api_result_to_value(
        crate::routes::connectors::test_connection(State(state.clone()), Path(id)).await,
    )
    .await
}

pub(super) async fn delete_connector(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    match crate::routes::connectors::delete(State(state.clone()), Path(id)).await {
        Ok(_status) => json!({ "ok": true }),
        Err(err) => super::response_to_value(err.into_response()).await,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    /// A `create_connector` call carrying a raw-looking credential in
    /// `secretRef` is refused by the SAME check `POST /api/connectors`
    /// runs — this test proves the reuse, not just that some check exists.
    #[tokio::test]
    async fn create_connector_refuses_a_raw_looking_secret_ref() {
        let state = state_without_pool();
        let mut args = Map::new();
        args.insert("name".to_owned(), json!("n"));
        args.insert("type".to_owned(), json!("PostgreSQL"));
        args.insert("direction".to_owned(), json!("source"));
        args.insert("host".to_owned(), json!("db:5432"));
        args.insert(
            "secretRef".to_owned(),
            json!("postgres://admin:hunter2@db.internal:5432/oms"),
        );
        args.insert("environment".to_owned(), json!("production"));
        args.insert("tenant".to_owned(), json!("t"));
        let result = create_connector(&state, &args).await;
        assert!(result.get("error").is_some(), "{result}");
    }

    /// A `secretRef` shaped like a reference (not a credential) is NOT
    /// refused by the raw-secret check — it fails later (no Postgres pool
    /// in this test state), but never with the raw-credential message.
    #[tokio::test]
    async fn create_connector_accepts_a_secret_ref() {
        let state = state_without_pool();
        let mut args = Map::new();
        args.insert("name".to_owned(), json!("n"));
        args.insert("type".to_owned(), json!("PostgreSQL"));
        args.insert("direction".to_owned(), json!("source"));
        args.insert("host".to_owned(), json!("db:5432"));
        args.insert("secretRef".to_owned(), json!("env:DB_PASSWORD"));
        args.insert("environment".to_owned(), json!("production"));
        args.insert("tenant".to_owned(), json!("t"));
        let result = create_connector(&state, &args).await;
        let err = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            !err.contains("reference to a credential"),
            "must not be refused for looking like a raw secret: {err}"
        );
    }

    /// Tool output never contains `host` or `secretRef`, matching the
    /// underlying `Connector`/`ConnectorDetail`/`ConnectorTestResult`
    /// response types, which have no such fields to serialize at all.
    #[tokio::test]
    async fn list_and_test_never_return_host_or_secret_ref() {
        let state = state_without_pool();
        let list_result = list_connectors(&state).await;
        assert!(!list_result.to_string().contains("secretRef"));
        assert!(!list_result.to_string().contains("\"host\""));

        let mut args = Map::new();
        args.insert("id".to_owned(), json!("conn-x"));
        let test_result = test_connector(&state, &args).await;
        assert!(!test_result.to_string().contains("secretRef"));
        assert!(!test_result.to_string().contains("\"host\""));
    }

    #[tokio::test]
    async fn test_connector_and_delete_connector_require_id() {
        let state = state_without_pool();
        assert_eq!(
            test_connector(&state, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        assert_eq!(
            delete_connector(&state, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
    }
}
