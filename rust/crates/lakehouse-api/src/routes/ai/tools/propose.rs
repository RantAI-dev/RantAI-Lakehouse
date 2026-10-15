//! `propose_sql`: the model proposes the whole corrected statement for a
//! Query Studio query that failed, and the server checks that it would run
//! for the caller.
//!
//! The tool runs no query and stores nothing. It answers `{ proposed: true,
//! sql, reason, engine }` when the statement passes
//! [`crate::routes::query::check_statement_would_run`], and `{ error }` when
//! it does not, so the model can correct it in the same turn. The console
//! shows an accepted proposal in the editor; the person runs it, not the
//! assistant.

use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::arg_str;
use super::data::clean_sql_error;
use crate::state::AppState;

/// Longest `sql`, in characters: a whole statement the editor can hold.
const MAX_SQL_CHARS: usize = 20_000;
/// Longest `reason`, in characters: one sentence.
const MAX_REASON_CHARS: usize = 300;

/// The refusal for `engine: "trino"`. Plan 16 replaces it with a check, so
/// the wording is the contract the console shows as the disabled button's
/// title.
const TRINO_REFUSAL: &str = "fixing a Trino query is not supported yet";

/// Validates the arguments, checks the statement for the caller, and returns
/// `{proposed: true, sql, reason, engine}` or `{error}` naming the first
/// broken rule. Lengths count characters after trimming.
pub(super) async fn propose_sql(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let sql = arg_str(args, "sql").trim().to_owned();
    if !(1..=MAX_SQL_CHARS).contains(&sql.chars().count()) {
        return json!({ "error": format!("sql must be 1 to {MAX_SQL_CHARS} characters") });
    }
    let reason = arg_str(args, "reason").trim().to_owned();
    if !(1..=MAX_REASON_CHARS).contains(&reason.chars().count()) {
        return json!({ "error": format!("reason must be 1 to {MAX_REASON_CHARS} characters") });
    }
    let engine = arg_str(args, "engine");
    if engine != "clickhouse" && engine != "trino" {
        return json!({ "error": "engine must be clickhouse or trino" });
    }
    if engine == "trino" {
        return json!({ "error": TRINO_REFUSAL });
    }
    // A check runs as the caller: a headless call (a schedule) has no one to
    // run as, so it is refused before any engine is asked, as `run_sql` does.
    let Some(principal) = principal else {
        return json!({
            "error": "proposing SQL needs a signed-in user; this call came from a schedule, \
                       which has no user to run as",
        });
    };
    match crate::routes::query::check_statement_would_run(state, principal, &sql, &engine).await {
        Ok(()) => json!({ "proposed": true, "sql": sql, "reason": reason, "engine": engine }),
        Err(message) => json!({ "error": clean_sql_error(&message) }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::config::Config;
    use std::collections::HashMap;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    fn args(sql: &str, reason: &str, engine: &str) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("sql".to_owned(), json!(sql));
        map.insert("reason".to_owned(), json!(reason));
        map.insert("engine".to_owned(), json!(engine));
        map
    }

    fn error_of(result: &Value) -> &str {
        result["error"]
            .as_str()
            .unwrap_or_else(|| panic!("expected an error, got {result}"))
    }

    #[tokio::test]
    async fn the_statement_must_be_one_to_twenty_thousand_characters() {
        let state = state();
        let reason = "Fixed a column name.";
        assert!(
            error_of(&propose_sql(&state, None, &args("", reason, "clickhouse")).await)
                .starts_with("sql ")
        );
        assert!(
            error_of(&propose_sql(&state, None, &args("   ", reason, "clickhouse")).await)
                .starts_with("sql ")
        );
        let too_long = "x".repeat(20_001);
        assert!(
            error_of(&propose_sql(&state, None, &args(&too_long, reason, "clickhouse")).await)
                .contains("20000")
        );
        // At the edge the bound passes and the check takes over: with no
        // signed-in user it is refused for that reason, not for length.
        let at_edge = "x".repeat(20_000);
        let refused = propose_sql(&state, None, &args(&at_edge, reason, "clickhouse")).await;
        assert!(!error_of(&refused).contains("characters"), "{refused}");
    }

    #[tokio::test]
    async fn the_reason_must_be_one_to_three_hundred_characters() {
        let state = state();
        let sql = "SELECT 1";
        assert!(
            error_of(&propose_sql(&state, None, &args(sql, "", "clickhouse")).await)
                .starts_with("reason ")
        );
        assert!(
            error_of(&propose_sql(&state, None, &args(sql, &"r".repeat(301), "clickhouse")).await)
                .contains("300")
        );
        let refused = propose_sql(&state, None, &args(sql, &"r".repeat(300), "clickhouse")).await;
        assert!(!error_of(&refused).contains("characters"), "{refused}");
    }

    #[tokio::test]
    async fn an_unknown_engine_is_refused_before_any_check() {
        let state = state();
        let result = propose_sql(&state, None, &args("SELECT 1", "why", "mysql")).await;
        assert_eq!(
            result,
            json!({ "error": "engine must be clickhouse or trino" })
        );
    }

    #[tokio::test]
    async fn trino_is_refused_with_the_sentence_the_console_shows() {
        let state = state();
        let result = propose_sql(&state, None, &args("SELECT 1", "why", "trino")).await;
        assert_eq!(
            result,
            json!({ "error": "fixing a Trino query is not supported yet" })
        );
    }

    #[tokio::test]
    async fn a_headless_call_with_no_principal_is_refused() {
        let state = state();
        let result = propose_sql(&state, None, &args("SELECT 1", "why", "clickhouse")).await;
        assert!(error_of(&result).contains("signed-in user"), "{result}");
    }
}
