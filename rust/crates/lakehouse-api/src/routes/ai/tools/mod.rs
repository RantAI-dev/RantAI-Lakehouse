//! Per-tool implementations, grouped by domain, and the dispatch that
//! calls one by name.
//!
//! - [`data`] — `run_sql`, `list_datasets`, `describe_dataset`,
//!   `get_lineage`, `get_quality`, `describe_mart`.
//! - [`dashboards`] — `create_chart`, `update_chart`, `delete_chart`,
//!   `create_board`, `list_boards`, `list_charts`, `suggest_dashboard`.
//! - [`pipelines`] — `trigger_lakehouse_build`, `get_build_status`.

mod dashboards;
mod data;
mod pipelines;

use serde_json::{Map, Value, json};

use super::registry;
use crate::state::AppState;

/// Dispatch one tool call by name, matching `runTool` in `ai-tools.ts`.
/// Unknown tool names return `{"error": "tool tak dikenal: <name>"}` rather
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
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    if registry::find(name).is_none() {
        return json!({ "error": format!("tool tak dikenal: {name}") });
    }
    let ch = &state.clickhouse;
    match name {
        "run_sql" => data::run_sql(ch, args).await,
        "list_datasets" => data::list_datasets(ch, args).await,
        "describe_dataset" => data::describe_dataset(ch, args).await,
        "get_lineage" => data::get_lineage(ch, args).await,
        "get_quality" => data::get_quality(ch).await,
        "trigger_lakehouse_build" => pipelines::trigger_build(&state.dagster).await,
        "get_build_status" => pipelines::get_build_status(&state.dagster).await,
        "describe_mart" => data::describe_mart(ch, args).await,
        "create_chart" => dashboards::create_chart(ch, args, None).await,
        "update_chart" => dashboards::update_chart(ch, args).await,
        "create_board" => dashboards::create_board(ch, args).await,
        "list_boards" => dashboards::list_boards(ch).await,
        "suggest_dashboard" => dashboards::suggest_dashboard(ch).await,
        "list_charts" => dashboards::list_charts(ch).await,
        "delete_chart" => dashboards::delete_chart(ch, args).await,
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
    /// "tool tak dikenal" branch), and an unregistered name gets exactly
    /// that refusal shape. See `tests/fixtures/tool_schemas.json` and the
    /// characterization test in `super::super::tests` for the pre-refactor
    /// baseline this must keep matching.
    #[tokio::test]
    async fn run_tool_dispatches_every_registered_tool_and_refuses_unknown() {
        let state = dispatch_test_state();
        for spec in registry::TOOLS {
            let result = run_tool(&state, spec.name, &Map::new()).await;
            if let Some(err) = result.get("error").and_then(Value::as_str) {
                assert!(
                    !err.starts_with("tool tak dikenal"),
                    "{} unexpectedly hit the unknown-tool dispatch arm: {err}",
                    spec.name
                );
            }
        }
        let unknown = run_tool(&state, "not_a_real_tool", &Map::new()).await;
        assert_eq!(
            unknown,
            json!({ "error": "tool tak dikenal: not_a_real_tool" })
        );
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
