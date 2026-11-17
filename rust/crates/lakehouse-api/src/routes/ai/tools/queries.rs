//! Saved-query tools (T1.4 of the copilot-operations-handover plan):
//! `save_query`, `list_saved_queries`, `run_saved_query`.
//!
//! `run_saved_query` fills the query's `{{name}}` parameters and then calls
//! `routes::query::run_statement`, the run `POST /api/query/run` ends in —
//! read-only guard (`is_read_only`) and query-history recording included —
//! rather than duplicating any part of it. This is why no visibility bump of
//! `is_read_only` was needed: calling the whole function already applies the
//! guard internally, which is a stronger form of reuse than exposing the
//! guard alone and re-calling `ClickHouse` a second way here.

use lakehouse_auth::Principal;
use lakehouse_store::PgPool;
use lakehouse_store::queries;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;

fn pool(state: &AppState) -> Result<&PgPool, Value> {
    state.pg.as_deref().ok_or_else(|| {
        json!({
            "error": "saved query store tidak tersedia: Postgres belum dikonfigurasi",
        })
    })
}

pub(super) async fn save_query(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let title = arg_str(args, "title");
    let sql = arg_str(args, "sql");
    if title.is_empty() || sql.is_empty() {
        return json!({ "error": "title and sql are required" });
    }
    // `query-parameters.md` decision 7: this tool stores no definitions, so SQL that holds a
    // `{{name}}` placeholder would be saved with nothing to say what the
    // placeholder is. It gets the refusal the save route gives the same SQL.
    if let Err(refusal) = crate::routes::query::checked_saved_definitions(&sql, None) {
        return json!({ "error": refusal.to_string() });
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
    // The copilot saves under the name it was given; the caller is the
    // author, so the query lands in the caller's personal space (`QS-3`).
    // A call with no principal has no caller: no author, no tenant, and
    // the shared space is the only place such a row can live.
    //
    // `SEC-20`: the tenant is the caller's own — the chat sends no
    // `X-Tenant` header, so it is the caller's first one, the same
    // answer every caller without headers gets. A call that belongs to
    // no tenant writes a row that belongs to no tenant.
    let tenant_id = principal
        .and_then(|principal| {
            crate::tenant_scope::resolve(principal, &axum::http::HeaderMap::new()).ok()
        })
        .flatten();
    let viewer = principal.map_or_else(queries::SavedQueryViewer::default, |principal| {
        crate::routes::query::saved_query_viewer(principal, tenant_id)
    });
    match queries::create_saved_query(pool, &viewer, &title, &sql, &owner, &tags, None).await {
        Ok(saved) => json!({ "ok": true, "query": saved }),
        Err(err) => upstream_error::store(&upstream_error::STORE, &err).to_json(),
    }
}

pub(super) async fn list_saved_queries(state: &AppState, principal: Option<&Principal>) -> Value {
    let pool = match pool(state) {
        Ok(p) => p,
        Err(err) => return err,
    };
    // `SEC-20`: the caller's tenant's saved queries, and the rows that
    // belong to no tenant when that tenant owns the shared catalog. A
    // scope that cannot be resolved is no rows at all — the list fails
    // closed.
    let scope = match principal {
        Some(principal) => match crate::routes::query::saved_query_scope(
            state,
            principal,
            &axum::http::HeaderMap::new(),
        )
        .await
        {
            Ok(scope) => scope,
            Err(err) => {
                tracing::warn!(?err, "chat: the saved query scope could not be resolved");
                queries::SavedQueryScope::None
            }
        },
        None => queries::SavedQueryScope::None,
    };
    match queries::list_saved(pool, scope).await {
        Ok(saved) => json!({ "queries": saved }),
        Err(err) => upstream_error::store(&upstream_error::STORE, &err).to_json(),
    }
}

/// Runs the saved query the caller named, through the same route the
/// console uses.
///
/// `DATA-29`: the answer is the route's own, so a `NULL` cell reaches the
/// model as `null` and an empty cell as `""` — this tool hands the answer
/// on as it came and does not flatten the two.
pub(super) async fn run_saved_query(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let pool = match pool(state) {
        Ok(p) => p,
        Err(err) => return err,
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
    //
    // The check comes before the lookup: the lookup is scoped to the
    // caller's tenant (`SEC-20`), and a caller with no principal has no
    // tenant to scope by.
    let Some(principal) = principal else {
        return json!({
            "error": "running a saved query needs an authenticated user; this run was \
                       triggered by a schedule, which has no user to run it as",
        });
    };
    // `SEC-20`: the lookup is one of the caller's tenant's own rows, so a
    // saved query of another tenant is not found at all — the chat's own
    // "not found" answer, not a refusal that would confirm it exists.
    let scope = match crate::routes::query::saved_query_scope(
        state,
        principal,
        &axum::http::HeaderMap::new(),
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            tracing::warn!(?err, "chat: the saved query scope could not be resolved");
            queries::SavedQueryScope::None
        }
    };
    let saved = match queries::get_saved(pool, &id, &scope).await {
        Ok(found) => found,
        Err(err) => return upstream_error::store(&upstream_error::STORE, &err).to_json(),
    };
    let Some(saved) = saved else {
        return json!({ "error": "saved query tidak ditemukan" });
    };
    // `query-parameters.md` decision 7: the saved definitions and the model's values fill the
    // placeholders, once, here. A refusal ("Fill in {{name}} to run this
    // query.", a value that does not fit) is the tool's error, so the model
    // asks the person instead of guessing. A saved query with no placeholder
    // and no `parameters` comes back byte for byte.
    let statement = match crate::routes::query::fill_tool_parameters(
        &saved.sql,
        args.get("parameters").cloned(),
        Some(saved.parameters.clone()),
    ) {
        Ok(filled) => filled,
        Err(refusal) => return json!({ "error": refusal.to_string() }),
    };
    // This internal call never goes through axum's auth middleware, so it
    // forwards the caller's own `Principal` itself: the copilot dispatcher
    // (`routes::ai::tools::run_tool`) is handed the SAME principal axum
    // extracted for `POST /api/ai/chat` / `POST /api/ai/tool`.
    // `run_statement` 401s with none, for every engine, so a copilot-run
    // saved query is recorded under the real user in `query_history`,
    // exactly like a console-run query, with the statement that ran.
    //
    // The final statement goes to `run_statement`, not `run`: `run` would
    // scan it a second time and refuse a text value that reads `'{{x}}'`.
    // The chat sends no `X-Tenant` header, so the tenant is the caller's
    // first one — the same answer `tenant_scope::resolve` gives every
    // caller without headers.
    //
    // LH-QS-2-08 (owner 2026-11-12): a failure comes in two copies — the
    // model's copy (the engine's trimmed diagnosis, so the model can
    // correct its SQL) and the browser's copy (a fixed sentence and the same
    // reference). The model's copy goes in `error`, as it always did; the
    // browser's copy goes next to it, in the one key only the server reads.
    match crate::routes::query::run_statement(
        state,
        Some(principal),
        &axum::http::HeaderMap::new(),
        &statement,
        "clickhouse",
    )
    .await
    {
        Ok(ok) => api_result_to_value(Ok(ok)).await,
        Err(failure) => {
            let failed: ApiResult<ApiJson<Value>> = Err(failure.response.into());
            let result = api_result_to_value(failed).await;
            crate::routes::ai::with_browser_error(result, &failure.browser)
        }
    }
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
            save_query(&state, None, &Map::new()).await,
            json!({ "error": "title and sql are required" })
        );
    }

    #[tokio::test]
    async fn run_saved_query_requires_id() {
        let state = state_without_pool();
        assert_eq!(
            run_saved_query(&state, None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
    }

    #[tokio::test]
    async fn every_query_tool_is_a_503_shaped_error_without_a_pool() {
        let state = state_without_pool();
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("q1"));
        let result = list_saved_queries(&state, None).await;
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

        /// The shared catalog belongs to one tenant, and the query path
        /// is scoped to the tenant: this fixture principal is the
        /// catalog's own member, so the shared tables stay reachable.
        const CATALOG_TENANT_ID: &str = "11111111-1111-4111-8111-000000000001";

        fn alice() -> Principal {
            Principal {
                id: PrincipalId::User(Uuid::nil()),
                tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000001")],
                display_name: "alice".to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
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
            env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
            let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

            // The migration seed (`0006_seed_queries.sql`) lands two saved
            // queries; either would do, so take the first rather than
            // hardcoding its id here.
            let saved = queries::list_saved(
                &pool,
                queries::SavedQueryScope::All {
                    viewer: crate::routes::query::saved_query_viewer(&alice(), None),
                },
            )
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

            let history =
                queries::list_history(&pool, Uuid::nil(), &queries::HistoryFilter::default())
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

        /// A saved query in the shared space with the definitions given, in
        /// the catalog tenant the test principal belongs to.
        async fn saved_with(pool: &PgPool, sql: &str, definitions: &Value) -> String {
            let viewer = crate::routes::query::saved_query_viewer(
                &alice(),
                Some(uuid::uuid!("11111111-1111-4111-8111-000000000001")),
            );
            queries::create_saved_query_with_parameters(
                pool,
                &viewer,
                "regional visitors",
                sql,
                definitions,
                "alice",
                &[],
                queries::SavedQueryPlacement::SharedSpace,
            )
            .await
            .expect("the saved query is stored")
            .id
        }

        /// A mock engine that answers every statement with one row and keeps
        /// the bodies it received.
        async fn engine() -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [{"name": "region", "type": "String"}],
                    "data": [{"region": "EU"}],
                    "rows": 1,
                })))
                .mount(&server)
                .await;
            server
        }

        fn state_with(server: &MockServer, pool: &PgPool) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            env.insert("CH_URL".to_owned(), server.uri());
            env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
            AppState::new(Config::from_map(&env).expect("valid config from a map"))
        }

        async fn bodies(server: &MockServer) -> Vec<String> {
            server
                .received_requests()
                .await
                .expect("recorded")
                .iter()
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                .collect()
        }

        fn region_dropdown() -> Value {
            json!([{
                "name": "region", "type": "dropdown", "default": "US",
                "options": [
                    { "value": "EU", "label": "Europe" },
                    { "value": "US", "label": "United States" },
                ],
            }])
        }

        fn run_args(id: &str, parameters: Option<Value>) -> Map<String, Value> {
            let mut args = Map::new();
            args.insert("id".to_owned(), json!(id));
            if let Some(parameters) = parameters {
                args.insert("parameters".to_owned(), parameters);
            }
            args
        }

        /// `query-parameters.md` decision 7: the tool fills the saved query's `{{region}}` with the
        /// model's value, and with the stored default when it gives none.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_saved_query_fills_the_saved_definitions_with_the_models_values(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let id = saved_with(&pool, "SELECT {{region}} AS region", &region_dropdown()).await;
            let server = engine().await;
            let state = state_with(&server, &pool);
            let principal = alice();

            let given = run_saved_query(
                &state,
                Some(&principal),
                &run_args(&id, Some(json!({ "region": "EU" }))),
            )
            .await;
            let defaulted = run_saved_query(&state, Some(&principal), &run_args(&id, None)).await;

            assert!(given.get("error").is_none(), "{given}");
            assert!(defaulted.get("error").is_none(), "{defaulted}");
            let sent = bodies(&server).await;
            assert_eq!(sent.len(), 2, "{sent:?}");
            assert!(
                sent[0].contains("SELECT 'EU' AS region"),
                "the model's value: {sent:?}"
            );
            assert!(
                sent[1].contains("SELECT 'US' AS region"),
                "the stored default: {sent:?}"
            );
            Ok(())
        }

        /// The stored definitions are forwarded: a dropdown refuses a value
        /// that is not one of its options. Run as text it would have passed.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_saved_query_forwards_the_stored_definitions(pool: PgPool) -> sqlx::Result<()> {
            let id = saved_with(&pool, "SELECT {{region}} AS region", &region_dropdown()).await;
            let server = engine().await;
            let state = state_with(&server, &pool);

            let result = run_saved_query(
                &state,
                Some(&alice()),
                &run_args(&id, Some(json!({ "region": "Mars" }))),
            )
            .await;

            let error = result["error"].as_str().expect("a refusal");
            assert!(error.contains("region"), "{error}");
            assert!(
                !error.contains("Mars"),
                "a refusal never repeats a value: {error}"
            );
            assert!(bodies(&server).await.is_empty(), "nothing is sent");
            Ok(())
        }

        /// A value that is missing with no default is the tool's error, so the
        /// model asks the person instead of guessing.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_saved_query_returns_a_missing_value_as_its_error(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let id = saved_with(
                &pool,
                "SELECT {{region}} AS region",
                &json!([{ "name": "region", "type": "text" }]),
            )
            .await;
            let server = engine().await;
            let state = state_with(&server, &pool);

            let result = run_saved_query(&state, Some(&alice()), &run_args(&id, None)).await;

            assert_eq!(
                result,
                json!({ "error": "Fill in {{region}} to run this query." })
            );
            assert!(bodies(&server).await.is_empty(), "nothing is sent");

            let bad_shape =
                run_saved_query(&state, Some(&alice()), &run_args(&id, Some(json!(["EU"])))).await;
            assert_eq!(
                bad_shape,
                json!({ "error": "parameters must be an object that maps a parameter name to its value." })
            );
            assert!(bodies(&server).await.is_empty(), "nothing is sent");
            Ok(())
        }

        /// One scan per request: a text value that reads `{{x}}` is printed in
        /// its literal and the final statement is not scanned again. History
        /// holds the statement that ran.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_saved_query_does_not_scan_the_printed_value_again(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let id = saved_with(
                &pool,
                "SELECT {{region}} AS region",
                &json!([{ "name": "region", "type": "text" }]),
            )
            .await;
            let server = engine().await;
            let state = state_with(&server, &pool);

            let result = run_saved_query(
                &state,
                Some(&alice()),
                &run_args(&id, Some(json!({ "region": "{{x}}" }))),
            )
            .await;

            assert!(result.get("error").is_none(), "{result}");
            let sent = bodies(&server).await;
            assert_eq!(sent.len(), 1, "{sent:?}");
            assert!(sent[0].contains("SELECT '{{x}}' AS region"), "{sent:?}");
            let history =
                queries::list_history(&pool, Uuid::nil(), &queries::HistoryFilter::default())
                    .await
                    .expect("query_history is readable");
            assert_eq!(history[0].sql, "SELECT '{{x}}' AS region");
            Ok(())
        }

        /// `query-parameters.md` decision 7: the `save_query` tool is held to the rule the save route
        /// applies. SQL with a placeholder and no definition is refused with
        /// the route's sentence and stores nothing; plain SQL still saves.
        #[sqlx::test(migrations = "../../migrations")]
        async fn save_query_refuses_a_placeholder_it_cannot_define(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = engine().await;
            let state = state_with(&server, &pool);
            let principal = alice();
            let mut args = Map::new();
            args.insert("title".to_owned(), json!("by region"));
            args.insert("sql".to_owned(), json!("SELECT {{region}} AS region"));

            let refused = save_query(&state, Some(&principal), &args).await;

            assert_eq!(
                refused,
                json!({ "error": "{{region}} is used in the SQL but has no definition." })
            );
            let stored = queries::list_saved(
                &pool,
                queries::SavedQueryScope::All {
                    viewer: crate::routes::query::saved_query_viewer(&principal, None),
                },
            )
            .await
            .expect("saved queries are readable");
            assert!(
                stored.iter().all(|q| q.title != "by region"),
                "nothing is stored"
            );

            args.insert("sql".to_owned(), json!("SELECT 'a' AS region"));
            let saved = save_query(&state, Some(&principal), &args).await;
            assert_eq!(saved["ok"], json!(true), "{saved}");
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
            env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
            let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

            let saved = queries::list_saved(
                &pool,
                queries::SavedQueryScope::All {
                    viewer: crate::routes::query::saved_query_viewer(&alice(), None),
                },
            )
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
