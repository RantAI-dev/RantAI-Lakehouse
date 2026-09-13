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

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
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

pub(super) async fn run_saved_query(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
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
    // `None` here is not just the interactive copilot's own missing
    // session — `run_headless_loop` (`routes::agents`) forwards the SAME
    // principal `run_employee`'s auth guard resolved, and that is `None`
    // precisely for `RunAuth::Token`: a schedule or service token, which
    // has no interactive user to run as (C2-F3). Returning a plain
    // forwarded 401 from `query::run` in that case reads as a broken
    // feature; naming the real reason here keeps the refusal fail-closed
    // (the query still never runs) while being honest about why, per
    // AGENTS.md principle 2.
    let Some(principal) = principal else {
        return json!({
            "error": "running a saved query needs an authenticated user; this run was \
                       triggered by a schedule, which has no user to run it as",
        });
    };
    let body = Bytes::from(json!({ "sql": saved.sql }).to_string());
    // This internal call never goes through axum's auth middleware, so it
    // must forward the caller's own `Principal` itself rather than relying
    // on `Extension` extraction: the copilot dispatcher
    // (`routes::ai::tools::run_tool`) is handed the SAME principal axum
    // extracted for `POST /api/ai/chat` / `POST /api/ai/tool`, and this is
    // where it is re-wrapped for `query::run`. That principal is required
    // now — `query::run` 401s with none, for every engine — so a
    // copilot-run saved query is recorded under the real user in
    // `query_history`, exactly like a console-run query.
    let extension = Some(Extension(principal.clone()));
    api_result_to_value(crate::routes::query::run(State(state.clone()), extension, body).await)
        .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    // Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
    // testcontainer bootstrap runs for this `--lib` test binary, the same
    // reason `connector_deprovision`'s test module (and `main.rs`) do this
    // — see that module's comment.
    use lakehouse_test_support as _;

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
            run_saved_query(&state, None, &Map::new()).await,
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
        let result = run_saved_query(&state, None, &args).await;
        assert!(result.get("error").is_some());
    }

    // ── C2-f regression coverage: the copilot's own principal must reach
    // `query::run`, not `None` ──────────────────────────────────────────
    //
    // This needs a real Postgres (a saved query row to run, and
    // `query_history` to inspect afterwards) plus a `ClickHouse` stand-in,
    // so it uses the SAME two harnesses the rest of this crate already
    // relies on: `lakehouse-test-support`'s `#[sqlx::test]` container (see
    // `lakehouse-store/tests/queries.rs`) for Postgres, and `wiremock` for
    // `ClickHouse` (see `routes::query`'s `trino_engine` tests for the
    // equivalent Trino case). `run_saved_query` never asks for
    // `engine: "trino"`, so `ClickHouse`, not Trino, is what needs mocking
    // here.
    mod principal_forwarding {
        use lakehouse_auth::{PermissionSet, PrincipalId};
        use uuid::Uuid;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;

        /// Builds a `DATABASE_URL` string dialing the SAME per-test
        /// Postgres database `#[sqlx::test]` handed us, the way
        /// `connector_deprovision`'s `target_for` derives a `PgTarget`
        /// from `pool.connect_options()` — reused here as a URL string
        /// because `AppState::new` takes `Config::database_url`, not a
        /// pre-built pool.
        fn database_url_for(pool: &PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        fn alice() -> Principal {
            Principal {
                id: PrincipalId::User(Uuid::nil()),
                tenant_ids: Vec::new(),
                display_name: "alice".to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
            }
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn run_saved_query_forwards_the_principal_to_query_run(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [{"name": "region", "type": "String"}],
                    "data": [{"region": "west"}],
                    "rows": 1,
                })))
                .mount(&server)
                .await;

            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
            env.insert("CH_URL".to_owned(), server.uri());
            let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

            // The migration seed (`0006_seed_queries.sql`) lands two saved
            // queries; either would do, so take the first rather than
            // hardcoding its id here.
            let saved = queries::list_saved(&pool)
                .await
                .expect("the migration seed provides saved queries");
            let first = saved
                .first()
                .expect("the migration seed provides at least one saved query");
            let mut args = Map::new();
            args.insert("id".to_owned(), json!(first.id));

            let principal = alice();
            let result = run_saved_query(&state, Some(&principal), &args).await;
            assert!(
                result.get("error").is_none(),
                "expected a successful run, got {result}"
            );

            let history = queries::list_history(&pool)
                .await
                .expect("query_history is readable");
            let recorded = history
                .first()
                .expect("run_saved_query must have recorded one history row");
            assert_eq!(
                recorded.user,
                Uuid::nil().to_string(),
                "query_history.user_name must be the calling principal's uuid, not a \
                 placeholder or a 401 short-circuit"
            );
            Ok(())
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn run_saved_query_without_a_principal_refuses_with_a_named_reason(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            // No `ClickHouse` mock is mounted, and `query::run` is never
            // called: this is the fail-closed "no principal" case
            // `run_saved_query` now short-circuits itself, before it would
            // otherwise forward a bare 401 shape from `query::run` — see
            // C2-F2. A schedule- or service-triggered digital-employee run
            // (`routes::agents::run_headless_loop` with no user behind it)
            // hits exactly this path.
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
            let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

            let saved = queries::list_saved(&pool)
                .await
                .expect("the migration seed provides saved queries");
            let first = saved
                .first()
                .expect("the migration seed provides at least one saved query");
            let mut args = Map::new();
            args.insert("id".to_owned(), json!(first.id));

            let result = run_saved_query(&state, None, &args).await;
            assert_eq!(
                result,
                json!({
                    "error": "running a saved query needs an authenticated user; this run \
                               was triggered by a schedule, which has no user to run it as",
                }),
                "expected the honest no-principal refusal, got {result}"
            );
            Ok(())
        }
    }
}
