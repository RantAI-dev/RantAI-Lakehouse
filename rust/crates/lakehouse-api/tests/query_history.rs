//! The query history's HTTP surface, both halves of it (`QS-4`).
//!
//! The filters half: the history endpoint's query string is the page's
//! only door to the rows (decisions 2, 5 and 7), so the door's own
//! behaviour is pinned here: a value the route cannot parse is a 400
//! whose body is the route's fixed sentence — never a serde rendering —
//! and a value it can parse reaches the store's filters, with the row
//! carrying its `tablesRead` and, for a failed or refused run, its
//! `message`. The SQL semantics behind the filters (the 14-day clamp, the
//! cursor, the literal `%`, owner scoping) are `tests/queries.rs`'s; the
//! `filters` module owns what the HTTP surface does with the caller's
//! text.
//!
//! The recording half: `run` used to write a history row only for a
//! completed run, so a refused or failed run existed nowhere but the
//! caller's screen. Now the same best-effort write that records a
//! completed run also records a `blocked` refusal (the policy rewrite,
//! the `SEC-20` table gate, and their sibling guards) and a `failed`
//! engine call, each row carrying the exact text the response carries —
//! so the history page can show why a run looks the way it does, without
//! the row and the screen ever disagreeing. The fake engine failure's
//! body carries a marker string that must appear in neither the response
//! nor the row: an upstream error is classified before it is surfaced,
//! and the row stores what the response carried, nothing more.
//!
//! One binary on purpose. CI links one binary per file in this directory,
//! and this branch's two fresh ones pushed the check's
//! `cargo test --workspace --locked` over the GitHub runner's disk (PR
//! #254's check died on `No space left on device` twice while main's own
//! job passes); the two concerns live on as two modules, their tests
//! unchanged.
//!
//! The `filters` tests drive the real router over a fresh database per
//! test, with a principal that holds only `query:read` — the history
//! endpoint reads no engine, so no `ClickHouse` mock is wired. The
//! `recording` tests drive the router over the seeded tenants, with a
//! `wiremock` `ClickHouse` wired through `CH_URL` — the arrangement of
//! `tests/query_tenant_scope.rs` — and read the recorded rows back
//! through the pool.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

/// What the route does with the caller's text before the store sees it.
mod filters {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use lakehouse_store::queries::{RecordHistoryInput, record_history};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::common::{
        self, TestApp, create_principal_with_permissions, session_cookie_for_user,
    };

    /// What a query string the route cannot read is told — the route's own
    /// sentence, byte for byte.
    const QUERY_UNREADABLE: &str = "The query string could not be read.";

    /// One recorded run for the pass-through test, written the way `run`
    /// writes them — the caller is the owner, the status is whatever the run
    /// reached.
    async fn insert_row(
        pool: &sqlx::PgPool,
        id: &str,
        owner: Uuid,
        status: &str,
        engine: &str,
        duration_ms: i64,
        tables: &[&str],
        message: Option<&str>,
    ) {
        let tables: Vec<String> = tables.iter().map(|t| (*t).to_owned()).collect();
        record_history(
            pool,
            &RecordHistoryInput {
                id,
                sql: "SELECT 1",
                user: &owner.to_string(),
                owner_id: Some(owner),
                status,
                duration_ms,
                scanned_bytes: 1,
                cost_units: 0.1,
                workload_class: "hot-analytics",
                engine,
                cache_assisted: false,
                tables_read: &tables,
                message,
            },
        )
        .await
        .unwrap();
    }

    /// `GET /api/query/history?query` as the principal's cookie carries it.
    async fn get_history(app: &TestApp, cookie: &str, query: &str) -> (StatusCode, Value) {
        let uri = if query.is_empty() {
            "/api/query/history".to_owned()
        } else {
            format!("/api/query/history?{query}")
        };
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        // A refusal the route itself builds is JSON; a refusal the extractor
        // builds is not (it is plain text), so the body is kept as text when
        // it is not JSON — otherwise the test could not say what the caller
        // was actually sent.
        let body: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }

    /// A value outside the parser's sets is a 400 whose `error` is the fixed
    /// sentence naming the field — the response never carries a serde or
    /// urlencoded rendering of what went wrong.
    #[tokio::test]
    async fn a_bad_filter_value_is_a_400_with_the_fixed_message() {
        let app = common::spin_up().await;
        let user = create_principal_with_permissions(&app.pool, "query:read").await;
        let cookie = session_cookie_for_user(&app.pool, user).await;

        for (query, fixed) in [
            (
                "status=yesterday",
                "status must be one of completed, failed, cancelled, blocked",
            ),
            ("engine=presto", "engine must be one of clickhouse, trino"),
            (
                "minDurationMs=soon",
                "minDurationMs must be a non-negative whole number of milliseconds",
            ),
            ("from=not-a-time", "from must be an RFC 3339 time"),
            (
                "before=2026-10-20T00:00:00Z",
                "before must be <at>,<id>: the at and the id of the last row you have seen",
            ),
        ] {
            let (status, body) = get_history(&app, &cookie, query).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
            assert_eq!(body["error"], fixed, "{query}");
            let rendered = body.to_string();
            assert!(
                !rendered.contains("serde") && !rendered.contains("unknown variant"),
                "{query}: the response carries a library error: {rendered}"
            );
        }
    }

    /// A query string the extractor itself cannot read — a parameter sent
    /// twice — is refused with this route's own fixed sentence, never with
    /// serde's rendering of what went wrong. The repeated parameter is
    /// refused, not merged: the second value is not silently dropped.
    #[tokio::test]
    async fn a_repeated_query_parameter_is_a_400_with_the_fixed_sentence() {
        let app = common::spin_up().await;
        let user = create_principal_with_permissions(&app.pool, "query:read").await;
        let cookie = session_cookie_for_user(&app.pool, user).await;

        let (status, body) = get_history(&app, &cookie, "status=completed&status=failed").await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], QUERY_UNREADABLE, "{body}");
        let rendered = body.to_string();
        assert!(
            !rendered.contains("duplicate field") && !rendered.contains("serde"),
            "the response carries a library error: {rendered}"
        );
    }

    /// The parsed filters reach the rows: the endpoint answers the page's
    /// one-axis questions, and each row carries `tablesRead` — with `message`
    /// only where there is something to say.
    #[tokio::test]
    async fn the_filters_reach_the_rows_and_the_row_carries_its_tables_and_message() {
        let app = common::spin_up().await;
        let user = create_principal_with_permissions(&app.pool, "query:read").await;
        let cookie = session_cookie_for_user(&app.pool, user).await;

        insert_row(
            &app.pool,
            "q-a",
            user,
            "completed",
            "clickhouse",
            100,
            &["marts.retails"],
            None,
        )
        .await;
        insert_row(
            &app.pool,
            "q-b",
            user,
            "failed",
            "clickhouse",
            5_000,
            &["bronze.orders"],
            Some("Arrow table is missing"),
        )
        .await;
        insert_row(
            &app.pool,
            "q-c",
            user,
            "blocked",
            "trino",
            2_500,
            &["marts.retails", "bronze.orders"],
            Some("Table not found or not available to your organisation: bronze.orders"),
        )
        .await;

        let (status, body) = get_history(&app, &cookie, "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let ids: Vec<&str> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["q-c", "q-b", "q-a"], "newest first");

        let (status, body) = get_history(&app, &cookie, "status=failed").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["id"], "q-b");
        assert_eq!(body[0]["message"], "Arrow table is missing");
        assert_eq!(body[0]["tablesRead"], json!(["bronze.orders"]));

        let (status, body) = get_history(&app, &cookie, "table=orders").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body.as_array().unwrap().len(),
            2,
            "any element of tablesRead"
        );

        let (status, body) = get_history(&app, &cookie, "minDurationMs=1000").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body.as_array().unwrap().len(), 2);

        let (_, body) = get_history(&app, &cookie, "status=completed").await;
        assert_eq!(body[0]["id"], "q-a");
        assert!(
            body[0].get("message").is_none(),
            "a completed run has nothing to say, and the key is omitted: {body}"
        );
        assert_eq!(body[0]["tablesRead"], json!(["marts.retails"]));
    }
}

/// What a run leaves behind: every answered run is recorded (decision 3).
mod recording {
    use std::collections::HashMap;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use lakehouse_store::uploads::claim_table;
    use serde_json::{Value, json};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

    /// The tenant that owns the shared catalog and its upload table below.
    const OWNER_TENANT: &str = "11111111-1111-4111-8111-000000000001";
    /// The upload table the owner tenant claimed.
    const OWNED_TABLE: &str = "group_orders";
    /// The statement that reads it.
    const OWNED_STATEMENT: &str = "SELECT * FROM bronze.group_orders";
    /// The table refusal, byte for byte: the table as the gate canonicalises it.
    const TABLE_REFUSAL: &str =
        "Table not found or not available to your organisation: bronze.group_orders";
    /// The policy refusal's fixed sentence, as `policy_engine::refusal_message`
    /// words the rewrite's sensitive-system-table rule.
    const POLICY_REFUSAL: &str =
        "Tabel sistem ini tidak bisa dibaca dari sini; gunakan halaman Audit.";
    /// Text only the fake engine's failure body carries. It must appear in
    /// neither the response nor the history row.
    const ENGINE_MARKER: &str = "TANGERINE-PANEL-NINE";
    /// The read-only guard's fixed sentence.
    const READ_ONLY_REFUSAL: &str =
        "Only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) are allowed in Query Studio.";
    /// The past-version refusal's fixed sentence (`DATA-16` F3).
    const PAST_VERSION_REFUSAL: &str =
        "Querying a past version of a table is available on the ClickHouse engine only.";
    /// The settings rule's fixed sentence (`SEC-21`).
    const SETTINGS_REFUSAL: &str =
        "Query Studio accepts only iceberg_snapshot_id in SETTINGS; the server sets the others.";
    /// The Trino keyword check's fixed sentence.
    const TRINO_KEYWORD_REFUSAL: &str = "This statement is not permitted against Trino (write, session, or procedure keyword \
         detected).";

    /// The caller acts in a deployment whose shared catalog the owner tenant
    /// holds (`CATALOG_TENANT_ID`), with `ClickHouse` at the mock.
    async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
        let mut overrides = HashMap::new();
        overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
        overrides.insert("CATALOG_TENANT_ID".to_owned(), OWNER_TENANT.to_owned());
        spin_up_with_env(&overrides).await
    }

    /// Answers the rewrite's own `system.*` lookups with an empty table, so
    /// that only the run's own statement is left to reach the engine mock.
    async fn mount_prefetch_ok(ch: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("system."))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "type", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [],
                "rows": 0,
            })))
            .mount(ch)
            .await;
    }

    /// The engine answers every run with a failure whose body carries only the
    /// marker — a text no classification is allowed to pass through.
    async fn mount_prefetch_ok_and_engine_failure(ch: &MockServer) {
        mount_prefetch_ok(ch).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(ENGINE_MARKER))
            .mount(ch)
            .await;
    }

    /// Answers the column lookup the rewrite prefetches with an empty table,
    /// and any other statement with one three-column row — enough for the run
    /// to complete.
    async fn mount_engine(ch: &MockServer) {
        mount_prefetch_ok(ch).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "database", "type": "String"},
                    {"name": "table", "type": "String"},
                    {"name": "rows", "type": "UInt64"},
                ],
                "data": [{"database": "bronze", "table": "group_orders", "rows": 10}],
                "rows": 1,
            })))
            .mount(ch)
            .await;
    }

    /// How many requests the engine received, prefetches included.
    async fn engine_requests(ch: &MockServer) -> usize {
        ch.received_requests()
            .await
            .expect("mock records requests")
            .len()
    }

    /// The statements the engine received that were not the rewrite's own
    /// `system.` lookups — the caller's statements, in short.
    async fn statements_reaching_the_engine(ch: &MockServer) -> Vec<String> {
        ch.received_requests()
            .await
            .expect("mock records requests")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| !b.contains("system."))
            .collect()
    }

    /// A tenant claims the upload table, the way an upload does.
    async fn claim_table_for(pool: &PgPool, tenant: &str) {
        let claimed = claim_table(
            pool,
            tenant.parse::<Uuid>().expect("a seeded tenant id"),
            OWNED_TABLE,
            "upload-group-orders",
        )
        .await
        .expect("claim the upload table for the tenant");
        assert!(claimed, "the tenant must hold the claim");
    }

    /// A run request with the given JSON body, as the console sends it.
    async fn post_json(router: &axum::Router, cookie: &str, body: Value) -> (StatusCode, Value) {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/query/run")
                    .header("cookie", cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }

    /// A run request carrying a statement.
    async fn post_sql(router: &axum::Router, cookie: &str, sql: &str) -> (StatusCode, Value) {
        post_json(router, cookie, json!({ "sql": sql })).await
    }

    /// How many rows the history holds — each test runs against its own fresh
    /// database, so the count is the test's own.
    async fn history_rows(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM query_history")
            .fetch_one(pool)
            .await
            .expect("read the run history")
    }

    /// The ids the history holds, oldest first.
    async fn recorded_ids(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("SELECT id FROM query_history ORDER BY at, id")
            .fetch_all(pool)
            .await
            .expect("read the recorded run ids")
    }

    /// The shape a recorded run's id carries: `q-<epoch_ms>-<32 lowercase
    /// hex>`.
    ///
    /// The rule is written out again here on purpose: the shape is the
    /// contract the download route and the parity harness rely on, so the
    /// test states it in full instead of asking the route's own helper
    /// whether the route's own helper is right.
    fn is_the_new_id_shape(id: &str) -> bool {
        let Some(rest) = id.strip_prefix("q-") else {
            return false;
        };
        let Some((epoch, suffix)) = rest.rsplit_once('-') else {
            return false;
        };
        !epoch.is_empty()
            && epoch.bytes().all(|b| b.is_ascii_digit())
            && suffix.len() == 32
            && suffix
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    /// The one row the caller's run recorded: its status, its message, and the
    /// tables its statement named.
    async fn the_recorded_run(pool: &PgPool, email: &str) -> (String, Option<String>, Vec<String>) {
        let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|e| {
                panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}")
            });
        sqlx::query_as("SELECT status, message, tables_read FROM query_history WHERE owner_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .expect("the run's row is recorded")
    }

    /// The engine the caller's run recorded — the request's own `engine`
    /// field, written into the row.
    async fn the_recorded_engine(pool: &PgPool, email: &str) -> String {
        let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|e| {
                panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}")
            });
        sqlx::query_scalar("SELECT engine FROM query_history WHERE owner_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .expect("the run's row is recorded")
    }

    #[tokio::test]
    async fn a_failed_engine_run_records_failed_with_the_responses_message_and_not_the_engines_text()
     {
        let ch = MockServer::start().await;
        mount_prefetch_ok_and_engine_failure(&ch).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, "SELECT 1").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let response_message = body["error"].as_str().expect("the fixed error text");
        assert!(response_message.contains("Reference: "), "{body}");
        assert!(!response_message.contains(ENGINE_MARKER), "{body}");
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(
            recorded_status, "failed",
            "the engine's failure is the run's outcome"
        );
        assert_eq!(
            message.as_deref(),
            Some(response_message),
            "the row says what the caller was told"
        );
        assert!(
            !message
                .expect("a failed row carries a message")
                .contains(ENGINE_MARKER)
        );
        assert!(
            tables_read.is_empty(),
            "SELECT 1 names no table: {tables_read:?}"
        );
    }

    #[tokio::test]
    async fn a_policy_refusal_records_blocked_and_the_statement_never_runs() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, "SELECT query FROM system.query_log").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], POLICY_REFUSAL, "{body}");
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(POLICY_REFUSAL));
        assert_eq!(
            tables_read,
            vec!["system.query_log".to_owned()],
            "{tables_read:?}"
        );
        // The rewrite's own `system.` prefetch may reach the engine; the
        // caller's statement itself must not.
        assert!(
            statements_reaching_the_engine(&ch).await.is_empty(),
            "the statement the caller wrote never reaches the engine"
        );
    }

    #[tokio::test]
    async fn a_table_another_tenant_claims_records_blocked_with_the_fixed_refusal() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        claim_table_for(&pool, OWNER_TENANT).await;
        let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, OWNED_STATEMENT).await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], TABLE_REFUSAL, "{body}");
        assert_eq!(
            engine_requests(&ch).await,
            0,
            "the tenant gate refuses before anything is asked of the engine"
        );
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "sari@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(TABLE_REFUSAL));
        assert_eq!(
            tables_read,
            vec!["bronze.group_orders".to_owned()],
            "{tables_read:?}"
        );
    }

    /// The read-only guard refuses a writing statement before anything is
    /// asked of the engine, and the refusal is recorded: the history says
    /// `blocked` and carries the sentence the caller was told.
    #[tokio::test]
    async fn a_statement_the_read_only_guard_refuses_records_blocked() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(
            &router,
            &cookie,
            "INSERT INTO bronze.group_orders VALUES (1)",
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], READ_ONLY_REFUSAL, "{body}");
        assert_eq!(
            engine_requests(&ch).await,
            0,
            "the guard refuses before the engine is asked"
        );
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(READ_ONLY_REFUSAL));
        assert!(
            tables_read.is_empty(),
            "a refused write is not a query, so it names no table: {tables_read:?}"
        );
    }

    /// A past-version clause cannot run on Trino: the rewrite cannot parse
    /// it, and the refusal is recorded as `blocked` — with the sentence
    /// the caller was told, and without the engine ever being asked.
    #[tokio::test]
    async fn a_past_version_clause_on_trino_records_blocked() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_json(
            &router,
            &cookie,
            json!({
                "sql": "SELECT * FROM bronze.group_orders VERSION AS OF 1",
                "engine": "trino",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], PAST_VERSION_REFUSAL, "{body}");
        assert_eq!(
            engine_requests(&ch).await,
            0,
            "the statement never reaches an engine"
        );
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(PAST_VERSION_REFUSAL));
        assert!(
            tables_read.is_empty(),
            "a statement that does not parse names no table: {tables_read:?}"
        );
        assert_eq!(
            the_recorded_engine(&pool, "rina@meridian.example").await,
            "trino",
            "the row carries the engine the request named"
        );
    }

    /// A Trino statement the keyword check refuses is refused before the
    /// engine is asked, and the refusal is recorded as `blocked` — with
    /// the sentence the caller was told.
    ///
    /// `EXPLAIN ANALYZE` is the first hazard the check names: on Trino it
    /// *executes* the statement it explains, so a read-only statement
    /// behind it is not read-only at all. It still passes the read-only
    /// guard (it starts with an allowed keyword and names none of the
    /// twelve denied ones), so the keyword check is what refuses it.
    #[tokio::test]
    async fn a_trino_statement_with_a_refused_keyword_records_blocked() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_json(
            &router,
            &cookie,
            json!({ "sql": "EXPLAIN ANALYZE SELECT 1", "engine": "trino" }),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], TRINO_KEYWORD_REFUSAL, "{body}");
        assert_eq!(
            engine_requests(&ch).await,
            0,
            "the keyword check refuses before the engine is asked"
        );
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(TRINO_KEYWORD_REFUSAL));
        assert!(
            tables_read.is_empty(),
            "the statement names no table: {tables_read:?}"
        );
        assert_eq!(
            the_recorded_engine(&pool, "rina@meridian.example").await,
            "trino",
            "the row carries the engine the request named"
        );
    }

    /// A statement naming a setting the console does not allow is refused
    /// before it runs, and the refusal is recorded as `blocked`.
    #[tokio::test]
    async fn a_statement_with_a_refused_setting_records_blocked() {
        let ch = MockServer::start().await;
        mount_prefetch_ok(&ch).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(
            &router,
            &cookie,
            "SELECT n FROM serving.t SETTINGS max_result_rows = 0",
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], SETTINGS_REFUSAL, "{body}");
        assert!(
            statements_reaching_the_engine(&ch).await.is_empty(),
            "the statement the caller wrote never reaches the engine"
        );
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(SETTINGS_REFUSAL));
        assert_eq!(tables_read, vec!["serving.t".to_owned()], "{tables_read:?}");
    }

    /// A 5xx from the rewrite is recorded as `failed`, not as `blocked`: the
    /// service failed while deciding, the caller was told so, and the row
    /// says the same thing — never the database's own text.
    #[tokio::test]
    async fn a_5xx_from_the_rewrite_records_failed() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        claim_table_for(&pool, OWNER_TENANT).await;
        // The tenant gate reads the claim table to answer "is this table
        // the caller's?". Hiding that table makes the read fail, which is
        // the one 5xx the rewrite can produce here.
        sqlx::query("ALTER TABLE upload_table_claim RENAME TO upload_table_claim_hidden")
            .execute(&pool)
            .await
            .expect("hide the claim table from the gate");
        let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, OWNED_STATEMENT).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{status}: {body}");
        assert_eq!(
            engine_requests(&ch).await,
            0,
            "the gate refuses before the engine is asked"
        );
        let response_message = body["error"].as_str().expect("the fixed error text");
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "sari@meridian.example").await;
        assert_eq!(recorded_status, "failed");
        assert_eq!(
            message.as_deref(),
            Some(response_message),
            "the row says what the caller was told"
        );
        assert!(
            !response_message.contains("upload_table_claim"),
            "the database's own text never reaches the caller: {response_message}"
        );
        assert_eq!(
            tables_read,
            vec!["bronze.group_orders".to_owned()],
            "{tables_read:?}"
        );
    }

    #[tokio::test]
    async fn a_completed_run_records_the_tables_its_statement_names() {
        let ch = MockServer::start().await;
        mount_engine(&ch).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        claim_table_for(&pool, OWNER_TENANT).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, OWNED_STATEMENT).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "completed");
        assert_eq!(message, None, "a completed run has nothing to say");
        assert_eq!(
            tables_read,
            vec!["bronze.group_orders".to_owned()],
            "{tables_read:?}"
        );
    }

    /// `DATA-29`: a `NULL` cell and an empty cell in the same row are told
    /// apart in the answer — the `NULL` is `null` and the empty cell stays
    /// `""`.
    ///
    /// The engine's own answer decides it: the mock reports one column as
    /// `null` and the other as an empty string, and the route passes both
    /// on as they came. Before this change the `NULL` arrived as `""` and
    /// the two were the same answer.
    #[tokio::test]
    async fn a_null_cell_is_null_and_an_empty_cell_is_an_empty_string() {
        let ch = MockServer::start().await;
        mount_prefetch_ok(&ch).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "maybe", "type": "Nullable(String)"},
                    {"name": "text", "type": "String"},
                ],
                "data": [{"maybe": null, "text": ""}],
                "rows": 1,
            })))
            .mount(&ch)
            .await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        claim_table_for(&pool, OWNER_TENANT).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, OWNED_STATEMENT).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["rows"],
            json!([{"maybe": null, "text": ""}]),
            "a NULL cell must be null and an empty cell must stay an empty string"
        );
    }

    /// Two runs are both recorded, and their ids cannot collide.
    ///
    /// The store inserts the history row with `ON CONFLICT (id) DO NOTHING`,
    /// so two runs whose ids are the same lose one of their rows: a run
    /// that answered is then nowhere in the history. The id carries the
    /// epoch milliseconds *and* a uuid, so two runs in the same
    /// millisecond still get two ids and two rows.
    #[tokio::test]
    async fn two_runs_are_both_recorded_and_their_ids_cannot_collide() {
        let ch = MockServer::start().await;
        mount_engine(&ch).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, "SELECT 1").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = post_sql(&router, &cookie, "SELECT 2").await;
        assert_eq!(status, StatusCode::OK, "{body}");

        assert_eq!(
            history_rows(&pool).await,
            2,
            "both runs are recorded: neither row is dropped"
        );
        let ids = recorded_ids(&pool).await;
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert_ne!(ids[0], ids[1], "two runs never share an id: {ids:?}");
        for id in &ids {
            assert!(is_the_new_id_shape(id), "the id's shape: {id}");
        }
    }

    #[tokio::test]
    async fn a_body_without_sql_records_nothing() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        for body in [json!({}), json!({ "sql": "" })] {
            let (status, response) = post_json(&router, &cookie, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
            assert_eq!(response["error"], "sql is required", "{response}");
        }
        assert_eq!(
            history_rows(&pool).await,
            0,
            "nothing to record: no statement, no run"
        );
    }
}
