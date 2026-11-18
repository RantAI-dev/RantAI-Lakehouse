//! Governance tools (T2.1 + T2.5 of the copilot-operations-handover plan):
//! `get_audit_history`, `list_classification_rules`, `list_quality_rules`,
//! `get_cdc_health`, `get_maintenance_metrics`, `draft_policy`,
//! `draft_classification_rule`, `draft_quality_rule`.
//!
//! Every read here except `get_audit_history` calls
//! `routes::governance::get` (the SAME `{kind}` dispatch
//! `GET /api/governance/{kind}` uses) via [`super::response_to_value`]
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
//! `get_audit_history` is the exception (`SEC-20`, plan 100): it reads the
//! trail through `routes::governance::audit_trail` — the one read of the
//! trail `GET /api/governance/audit` uses too — with the person's own
//! scope, so the route and the tool cannot disagree about which rows the
//! caller may see. A headless call answers
//! `{"error": "reading the audit trail needs a signed-in user"}` instead
//! of rows.
//!
//! The three draft tools call the real `routes::governance::create_policy`/
//! `create_quality_rule`/`create_classification_rule` handlers (via
//! [`super::api_result_to_value`]), never a second copy of the insert
//! logic — see the module doc comment on `registry::draft_policy_schema`
//! (and its siblings) for why each of the three genuinely can only ever
//! create a non-active record with today's schema.

use axum::extract::State;
use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str, response_to_value};
use crate::state::AppState;

async fn governance_kind(state: &AppState, kind: &str) -> Value {
    // No principal: this is the headless path, and it does not narrow the
    // rows it reads. A call with no person is not narrowed, the convention
    // `ensure_rule_table` has (`GOV-15` D7). A person's maintenance history is
    // narrowed by `read_kind` when `get_maintenance_metrics` or
    // `list_classification_rules` passes the principal.
    response_to_value(
        crate::routes::governance::read_kind(state, None, &axum::http::HeaderMap::new(), kind)
            .await,
    )
    .await
}

/// `SEC-20` (plan 100): the audit trail, as the person behind the chat may
/// read it.
///
/// The trail is read per tenant: the person's own organisation's rows, plus
/// the deployment's own rows when that organisation owns the shared
/// catalog. The `audit:read` permission is NOT checked here — it stays on
/// the route (`GET /api/governance/audit`), and the assistant's own
/// permission is plan 99's business. The reach is resolved from the
/// person's own tenant, the convention the other governance tools follow
/// (the tool has no request headers of its own).
pub(super) async fn get_audit_history(state: &AppState, principal: Option<&Principal>) -> Value {
    let Some(principal) = principal else {
        // A headless run has no person behind it: it belongs to no tenant
        // and owns no row of the trail.
        return json!({ "error": "reading the audit trail needs a signed-in user" });
    };
    let scope = match crate::routes::audit_scope::scope_for_principal(
        state,
        principal,
        &axum::http::HeaderMap::new(),
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            tracing::warn!(error = %err, "the audit trail could not be read");
            return json!({ "error": "the audit trail is not available" });
        }
    };
    match crate::routes::governance::audit_trail(state, &scope).await {
        Ok(body) => body,
        Err(err) => {
            tracing::warn!(error = %err, "the audit trail could not be read");
            json!({ "error": "the audit trail is not available" })
        }
    }
}

/// `SEC-20` (plan 96): the classification list, as the person behind the chat
/// may read it. The rows name the tables of the shared catalog and the upload
/// tables of every tenant, so a person's list is narrowed to the tables in
/// their reach by [`crate::routes::governance::read_kind`], the rule
/// `GET /api/governance/classification` applies. The tool has no request
/// headers, so the reach is the person's own tenant's, the convention
/// `get_maintenance_metrics` follows. No principal is a headless run and is not
/// narrowed, exactly as it was.
pub(super) async fn list_classification_rules(
    state: &AppState,
    principal: Option<&Principal>,
) -> Value {
    let Some(principal) = principal else {
        return governance_kind(state, "classification").await;
    };
    response_to_value(
        crate::routes::governance::read_kind(
            state,
            Some(principal),
            &axum::http::HeaderMap::new(),
            "classification",
        )
        .await,
    )
    .await
}

pub(super) async fn list_quality_rules(state: &AppState, principal: Option<&Principal>) -> Value {
    response_to_value(
        // The tool has no request headers of its own, so the tenant rule reads
        // the person's own tenant — the same convention `draft_quality_rule`
        // follows below. Refused for a person outside the tenant, and narrowed
        // to the tables their roles read (`GOV-15` D7).
        crate::routes::governance::read_kind(
            state,
            principal,
            &axum::http::HeaderMap::new(),
            "quality",
        )
        .await,
    )
    .await
}

pub(super) async fn get_cdc_health(state: &AppState) -> Value {
    latest_per_slot(governance_kind(state, "replication").await)
}

/// The replication route returns every 15-minute sample (40 rows for one
/// slot on the local stack). The model needs each slot's current state:
/// measured with `qwen3:4b`, the full history produced "the slot is
/// healthy" without ever naming the slot. Each slot's newest sample is
/// kept, with how many samples were seen; an error or an unexpected shape
/// passes through untouched.
fn latest_per_slot(result: Value) -> Value {
    let Some(Value::Array(samples)) = result.get("replicationSlots") else {
        return result;
    };
    let mut latest: Vec<(String, Value, usize)> = Vec::new();
    for sample in samples {
        let slot = sample
            .get("slotName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let checked = sample
            .get("checkedAt")
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(entry) = latest.iter_mut().find(|(s, _, _)| *s == slot) {
            entry.2 += 1;
            let newer = checked
                > entry
                    .1
                    .get("checkedAt")
                    .and_then(Value::as_str)
                    .unwrap_or("");
            if newer {
                entry.1 = sample.clone();
            }
        } else {
            latest.push((slot, sample.clone(), 1));
        }
    }
    let slots: Vec<Value> = latest
        .into_iter()
        .map(|(_, mut sample, n)| {
            if let Value::Object(map) = &mut sample {
                map.insert("samplesSeen".to_owned(), json!(n));
            }
            sample
        })
        .collect();
    json!({ "slots": slots, "note": "each slot's most recent check" })
}

/// `SEC-20` (plan 95): every table's maintenance history, as the person
/// behind the chat may read it.
///
/// The history names every table the Bronze job ever compacted, upload
/// tables included. A person the shared-catalog rule refuses gets that rule's
/// answer and no row; a person it admits gets the rows of the tables in the
/// caller's reach, by the rule Query Studio keeps. The route
/// `GET /api/governance/maintenance` narrows its rows the same way, in
/// [`crate::routes::governance::read_kind`], and this tool reads through it,
/// so the two cannot disagree.
///
/// No principal is a scheduled run, which has no person whose reach to read
/// the history through, so it is refused and reads nothing (`SEC-20`, plan 97;
/// it used to read the whole history).
pub(super) async fn get_maintenance_metrics(
    state: &AppState,
    principal: Option<&Principal>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "reading the maintenance history needs a signed-in user" });
    };
    if let Some(refusal) = super::data::shared_catalog_refusal(state, principal).await {
        return refusal;
    }
    // The tool has no request headers of its own, so the reach is the
    // person's own tenant's, the convention `list_quality_rules` follows.
    response_to_value(
        crate::routes::governance::read_kind(
            state,
            Some(principal),
            &axum::http::HeaderMap::new(),
            "maintenance",
        )
        .await,
    )
    .await
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
        // Serialising our own `Value` (infallible in practice): not an upstream error.
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
        return json!({ "error": "asset is required" });
    }
    let bytes = match serde_json::to_vec(&Value::Object(args.clone())) {
        Ok(bytes) => bytes,
        // Serialising our own `Value` (infallible in practice): not an upstream error.
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

/// `draft_quality_rule` creates the rule through the console's own handler, as
/// the person behind the chat: the table they may not read is refused exactly
/// as the form refuses it, and a table that is not there is refused too
/// (QA `LH-DATA-14-01` F2, `DATA-54`; review round 1 R1). With no signed-in
/// person (a headless run) there is nobody to narrow by, and `TableAccess`
/// documents that such a call is not narrowed.
pub(super) async fn draft_quality_rule(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let name = arg_str(args, "name");
    if name.is_empty() {
        return json!({ "error": "name is required" });
    }
    let bytes = match serde_json::to_vec(&Value::Object(args.clone())) {
        Ok(bytes) => bytes,
        // Serialising our own `Value` (infallible in practice): not an upstream error.
        Err(err) => return json!({ "error": err.to_string() }),
    };
    api_result_to_value(
        crate::routes::governance::create_quality_rule(
            State(state.clone()),
            principal,
            // The tool has no request headers of its own, so the tenant rule
            // reads the person's own tenant (`tenant_scope::resolve`).
            &axum::http::HeaderMap::new(),
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
            json!({ "error": "asset is required" })
        );
        assert_eq!(
            draft_quality_rule(&s, None, &Map::new()).await,
            json!({ "error": "name is required" })
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

#[cfg(test)]
mod latest_slot {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn keeps_each_slots_newest_sample_and_counts_the_rest() {
        let result = json!({ "replicationSlots": [
            { "slotName": "a", "checkedAt": "2026-09-27T01:00:00Z", "status": "ok" },
            { "slotName": "a", "checkedAt": "2026-09-27T01:15:00Z", "status": "lagging" },
            { "slotName": "b", "checkedAt": "2026-09-27T01:10:00Z", "status": "ok" },
        ]});
        let out = latest_per_slot(result);
        let slots = out["slots"].as_array().unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0]["status"], "lagging");
        assert_eq!(slots[0]["samplesSeen"], 2);
        assert_eq!(slots[1]["slotName"], "b");
    }

    #[test]
    fn an_error_passes_through() {
        let err = json!({ "error": "no results yet" });
        assert_eq!(latest_per_slot(err.clone()), err);
    }
}
