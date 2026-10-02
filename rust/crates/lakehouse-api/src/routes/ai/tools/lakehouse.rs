//! Iceberg and storage tools: list the Iceberg tables, describe one (schema,
//! partitioning, snapshots), read and set its maintenance policy, and read
//! storage capacity.
//!
//! These answered only in the console (Table Maintenance, Capacity). Each
//! tool calls the console's own route handler, so the identifier
//! validation, the bounded Lakekeeper fan-out and the error classification
//! are the same, and a tool sees exactly what the console page shows.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::state::AppState;

/// Namespaces listed per call, so a warehouse with many namespaces cannot
/// flood the model's context.
const MAX_NAMESPACES: usize = 20;

fn ident_arg(args: &Map<String, Value>, key: &str) -> Result<String, Value> {
    let value = arg_str(args, key);
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Err(json!({ "error": format!("{key} must be an Iceberg identifier (letters, digits, _)") }))
    } else {
        Ok(value)
    }
}

async fn tables_in(state: &AppState, namespace: &str) -> Value {
    let Ok(uri) = format!("/?namespace={namespace}").parse::<Uri>() else {
        return json!({ "error": "namespace is invalid" });
    };
    let Ok(query) = Query::try_from_uri(&uri) else {
        return json!({ "error": "namespace is invalid" });
    };
    api_result_to_value(crate::routes::lakehouse::tables(State(state.clone()), query).await).await
}

/// With `namespace`: that namespace's tables. Without: every namespace
/// with its tables, up to [`MAX_NAMESPACES`].
pub(super) async fn list_iceberg_tables(state: &AppState, args: &Map<String, Value>) -> Value {
    if !arg_str(args, "namespace").is_empty() {
        let namespace = match ident_arg(args, "namespace") {
            Ok(ns) => ns,
            Err(err) => return err,
        };
        return tables_in(state, &namespace).await;
    }
    let Ok(query) = Query::try_from_uri(&Uri::from_static("/")) else {
        return json!({ "error": "the namespace list could not be requested" });
    };
    let namespaces = api_result_to_value(
        crate::routes::lakehouse::namespaces(State(state.clone()), query).await,
    )
    .await;
    if namespaces.get("error").is_some() {
        return namespaces;
    }
    let names: Vec<String> = namespaces
        .get("namespaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|ns| ns.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    if names.is_empty() {
        return json!({ "namespaces": [], "note": "the Iceberg warehouse has no namespaces" });
    }
    let mut out = Vec::new();
    for name in names.iter().take(MAX_NAMESPACES) {
        let tables = tables_in(state, name).await;
        out.push(
            json!({ "namespace": name, "tables": tables.get("tables").cloned().unwrap_or(tables) }),
        );
    }
    let mut body = json!({ "namespaces": out });
    if names.len() > MAX_NAMESPACES {
        body["note"] = json!(format!(
            "{} of {} namespaces shown; pass namespace to list one",
            MAX_NAMESPACES,
            names.len()
        ));
    }
    body
}

pub(super) async fn describe_iceberg_table(state: &AppState, args: &Map<String, Value>) -> Value {
    let (namespace, table) = match (ident_arg(args, "namespace"), ident_arg(args, "table")) {
        (Ok(ns), Ok(t)) => (ns, t),
        (Err(err), _) | (_, Err(err)) => return err,
    };
    let mut detail = api_result_to_value(
        crate::routes::lakehouse::table_detail(State(state.clone()), Path((namespace, table)))
            .await,
    )
    .await;
    // Snapshot history can run to thousands of entries; the model needs
    // the current state and the recent history, not every one.
    if let Some(Value::Array(snapshots)) = detail.get_mut("snapshots") {
        let total = snapshots.len();
        if total > 20 {
            let recent = snapshots.split_off(total - 20);
            *snapshots = recent;
            detail["snapshotsNote"] = json!(format!("the 20 most recent of {total} snapshots"));
        }
    }
    detail
}

pub(super) async fn get_table_maintenance(state: &AppState, args: &Map<String, Value>) -> Value {
    let (namespace, table) = match (ident_arg(args, "namespace"), ident_arg(args, "table")) {
        (Ok(ns), Ok(t)) => (ns, t),
        (Err(err), _) | (_, Err(err)) => return err,
    };
    api_result_to_value(
        crate::routes::lakehouse::maintenance(State(state.clone()), Path((namespace, table))).await,
    )
    .await
}

pub(super) async fn set_table_maintenance(state: &AppState, args: &Map<String, Value>) -> Value {
    let (namespace, table) = match (ident_arg(args, "namespace"), ident_arg(args, "table")) {
        (Ok(ns), Ok(t)) => (ns, t),
        (Err(err), _) | (_, Err(err)) => return err,
    };
    let mut body = Map::new();
    for key in [
        "snapshotsToKeep",
        "orphanAgeHours",
        "compactSmallFiles",
        "schedule",
    ] {
        if let Some(value) = args.get(key) {
            body.insert(key.to_owned(), value.clone());
        }
    }
    body.entry("compactSmallFiles").or_insert(json!(false));
    api_result_to_value(
        crate::routes::lakehouse::set_maintenance_policy(
            State(state.clone()),
            Path((namespace, table)),
            Bytes::from(Value::Object(body).to_string()),
        )
        .await,
    )
    .await
}

pub(super) async fn get_capacity(state: &AppState) -> Value {
    api_result_to_value(crate::routes::lakehouse::capacity(State(state.clone())).await).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_identifier_argument_refuses_anything_but_word_characters() {
        let mut args = Map::new();
        args.insert("namespace".to_owned(), json!("bronze"));
        assert_eq!(ident_arg(&args, "namespace").unwrap(), "bronze");
        args.insert("namespace".to_owned(), json!("bronze; drop"));
        assert!(ident_arg(&args, "namespace").is_err());
        assert!(ident_arg(&Map::new(), "table").is_err());
    }
}
