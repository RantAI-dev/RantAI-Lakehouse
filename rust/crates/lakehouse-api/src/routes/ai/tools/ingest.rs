//! Ingestion tools: read and set a connector's ingest spec, discover the
//! source's tables, run an ingest now, read its run history, and rotate a
//! connector's credential.
//!
//! The copilot could register a connector but not make it ingest anything:
//! the ingest spec, discovery, "run now" and credential rotation existed
//! only as console routes. Each tool here calls the SAME route handler the
//! console calls (the `tools::connectors` / `tools::queries` pattern), so
//! the dial validation, the SSRF check at `PUT` time, the derived-credential
//! rule (ADR 0002 Addendum 3) and the probe-before-rotate behaviour are the
//! console's own, not a second copy that could drift.

use std::fmt::Write as _;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use axum::response::IntoResponse;
use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str, response_to_value};
use crate::state::AppState;

/// The connector id argument, or the error the model sees when it is
/// missing.
fn connector_id(args: &Map<String, Value>) -> Result<String, Value> {
    let id = arg_str(args, "id");
    if id.is_empty() {
        Err(json!({ "error": "id is required: the connector id from list_connectors" }))
    } else {
        Ok(id)
    }
}

pub(super) async fn get_ingest_spec(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = match connector_id(args) {
        Ok(id) => id,
        Err(err) => return err,
    };
    api_result_to_value(
        crate::routes::connectors::ingest_spec_get(State(state.clone()), Path(id)).await,
    )
    .await
}

pub(super) async fn set_ingest_spec(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = match connector_id(args) {
        Ok(id) => id,
        Err(err) => return err,
    };
    let mut body = Map::new();
    for key in [
        "adapter",
        "ingestMode",
        "dial",
        "sourceObjects",
        "scheduleCron",
    ] {
        if let Some(value) = args.get(key) {
            body.insert(key.to_owned(), value.clone());
        }
    }
    let bytes = Bytes::from(Value::Object(body).to_string());
    // SEC-14: only the five keys above are forwarded, so the model can never
    // carry a `credential`, and no principal is passed because no credential
    // is. A save that changes where the connector points therefore ends in
    // the route's fixed 409, which `api_result_to_value` hands to the model
    // as its `error`; the change is made in the console.
    api_result_to_value(
        crate::routes::connectors::ingest_spec_put(State(state.clone()), None, Path(id), bytes)
            .await,
    )
    .await
}

pub(super) async fn discover_source(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = match connector_id(args) {
        Ok(id) => id,
        Err(err) => return err,
    };
    let schema = arg_str(args, "schema");
    let uri: Uri = if schema.is_empty() {
        Uri::from_static("/")
    } else {
        match format!("/?schema={}", urlencode(&schema)).parse() {
            Ok(uri) => uri,
            Err(_) => return json!({ "error": "schema is not a valid name" }),
        }
    };
    let Ok(query) = Query::try_from_uri(&uri) else {
        return json!({ "error": "schema is not a valid name" });
    };
    api_result_to_value(
        crate::routes::connectors::discover(State(state.clone()), Path(id), query).await,
    )
    .await
}

pub(super) async fn run_ingest(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = match connector_id(args) {
        Ok(id) => id,
        Err(err) => return err,
    };
    api_result_to_value(crate::routes::connectors::ingest_run(State(state.clone()), Path(id)).await)
        .await
}

pub(super) async fn list_ingest_runs(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = match connector_id(args) {
        Ok(id) => id,
        Err(err) => return err,
    };
    let Ok(uri) = format!("/?connectorId={}", urlencode(&id)).parse::<Uri>() else {
        return json!({ "error": "id is not a valid connector id" });
    };
    let Ok(query) = Query::try_from_uri(&uri) else {
        return json!({ "error": "id is not a valid connector id" });
    };
    response_to_value(
        crate::routes::governance::ingest_runs(State(state.clone()), query)
            .await
            .into_response(),
    )
    .await
}

pub(super) async fn rotate_connector_credential(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = match connector_id(args) {
        Ok(id) => id,
        Err(err) => return err,
    };
    // The route needs the acting principal for its audit event; a
    // scheduled run with no principal has nobody to rotate on behalf of.
    let Some(principal) = principal else {
        return json!({ "error": "rotating a credential needs a signed-in user" });
    };
    let mut body = Map::new();
    for key in ["slot", "source", "kind"] {
        if let Some(value) = args.get(key) {
            body.insert(key.to_owned(), value.clone());
        }
    }
    let bytes = Bytes::from(Value::Object(body).to_string());
    api_result_to_value(
        crate::routes::connectors::rotate_secret(
            State(state.clone()),
            Extension(principal.clone()),
            Path(id),
            bytes,
        )
        .await,
    )
    .await
}

/// Percent-encodes a query value: everything outside the unreserved set.
fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_missing_connector_id_is_named_not_sent() {
        let err = connector_id(&Map::new()).unwrap_err();
        assert!(
            err["error"]
                .as_str()
                .unwrap_or_default()
                .contains("id is required")
        );
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(urlencode("public"), "public");
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
    }
}
