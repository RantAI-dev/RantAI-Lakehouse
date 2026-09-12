//! Saved-query tools (T1.4 of the copilot-operations-handover plan):
//! `save_query`, `list_saved_queries`, `run_saved_query`.
//!
//! `run_saved_query` calls `routes::query::run` directly — the exact
//! `POST /api/query/run` handler, read-only guard (`is_read_only`,
//! `routes/query.rs:61-133`) and query-history recording included — rather
//! than duplicating any part of it. This is why no visibility bump of
//! `is_read_only` was needed: calling the whole route function already
//! applies the guard internally, which is a stronger form of reuse than
//! exposing the guard alone and re-calling `ClickHouse` a second way here.

use axum::body::Bytes;
use axum::extract::State;
use lakehouse_store::PgPool;
use lakehouse_store::queries;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::state::AppState;

fn pool(state: &AppState) -> Result<&PgPool, Value> {
    state.pg.as_deref().ok_or_else(|| {
        json!({
            "error": "saved query store tidak tersedia: Postgres belum dikonfigurasi",
        })
    })
}

pub(super) async fn save_query(state: &AppState, args: &Map<String, Value>) -> Value {
    let title = arg_str(args, "title");
    let sql = arg_str(args, "sql");
    if title.is_empty() || sql.is_empty() {
        return json!({ "error": "title dan sql wajib diisi" });
    }
    let owner = {
        let o = arg_str(args, "owner");
        if o.is_empty() {
            "AI Copilot".to_owned()
        } else {
            o
        }
    };
    let tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let pool = match pool(state) {
        Ok(p) => p,
        Err(err) => return err,
    };
    match queries::create_saved_query(pool, &title, &sql, &owner, &tags).await {
        Ok(saved) => json!({ "ok": true, "query": saved }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn list_saved_queries(state: &AppState) -> Value {
    let pool = match pool(state) {
        Ok(p) => p,
        Err(err) => return err,
    };
    match queries::list_saved(pool).await {
        Ok(saved) => json!({ "queries": saved }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn run_saved_query(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    let pool = match pool(state) {
        Ok(p) => p,
        Err(err) => return err,
    };
    let saved = match queries::list_saved(pool).await {
        Ok(all) => all.into_iter().find(|q| q.id == id),
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let Some(saved) = saved else {
        return json!({ "error": "saved query tidak ditemukan" });
    };
    let body = Bytes::from(json!({ "sql": saved.sql }).to_string());
    api_result_to_value(crate::routes::query::run(State(state.clone()), body).await).await
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

    #[tokio::test]
    async fn save_query_requires_title_and_sql() {
        let state = state_without_pool();
        assert_eq!(
            save_query(&state, &Map::new()).await,
            json!({ "error": "title dan sql wajib diisi" })
        );
    }

    #[tokio::test]
    async fn run_saved_query_requires_id() {
        let state = state_without_pool();
        assert_eq!(
            run_saved_query(&state, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
    }

    #[tokio::test]
    async fn every_query_tool_is_a_503_shaped_error_without_a_pool() {
        let state = state_without_pool();
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("q1"));
        let result = list_saved_queries(&state).await;
        assert!(result.get("error").is_some());
        let result = run_saved_query(&state, &args).await;
        assert!(result.get("error").is_some());
    }
}
