//! Thin wrapper over [`lakehouse_store::audit`] for the copilot's own audit
//! trail (T0.3/T0.4 of the copilot-operations-handover plan): exactly one
//! `audit_event` row per gate decision or executed tool call, written from
//! both the chat loop ([`super::chat`]) and `POST /api/ai/tool`
//! ([`super::tool_call`]).
//!
//! # Redaction is mandatory, and lives here
//!
//! [`lakehouse_store::audit::insert`] writes `args` verbatim — by design,
//! it has no idea which tool it's auditing or what a given argument means.
//! [`redact`] is this module's answer to plan invariant 5 ("never store
//! `secretRef` values, never store SQL result rows"): every call to
//! [`record`] redacts `args` before they ever reach [`NewAuditEvent`]. See
//! [`redact`]'s own doc comment for the exact rules.
//!
//! # Never breaks the caller
//!
//! [`record`] returns `()`, not a `Result` — a failed audit write (no
//! Postgres configured, a transient database error, ...) is logged and
//! swallowed, never propagated. An audit sink that could break the chat
//! loop or `/api/ai/tool` on its own failure would make governance the
//! copilot's single point of failure, which is worse than an occasional
//! missing audit row.

use lakehouse_auth::Principal;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self, NewAuditEvent};
use serde_json::{Map, Value};

/// Object keys that look like they hold a credential, matched
/// case-insensitively as a substring of the key name — so `secretRef`,
/// `apiKey`, `accessToken`, and a bare `password` are all caught by one of
/// these four needles.
const SECRET_KEY_NEEDLES: [&str; 4] = ["secret", "password", "token", "key"];

/// Longest string value kept verbatim in an audited `args` payload; any
/// longer string is truncated to this many characters with a
/// `"…[truncated]"` marker appended. This is what keeps something
/// SQL-result-shaped (or just a very long free-text argument) from
/// bloating `audit_event.args` — it has no idea what a "SQL result row"
/// looks like, so it treats every over-long string the same way.
const MAX_STRING_LEN: usize = 500;

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_NEEDLES
        .iter()
        .any(|needle| lower.contains(needle))
}

fn truncate_string(s: &str) -> Value {
    if s.chars().count() > MAX_STRING_LEN {
        let truncated: String = s.chars().take(MAX_STRING_LEN).collect();
        Value::String(format!("{truncated}…[truncated]"))
    } else {
        Value::String(s.to_owned())
    }
}

/// Redacts a tool call's `args` before they are stored in `audit_event`,
/// recursively:
///
/// 1. Any object key that [`is_secret_key`] matches has its value replaced
///    with the literal string `"[redacted]"` — regardless of that value's
///    own shape (a string, a nested object, ...), since the point is "this
///    key's VALUE must never be legible in the audit log", not "this key's
///    value happens to be a plain string today".
/// 2. Any other string value longer than [`MAX_STRING_LEN`] characters is
///    truncated (see [`truncate_string`]).
/// 3. Arrays and non-secret object values are redacted element-by-element/
///    field-by-field. Numbers, bools, and `null` pass through unchanged.
///
/// This function only ever sees a tool call's INPUT `args` — callers of
/// [`record`] must never pass a tool's RESULT through this path (plan
/// invariant 5's "never store SQL result rows" half is enforced simply by
/// never handing a result to this function at all, not by trying to
/// recognise a "result-shaped" value here).
#[must_use]
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (key, val) in map {
                if is_secret_key(key) {
                    out.insert(key.clone(), Value::String("[redacted]".to_owned()));
                } else {
                    out.insert(key.clone(), redact(val));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(s) => truncate_string(s),
        other => other.clone(),
    }
}

/// Best-effort `(resource_kind, resource_id)` for a tool call, so
/// `audit_event` can be queried by "what did this touch" — derived from
/// the call's args and, when the tool just created something server-side
/// (e.g. `create_chart`'s generated id), its result. `(None, None)` for a
/// tool with no single targeted resource (`run_sql`, every `list_*`, ...).
///
/// Only ever reads `result["id"]`/`result["runId"]` — small, known-shape
/// identifiers a tool always includes on success, never the bulk of the
/// result (e.g. `run_sql`'s `rows`) — so this stays compatible with
/// invariant 5 even though it is handed the result, unlike [`redact`].
#[must_use]
pub fn resource_for(
    tool_name: &str,
    args: &Map<String, Value>,
    result: &Value,
) -> (Option<&'static str>, Option<String>) {
    let str_field = |key: &str| -> Option<String> {
        result
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| args.get(key).and_then(Value::as_str))
            .map(str::to_owned)
    };
    match tool_name {
        "create_chart" | "update_chart" | "delete_chart" => (Some("chart"), str_field("id")),
        "create_board" => (Some("board"), str_field("id")),
        "trigger_lakehouse_build" => (Some("pipeline"), str_field("runId")),
        _ => (None, None),
    }
}

/// Writes one `audit_event` row for a copilot gate decision or tool
/// execution. Every argument here maps directly onto
/// [`NewAuditEvent`]'s fields except `args`, which is [`redact`]-ed first.
///
/// `principal_kind` is always `"copilot"` — this module is only ever
/// called from the copilot's own dispatch paths ([`super::chat`],
/// [`super::tool_call`]); a headless "digital employee" run (T3.2, not
/// this task) or a human's own action elsewhere in the console uses a
/// different `principal_kind` through a different call site.
///
/// # Never fails visibly
///
/// See the module doc comment: on any error (including `pg: None`, i.e.
/// no Postgres configured for this deployment), this logs a `tracing::warn!`
/// and returns — never a panic, never a propagated error. Every caller in
/// the chat loop / `/api/ai/tool` must keep responding to the user even
/// when this degrades.
#[allow(
    clippy::too_many_arguments,
    reason = "one flat record of what happened; \
    a builder would only hide that every field here is genuinely independent \
    input, not add real clarity"
)]
pub async fn record(
    pg: Option<&PgPool>,
    principal: Option<&Principal>,
    session_id: Option<&str>,
    action: &str,
    resource_kind: Option<&str>,
    resource_id: Option<&str>,
    args: &Value,
    outcome: &str,
    detail: Option<&str>,
    run_id: Option<&str>,
    approval_id: Option<&str>,
) {
    let Some(pool) = pg else {
        tracing::warn!(
            action,
            outcome,
            "copilot audit: no Postgres pool configured, skipping audit_event write"
        );
        return;
    };
    let event = NewAuditEvent {
        principal_id: principal.map(|p| p.id.uuid().to_string()),
        principal_kind: Some("copilot".to_owned()),
        actor_label: principal.map(|p| p.display_name.clone()),
        action: action.to_owned(),
        resource_kind: resource_kind.map(str::to_owned),
        resource_id: resource_id.map(str::to_owned),
        args: Some(redact(args)),
        outcome: outcome.to_owned(),
        detail: detail.map(str::to_owned),
        run_id: run_id.map(str::to_owned),
        approval_id: approval_id.map(str::to_owned),
        session_id: session_id.map(str::to_owned),
    };
    if let Err(err) = audit::insert(pool, event).await {
        tracing::warn!(
            %err,
            action,
            outcome,
            "copilot audit: failed to write audit_event; continuing without audit"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn redact_replaces_secret_shaped_keys_regardless_of_casing() {
        let input = json!({
            "secretRef": "env:DATABASE_URL",
            "password": "hunter2",
            "apiKey": "sk-abc123",
            "accessToken": "tok-xyz",
            "title": "ok to keep",
        });
        let redacted = redact(&input);
        assert_eq!(redacted["secretRef"], json!("[redacted]"));
        assert_eq!(redacted["password"], json!("[redacted]"));
        assert_eq!(redacted["apiKey"], json!("[redacted]"));
        assert_eq!(redacted["accessToken"], json!("[redacted]"));
        assert_eq!(redacted["title"], json!("ok to keep"));
    }

    #[test]
    fn redact_truncates_long_strings() {
        let long = "x".repeat(MAX_STRING_LEN + 50);
        let input = json!({ "sql": long });
        let redacted = redact(&input);
        let out = redacted["sql"].as_str().unwrap();
        assert!(out.ends_with("…[truncated]"));
        assert!(out.len() < long.len());
    }

    #[test]
    fn redact_leaves_short_strings_and_non_strings_untouched() {
        let input = json!({ "limit": 10, "flag": true, "title": "short" });
        let redacted = redact(&input);
        assert_eq!(redacted, input);
    }

    #[test]
    fn redact_recurses_into_nested_objects_and_arrays() {
        let input = json!({
            "outer": { "secretRef": "env:X", "keep": "y" },
            "list": [{ "token": "abc" }, { "keep": "z" }],
        });
        let redacted = redact(&input);
        assert_eq!(redacted["outer"]["secretRef"], json!("[redacted]"));
        assert_eq!(redacted["outer"]["keep"], json!("y"));
        assert_eq!(redacted["list"][0]["token"], json!("[redacted]"));
        assert_eq!(redacted["list"][1]["keep"], json!("z"));
    }

    #[test]
    fn resource_for_chart_tools_prefers_result_id_then_args_id() {
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("from-args"));
        let result = json!({ "id": "from-result" });
        assert_eq!(
            resource_for("update_chart", &args, &result),
            (Some("chart"), Some("from-result".to_owned()))
        );
        assert_eq!(
            resource_for("update_chart", &args, &json!({})),
            (Some("chart"), Some("from-args".to_owned()))
        );
    }

    #[test]
    fn resource_for_unrelated_tool_is_none() {
        assert_eq!(
            resource_for("run_sql", &Map::new(), &json!({})),
            (None, None)
        );
    }

    /// The degradation invariant (T0.3/T0.4): with no Postgres pool at
    /// all, `record` must not panic — it logs and returns.
    #[tokio::test]
    async fn record_degrades_gracefully_with_no_postgres_pool() {
        record(
            None,
            None,
            None,
            "create_board",
            Some("board"),
            Some("b-1"),
            &json!({ "name": "Test" }),
            "executed",
            None,
            None,
            None,
        )
        .await;
        // Reaching this line without panicking is the assertion.
    }
}
