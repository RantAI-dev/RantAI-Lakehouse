//! Per-tool implementations, grouped by domain, and the dispatch that
//! calls one by name.
//!
//! - [`data`] — `run_sql`, `list_datasets`, `describe_dataset`,
//!   `get_lineage`, `get_quality`, `describe_mart`.
//! - [`ask`] — `ask_user`, the question with options the console shows as
//!   buttons.
//! - [`dashboards`] — `create_chart`, `update_chart`, `delete_chart`,
//!   `create_board`, `list_boards`, `list_charts`, `list_sql_sources`,
//!   `suggest_dashboard`.
//! - [`pipelines`] — `trigger_lakehouse_build`, `get_build_status`, plus
//!   the Tier 1 pipeline-operations tools (T1.3).
//! - [`alerts`] — Tier 1 alert-rule tools (T1.1).
//! - [`connectors`] — Tier 1 connector tools (T1.2).
//! - [`queries`] — Tier 1 saved-query tools (T1.4).
//! - [`governance`] — Tier 2 governance reads + draft tools (T2.1, T2.5).
//! - [`ops`] — Tier 2 workload tools (T2.3).
//! - [`gold`] — Tier 2 Gold export tools (T2.4). `run_bronze_maintenance`
//!   (T2.2) lives in [`pipelines`] alongside the other `DgClient::launch_run`
//!   caller (`trigger_build`), rather than a one-tool `maintenance` module.

mod alerts;
mod ask;
mod connectors;
mod dashboards;
// `pub(in crate::routes)` (not merely `mod`) so `routes::lineage` — a
// SIBLING module of `routes::ai`, not a descendant — can reuse
// `data::CATALOG_UNION`, the SAME bronze-catalog query `list_datasets`
// already runs, rather than re-deriving it.
pub(in crate::routes) mod data;
mod gold;
mod governance;
mod ingest;
mod lakehouse;
mod ops;
mod pipelines;
mod queries;

use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::registry;
use crate::error::ApiResult;
use crate::state::AppState;

/// Turns an axum [`Response`] into the [`Value`] a copilot tool returns,
/// by reading its (always-`ApiJson`, hence always-JSON) body back out.
///
/// This is how the Tier 1 tools (T1.1-T1.4) REUSE the actual console route
/// handlers — `routes::pipelines::{list,runs,trigger,pause,resume,
/// cancel_run,retry_run}` already return a plain [`Response`], built with
/// [`crate::json::ApiJson`] — rather than re-implementing their branching
/// (authored-vs-Dagster dispatch, 404/409/503 mapping, ...) a second time
/// for the copilot. Calling the handler directly means a guard or a bug fix
/// applied to the console route is automatically applied to the copilot
/// tool too, with no risk of the two drifting apart.
///
/// A malformed/non-JSON body (never true for an `ApiJson` response in
/// practice) falls back to `{}` rather than panicking — `unwrap`/`expect`
/// are denied outside test modules, and a copilot tool must always return
/// *some* JSON `Value`, not fail the whole chat turn over its own
/// response-parsing.
pub(super) async fn response_to_value(resp: Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

/// Same as [`response_to_value`], for the handlers (`routes::connectors::*`,
/// `routes::query::run`, `routes::alerts::{list,create,update,delete}`)
/// that return [`ApiResult<T>`] rather than a bare [`Response`] — `T`'s own
/// [`IntoResponse`] impl (`ApiJson<T>`, or a tuple like
/// `(StatusCode, ApiJson<T>)`) and [`crate::error::ApiRejection`]'s both
/// produce a JSON body, so routing either arm through
/// [`response_to_value`] gives the same "call the real handler, read its
/// JSON back" reuse as pipelines' bare-`Response` handlers.
pub(super) async fn api_result_to_value<T: IntoResponse>(result: ApiResult<T>) -> Value {
    match result {
        Ok(ok) => response_to_value(ok.into_response()).await,
        Err(err) => response_to_value(err.into_response()).await,
    }
}

/// Dispatch one tool call by name, matching `runTool` in `ai-tools.ts`.
/// Unknown tool names return `{"error": "unknown tool: <name>"}` rather
/// than failing the request — the LLM sees the error and can recover.
///
/// The unknown-name check goes through [`registry::find`] first — the
/// single source of truth for which names are real tools — so this
/// `match`'s arms and the registry can never quietly drift apart: a name
/// [`registry::find`] recognises but this `match` has no arm for hits the
/// `unreachable!` below instead of silently falling through to "tool tak
/// dikenal", which would be a bug worth crashing a test over, not masking.
pub(in crate::routes) async fn run_tool(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    if registry::find(name).is_none() {
        return json!({ "error": format!("unknown tool: {name}") });
    }
    // The switch hides `ask_user` from the chat's tool list, but a model can
    // name a tool it was not given and the gate does not know the switch, so
    // the refusal sits here, where `/api/ai/tool` and the chat both run tools.
    // PR review fix (SHOULD-FIX): with `AI_ASK_BACK=false` the call still ran.
    if name == registry::ASK_USER && !state.config.ai_ask_back {
        return json!({ "error": "asking the user is switched off" });
    }
    let ch = &state.clickhouse;
    match name {
        "run_sql" => data::run_sql(state, principal, args).await,
        "ask_user" => ask::ask_user(args),
        "list_datasets" => data::list_datasets(ch, args).await,
        "lakehouse_overview" => data::lakehouse_overview(state, principal).await,
        "describe_dataset" => data::describe_dataset(ch, args).await,
        "get_lineage" => data::get_lineage(state, principal, args).await,
        "get_quality" => data::get_quality(ch).await,
        "trigger_lakehouse_build" => pipelines::trigger_build(state, principal).await,
        "get_build_status" => pipelines::get_build_status(&state.dagster).await,
        "describe_mart" => data::describe_mart(ch, args).await,
        "create_chart" => dashboards::create_chart(ch, args, None).await,
        "update_chart" => dashboards::update_chart(ch, args).await,
        "create_board" => dashboards::create_board(ch, args).await,
        "list_boards" => dashboards::list_boards(ch).await,
        "suggest_dashboard" => dashboards::suggest_dashboard(ch).await,
        "list_charts" => dashboards::list_charts(ch).await,
        "list_sql_sources" => dashboards::list_sql_sources(ch).await,
        "delete_chart" => dashboards::delete_chart(ch, args).await,
        "list_alert_rules" => alerts::list_alert_rules(ch).await,
        "create_alert_rule" => alerts::create_alert_rule(ch, args).await,
        "update_alert_rule" => alerts::update_alert_rule(ch, args).await,
        "delete_alert_rule" => alerts::delete_alert_rule(ch, args).await,
        "run_alert_rule" => alerts::run_alert_rule(state, args).await,
        "list_connectors" => connectors::list_connectors(state, principal).await,
        "create_connector" => connectors::create_connector(state, principal, args).await,
        "test_connector" => connectors::test_connector(state, principal, args).await,
        "delete_connector" => connectors::delete_connector(state, principal, args).await,
        "list_pipelines" => pipelines::list_pipelines(state, principal).await,
        "list_pipeline_runs" => pipelines::list_pipeline_runs(state, args).await,
        "trigger_pipeline" => pipelines::trigger_pipeline(state, principal, args).await,
        "retry_pipeline_run" => pipelines::retry_pipeline_run(state, args).await,
        "pause_pipeline" => pipelines::pause_pipeline(state, principal, args).await,
        "resume_pipeline" => pipelines::resume_pipeline(state, principal, args).await,
        "cancel_pipeline_run" => pipelines::cancel_pipeline_run(state, args).await,
        "save_query" => queries::save_query(state, args).await,
        "list_saved_queries" => queries::list_saved_queries(state).await,
        "run_saved_query" => queries::run_saved_query(state, principal, args).await,
        "get_audit_history" => governance::get_audit_history(state).await,
        "list_classification_rules" => governance::list_classification_rules(state).await,
        "list_quality_rules" => governance::list_quality_rules(state).await,
        "get_cdc_health" => governance::get_cdc_health(state).await,
        "get_maintenance_metrics" => governance::get_maintenance_metrics(state).await,
        "run_bronze_maintenance" => pipelines::run_bronze_maintenance(&state.dagster).await,
        "list_workloads" => ops::list_workloads(state).await,
        "kill_query" => ops::kill_query(state, args).await,
        "export_gold_mart" => gold::export_gold_mart(state, principal, args).await,
        "get_gold_export" => gold::get_gold_export(state, args).await,
        "draft_policy" => governance::draft_policy(state, args).await,
        "draft_classification_rule" => governance::draft_classification_rule(state, args).await,
        "draft_quality_rule" => governance::draft_quality_rule(state, args).await,
        "get_ingest_spec" => ingest::get_ingest_spec(state, args).await,
        "set_ingest_spec" => ingest::set_ingest_spec(state, args).await,
        "discover_source" => ingest::discover_source(state, args).await,
        "run_ingest" => ingest::run_ingest(state, args).await,
        "list_ingest_runs" => ingest::list_ingest_runs(state, args).await,
        "rotate_connector_credential" => {
            ingest::rotate_connector_credential(state, principal, args).await
        }
        "create_pipeline" => pipelines::create_pipeline(state, principal, args).await,
        "get_pipeline" => pipelines::get_pipeline(state, args).await,
        "mark_pipeline_ready" => pipelines::mark_pipeline_ready(state, args).await,
        "list_iceberg_tables" => lakehouse::list_iceberg_tables(state, args).await,
        "describe_iceberg_table" => lakehouse::describe_iceberg_table(state, args).await,
        "get_table_maintenance" => lakehouse::get_table_maintenance(state, args).await,
        "set_table_maintenance" => lakehouse::set_table_maintenance(state, args).await,
        "get_capacity" => lakehouse::get_capacity(state).await,
        other => {
            unreachable!(
                "registry::find recognised {other:?} but run_tool has no dispatch arm for it"
            )
        }
    }
}

/// Reads `args[key]` as a string, matching `String(args[key] ?? "")`: a
/// missing/`null` value becomes `""`, a non-string `Value` is
/// stringified rather than rejected (mirroring `TypeScript`'s implicit
/// coercion at this same boundary in `ai-tools.ts`).
fn arg_str(args: &Map<String, Value>, key: &str) -> String {
    args.get(key)
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn dispatch_test_state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    /// D3.5: every registered tool reaches a real dispatch arm (never the
    /// `unreachable!` in [`run_tool`]'s `other` arm, and never the
    /// "unknown tool" branch), and an unregistered name gets exactly
    /// that refusal shape. See `tests/fixtures/tool_schemas.json` and the
    /// characterization test in `super::super::tests` for the pre-refactor
    /// baseline this must keep matching.
    #[tokio::test]
    async fn run_tool_dispatches_every_registered_tool_and_refuses_unknown() {
        let state = dispatch_test_state();
        for spec in registry::TOOLS {
            let result = run_tool(&state, None, spec.name, &Map::new()).await;
            if let Some(err) = result.get("error").and_then(Value::as_str) {
                assert!(
                    !err.starts_with("unknown tool"),
                    "{} unexpectedly hit the unknown-tool dispatch arm: {err}",
                    spec.name
                );
            }
        }
        let unknown = run_tool(&state, None, "not_a_real_tool", &Map::new()).await;
        assert_eq!(unknown, json!({ "error": "unknown tool: not_a_real_tool" }));
    }

    // PR review fix (SHOULD-FIX): `AI_ASK_BACK=false` only hid `ask_user` from
    // the tool list, so a model that named it anyway still ran it.
    #[tokio::test]
    async fn ask_user_runs_with_the_switch_on_and_is_refused_with_it_off() {
        let mut args = Map::new();
        args.insert("term".to_owned(), json!("hotel"));
        args.insert("question".to_owned(), json!("Which one?"));
        args.insert("options".to_owned(), json!(["hotel_a", "hotel_b"]));
        let state_with = |switch: &str| {
            let env = HashMap::from([("AI_ASK_BACK".to_owned(), switch.to_owned())]);
            AppState::new(Config::from_map(&env).unwrap())
        };

        let on = run_tool(&state_with("true"), None, "ask_user", &args).await;
        assert_eq!(on["asked"], json!(true), "{on}");

        let off = run_tool(&state_with("false"), None, "ask_user", &args).await;
        assert_eq!(off, json!({ "error": "asking the user is switched off" }));
        // Only `ask_user` is behind the switch.
        let other = run_tool(&state_with("false"), None, "list_datasets", &Map::new()).await;
        assert_ne!(other["error"], json!("asking the user is switched off"));
    }

    #[test]
    fn arg_str_defaults_missing_or_null_to_empty_string() {
        let mut args = Map::new();
        args.insert("a".to_owned(), Value::Null);
        assert_eq!(arg_str(&args, "a"), "");
        assert_eq!(arg_str(&args, "missing"), "");
    }

    #[test]
    fn arg_str_returns_string_value_verbatim() {
        let mut args = Map::new();
        args.insert("a".to_owned(), Value::String("hi".to_owned()));
        assert_eq!(arg_str(&args, "a"), "hi");
    }
}
