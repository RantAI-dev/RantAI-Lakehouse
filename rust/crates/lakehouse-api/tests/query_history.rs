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

    /// What a `page` that is not a whole number from 1 is told.
    const PAGE_REFUSAL: &str = "page must be a whole number of 1 or more";

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
    pub(crate) async fn get_history(
        app: &TestApp,
        cookie: &str,
        query: &str,
    ) -> (StatusCode, Value) {
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
                "status must be one of completed, failed, stopped, blocked",
            ),
            ("engine=presto", "engine must be one of clickhouse, trino"),
            (
                "minDurationMs=soon",
                "minDurationMs must be a non-negative whole number of milliseconds",
            ),
            ("from=not-a-time", "from must be an RFC 3339 time"),
            ("page=0", PAGE_REFUSAL),
            ("page=abc", PAGE_REFUSAL),
            ("page=-2", PAGE_REFUSAL),
            ("page=1.5", PAGE_REFUSAL),
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
        let ids: Vec<&str> = body["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["q-c", "q-b", "q-a"], "newest first");

        let (status, body) = get_history(&app, &cookie, "status=failed").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let runs = &body["runs"];
        assert_eq!(runs.as_array().unwrap().len(), 1);
        assert_eq!(body["total"], 1);
        assert_eq!(runs[0]["id"], "q-b");
        assert_eq!(runs[0]["message"], "Arrow table is missing");
        assert_eq!(runs[0]["tablesRead"], json!(["bronze.orders"]));

        let (status, body) = get_history(&app, &cookie, "table=orders").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["runs"].as_array().unwrap().len(),
            2,
            "any element of tablesRead"
        );
        assert_eq!(body["total"], 2);

        let (status, body) = get_history(&app, &cookie, "minDurationMs=1000").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["runs"].as_array().unwrap().len(), 2);
        assert_eq!(body["total"], 2);

        let (_, body) = get_history(&app, &cookie, "status=completed").await;
        let runs = &body["runs"];
        assert_eq!(runs[0]["id"], "q-a");
        assert!(
            runs[0].get("message").is_none(),
            "a completed run has nothing to say, and the key is omitted: {body}"
        );
        assert_eq!(runs[0]["tablesRead"], json!(["marts.retails"]));
    }

    /// The answer is an envelope, not a bare list (`QS-4` QA of 2026-11-05):
    /// `runs` is one page of 50, `total` counts every row the filters keep,
    /// and `page` and `pageSize` say which slice this is.
    #[tokio::test]
    async fn the_answer_is_an_envelope_of_one_page_and_the_true_total() {
        let app = common::spin_up().await;
        let user = create_principal_with_permissions(&app.pool, "query:read").await;
        let cookie = session_cookie_for_user(&app.pool, user).await;
        for i in 0..55 {
            let status = if i % 5 == 0 { "stopped" } else { "completed" };
            insert_row(
                &app.pool,
                &format!("q-{i:02}"),
                user,
                status,
                "clickhouse",
                1,
                &[],
                None,
            )
            .await;
        }

        let (status, body) = get_history(&app, &cookie, "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["page", "pageSize", "runs", "total"], "{body}");
        assert_eq!(body["runs"].as_array().unwrap().len(), 50);
        assert_eq!(body["runs"][0]["id"], "q-54");
        assert_eq!(body["total"], 55);
        assert_eq!(body["page"], 1);
        assert_eq!(body["pageSize"], 50);

        let (_, body) = get_history(&app, &cookie, "page=2").await;
        let ids: Vec<&str> = body["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["q-04", "q-03", "q-02", "q-01", "q-00"]);
        assert_eq!(body["total"], 55);
        assert_eq!(body["page"], 2);

        // The status filter reaches both the rows and the total.
        let (_, body) = get_history(&app, &cookie, "status=stopped").await;
        assert_eq!(body["total"], 11, "{body}");
        let runs = body["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 11);
        assert!(runs.iter().all(|r| r["status"] == "stopped"), "{body}");
        let (_, body) = get_history(&app, &cookie, "status=completed&page=2").await;
        assert_eq!(body["total"], 44, "{body}");
        assert_eq!(body["runs"], json!([]), "44 rows fit on page 1");
    }

    /// A page past the end is an empty `runs` with the true `total`; a page
    /// no table could hold (more digits than a 64-bit number) is the same
    /// empty page, not a 400 or a 500.
    #[tokio::test]
    async fn a_page_past_the_end_is_empty_with_the_true_total() {
        let app = common::spin_up().await;
        let user = create_principal_with_permissions(&app.pool, "query:read").await;
        let cookie = session_cookie_for_user(&app.pool, user).await;
        for i in 0..3 {
            insert_row(
                &app.pool,
                &format!("q-{i}"),
                user,
                "completed",
                "clickhouse",
                1,
                &[],
                None,
            )
            .await;
        }
        for page in ["2", "999", "99999999999999999999999999"] {
            let (status, body) = get_history(&app, &cookie, &format!("page={page}")).await;
            assert_eq!(status, StatusCode::OK, "page={page}: {body}");
            assert_eq!(body["runs"], json!([]), "page={page}");
            assert_eq!(body["total"], 3, "page={page}");
        }
    }

    /// A `before=` an old cached client still sends is ignored, like every
    /// unknown key (the cursor it carried is gone): the first page answers.
    #[tokio::test]
    async fn a_leftover_before_cursor_is_ignored() {
        let app = common::spin_up().await;
        let user = create_principal_with_permissions(&app.pool, "query:read").await;
        let cookie = session_cookie_for_user(&app.pool, user).await;
        insert_row(
            &app.pool,
            "q-a",
            user,
            "completed",
            "clickhouse",
            1,
            &[],
            None,
        )
        .await;

        let (status, body) = get_history(&app, &cookie, "before=2020-01-01T00:00:00Z,q-zzz").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["runs"][0]["id"], "q-a", "{body}");
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
    use wiremock::matchers::{body_string_contains, method, path};
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
    /// A `ClickHouse` statement error as the engine words it: the code, a
    /// catalog name and the version suffix. The response may show the author
    /// this; the History row must not keep any of it.
    const UNKNOWN_TABLE_ERROR: &str = "Code: 60. DB::Exception: Unknown table expression identifier 'icecat_api.x' in scope SELECT * FROM icecat_api.x. (UNKNOWN_TABLE) (version 25.8.1.3 (official build))";
    /// What History keeps for that error: the cause, with no engine text.
    const UNKNOWN_TABLE_SENTENCE: &str =
        "This query reads a table that does not exist or that you cannot read.";
    /// The engine's answer to a query a Stop killed (`QUERY_WAS_CANCELLED`).
    const KILLED_ERROR: &str =
        "Code: 394. DB::Exception: Query was cancelled. (QUERY_WAS_CANCELLED) (version 25.8.1.3)";
    /// The engine's answer to a query that ran past its time limit
    /// (`TIMEOUT_EXCEEDED`).
    const TIMEOUT_ERROR: &str = "Code: 159. DB::Exception: Timeout exceeded: elapsed 6.2 seconds, maximum: 5. (TIMEOUT_EXCEEDED) (version 25.8.1.3)";
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

    /// The caller acts in a deployment whose shared catalog the owner tenant
    /// holds (`CATALOG_TENANT_ID`), with `ClickHouse` and `Trino` at mocks.
    async fn spin_up_with_clickhouse_and_trino(ch_uri: &str, trino_uri: &str) -> TestApp {
        let mut overrides = HashMap::new();
        overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
        overrides.insert("TRINO_URL".to_owned(), trino_uri.to_owned());
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

    /// The engine answers every run with `status` and `body`, as a real
    /// `ClickHouse` words a failed statement.
    async fn mount_prefetch_ok_and_engine_answer(ch: &MockServer, status: u16, body: &str) {
        mount_prefetch_ok(ch).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
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

    /// `QS-4` QA (2026-11-05, owner's decision 1) and owner 2026-11-12,
    /// LH-QS-2-08: the response, History and the history API all carry the
    /// classified sentence with the same reference id, and no engine text and
    /// no catalog name. (The response used to show the engine's diagnosis,
    /// `QS-1-AC5`.)
    #[tokio::test]
    async fn a_failed_statement_answers_and_records_the_same_classified_sentence() {
        let ch = MockServer::start().await;
        mount_prefetch_ok_and_engine_answer(&ch, 404, UNKNOWN_TABLE_ERROR).await;
        let app = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&app.pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&app.router, &cookie, "SELECT * FROM icecat_api.x").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let shown = body["error"].as_str().expect("the failure's sentence");
        for gone in ["Code:", "DB::Exception", "icecat_api", "UNKNOWN_TABLE"] {
            assert!(
                !shown.contains(gone),
                "the response carries no engine text: {gone} in {shown}"
            );
        }
        let (_, reference) = shown
            .rsplit_once("Reference: ")
            .expect("the sentence ends with its reference");
        let expected = format!("{UNKNOWN_TABLE_SENTENCE} Reference: {reference}");
        assert_eq!(
            shown, expected,
            "the response says what the History row keeps"
        );

        let (recorded_status, message, _) =
            the_recorded_run(&app.pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "failed");
        assert_eq!(
            message.as_deref(),
            Some(expected.as_str()),
            "the row keeps the cause as a fixed sentence and the same reference"
        );

        let (status, body) = crate::filters::get_history(&app, &cookie, "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let runs = &body["runs"];
        assert_eq!(runs[0]["message"], expected.as_str(), "{runs}");
        // The run's own `sql` and `tablesRead` name the table: the person
        // wrote that. The message is the engine's side.
        let listed = runs[0]["message"].as_str().expect("a failed row has one");
        for gone in ["Code:", "DB::Exception", "icecat_api", "UNKNOWN_TABLE"] {
            assert!(
                !listed.contains(gone),
                "the history API's message carries no engine text: {gone} in {listed}"
            );
        }
    }

    /// `QS-4` QA, owner's decision 3: a run Stop ended is `stopped`, with the
    /// sentence the caller was told, and the History filter finds it.
    #[tokio::test]
    async fn a_run_ended_with_stop_records_stopped() {
        let ch = MockServer::start().await;
        mount_prefetch_ok_and_engine_answer(&ch, 500, KILLED_ERROR).await;
        let app = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&app.pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&app.router, &cookie, "SELECT 1").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], "Stopped", "{body}");
        let (recorded_status, message, _) =
            the_recorded_run(&app.pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "stopped");
        assert_eq!(message.as_deref(), Some("Stopped"));

        let (status, body) = crate::filters::get_history(&app, &cookie, "status=stopped").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["runs"][0]["status"], "stopped", "{body}");
        assert_eq!(body["total"], 1, "{body}");
        let (status, body) = crate::filters::get_history(&app, &cookie, "status=failed").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["runs"], json!([]), "a stopped run is not a failed one");
        assert_eq!(body["total"], 0, "{body}");
    }

    /// A run the time limit ended stays `failed` with the limit's own
    /// sentence: it is the limit's doing, not the person's.
    #[tokio::test]
    async fn a_run_ended_by_the_time_limit_stays_failed() {
        let ch = MockServer::start().await;
        mount_prefetch_ok_and_engine_answer(&ch, 500, TIMEOUT_ERROR).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, "SELECT 1").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let sentence = body["error"].as_str().expect("the limit's sentence");
        assert_ne!(sentence, "Stopped", "{body}");
        let (recorded_status, message, _) = the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "failed");
        assert_eq!(message.as_deref(), Some(sentence));
        assert!(!sentence.contains("Code:"), "{sentence}");
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

    /// A past version by its id runs on `Trino` now (`DATA-21` D4), so
    /// the run is recorded as `ok` — the test changes because the
    /// behaviour it guarded (the refusal of a past version on `Trino`)
    /// is supported now.
    #[tokio::test]
    async fn a_past_version_clause_on_trino_runs_and_records_ok() {
        let ch = MockServer::start().await;
        mount_prefetch_ok(&ch).await;
        let trino = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "q1",
                "columns": [{"name": "n", "type": "bigint"}],
                "data": [[1]],
            })))
            .mount(&trino)
            .await;

        let TestApp { router, pool } =
            spin_up_with_clickhouse_and_trino(&ch.uri(), &trino.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_json(
            &router,
            &cookie,
            json!({
                "sql": "SELECT * FROM bronze.group_orders FOR VERSION AS OF 1",
                "engine": "trino",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        // The statement `Trino` received carries the version, in `Trino`'s
        // own form.
        assert!(
            trino
                .received_requests()
                .await
                .expect("mock records requests")
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body).contains("FOR VERSION AS OF 1")),
            "the statement Trino received must carry the version"
        );
        let (recorded_status, message, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        // The store's word for a successful run (`completed`); the plan's
        // "records `ok`" is that, not a status of its own.
        assert_eq!(recorded_status, "completed");
        assert_eq!(message.as_deref(), None);
        assert_eq!(
            tables_read,
            vec!["bronze.group_orders".to_owned()],
            "{tables_read:?}"
        );
        assert_eq!(
            the_recorded_engine(&pool, "rina@meridian.example").await,
            "trino",
            "the row carries the engine the request named"
        );
    }

    /// A past version by TIMESTAMP is still refused on `Trino` (`DATA-21`
    /// D2/D4): no engine has a timestamp form, `ClickHouse` included. The
    /// sentence the caller is told is the one they already know.
    #[tokio::test]
    async fn a_past_timestamp_clause_on_trino_is_refused_with_the_fixed_sentence() {
        let ch = MockServer::start().await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_json(
            &router,
            &cookie,
            json!({
                "sql": "SELECT * FROM bronze.group_orders FOR TIMESTAMP AS OF TIMESTAMP '2026-01-01 00:00:00 UTC'",
                "engine": "trino",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"], PAST_VERSION_REFUSAL, "{body}");
        let (recorded_status, message, _) = the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "blocked");
        assert_eq!(message.as_deref(), Some(PAST_VERSION_REFUSAL));
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

    /// QA `LH-QS-2-06` (`QS-9`): a person whose every role is switched off
    /// from downloading gets `canDownload: false` on the run's own answer, so
    /// the console turns Download and "Copy as TSV" off. The download route
    /// refuses on its own either way.
    #[tokio::test]
    async fn a_run_by_a_person_whose_every_role_is_switched_off_carries_can_download_false() {
        let ch = MockServer::start().await;
        mount_prefetch_ok_and_engine_answer(
            &ch,
            200,
            r#"{"meta":[{"name":"n","type":"UInt8"}],"data":[{"n":1}],"rows":1}"#,
        )
        .await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        // Every role the caller holds is switched off: the rule is about the
        // roles a person has, not about one of them (`BI-1` D7).
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT r.name FROM app_user_role ur JOIN role r ON r.id = ur.role_id \
             JOIN app_user u ON u.id = ur.user_id WHERE u.email = 'rina@meridian.example'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(!roles.is_empty(), "the seeded user holds at least one role");
        sqlx::query(
            "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
             VALUES (TRUE, 'UTC', 'monday', $1)",
        )
        .bind(&roles)
        .execute(&pool)
        .await
        .unwrap();
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(&router, &cookie, "SELECT 1 AS n").await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["canDownload"], json!(false), "{body}");
    }

    /// Review round 1, R4 (`QS-9`, D9): the flag fails closed when the
    /// settings cannot be read. Without this test, `map_or(true, ..)` in
    /// place of `is_ok_and` kept every other test green and the console would
    /// offer Download and "Copy as TSV" whenever the switch is unknown.
    #[tokio::test]
    async fn a_run_carries_can_download_false_when_the_settings_cannot_be_read() {
        let ch = MockServer::start().await;
        mount_prefetch_ok_and_engine_answer(
            &ch,
            200,
            r#"{"meta":[{"name":"n","type":"UInt8"}],"data":[{"n":1}],"rows":1}"#,
        )
        .await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
        // The read fails after the session exists, so what the run meets is
        // the unreadable settings switch and not a broken sign-in.
        sqlx::query("ALTER TABLE reporting_settings RENAME TO reporting_settings_gone")
            .execute(&pool)
            .await
            .expect("hide the settings table");

        let (status, body) = post_sql(&router, &cookie, "SELECT 1 AS n").await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["canDownload"], json!(false), "{body}");
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

    /// `QS-4` QA (case QS-4-01, 2026-11-05): a statement that names a silver
    /// table with backticks is a completed run, and the table is listed in
    /// `tablesRead` as `silver.<name>`, not dropped for its quoting.
    #[tokio::test]
    async fn a_completed_run_on_a_backtick_quoted_silver_table_records_the_table() {
        let ch = MockServer::start().await;
        mount_engine(&ch).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        let (status, body) = post_sql(
            &router,
            &cookie,
            "SELECT * FROM silver.`g9_upload_0ed99141_clean`",
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let (recorded_status, _, tables_read) =
            the_recorded_run(&pool, "rina@meridian.example").await;
        assert_eq!(recorded_status, "completed");
        assert_eq!(
            tables_read,
            vec!["silver.g9_upload_0ed99141_clean".to_owned()],
            "{tables_read:?}"
        );
    }

    /// `QS-4` QA, found beside QS-4-01: `DESCRIBE` and `SHOW COLUMNS FROM`
    /// name a table the tenant gate checks, so History lists it too instead
    /// of recording `[]`. `SHOW TABLES` names none and stays empty.
    #[tokio::test]
    async fn a_statement_that_describes_a_table_records_that_table() {
        let ch = MockServer::start().await;
        mount_engine(&ch).await;
        let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
        let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

        for (sql, tables) in [
            ("DESCRIBE bronze.group_orders", vec!["bronze.group_orders"]),
            (
                "SHOW COLUMNS FROM bronze.group_orders",
                vec!["bronze.group_orders"],
            ),
            ("SHOW TABLES", vec![]),
        ] {
            sqlx::query("DELETE FROM query_history")
                .execute(&pool)
                .await
                .expect("start from an empty history");
            let (status, body) = post_sql(&router, &cookie, sql).await;
            assert_eq!(status, StatusCode::OK, "{sql}: {body}");
            let (recorded_status, _, tables_read) =
                the_recorded_run(&pool, "rina@meridian.example").await;
            assert_eq!(recorded_status, "completed", "{sql}");
            assert_eq!(tables_read, tables, "{sql}");
        }
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

/// What the migration does to the rows already stored (`QS-2` and `QS-4` QA
/// of 2026-11-04/05).
mod migration {
    use lakehouse_store::queries::{RecordHistoryInput, record_history};
    use uuid::Uuid;

    use crate::common;

    /// The migration file itself, run again over a migrated database: its
    /// statements are written to be re-run, so the test exercises the real
    /// SQL and not a copy of it.
    const MIGRATION: &str = include_str!("../../../migrations/0127_query_history_stopped.sql");

    async fn seed(
        pool: &sqlx::PgPool,
        owner: Uuid,
        (id, status, message): (&str, &str, Option<&str>),
    ) {
        record_history(
            pool,
            &RecordHistoryInput {
                id,
                sql: "SELECT 1",
                user: &owner.to_string(),
                owner_id: Some(owner),
                status,
                duration_ms: 1,
                scanned_bytes: 0,
                cost_units: 0.0,
                workload_class: "hot-analytics",
                engine: "clickhouse",
                cache_assisted: false,
                tables_read: &[],
                message,
            },
        )
        .await
        .unwrap();
    }

    async fn stored(pool: &sqlx::PgPool, id: &str) -> (String, Option<String>) {
        sqlx::query_as("SELECT status, message FROM query_history WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn stopped_rows_become_stopped_and_stored_engine_text_becomes_a_sentence() {
        let app = common::spin_up().await;
        let owner = Uuid::new_v4();
        let engine_text = "Code: 60. DB::Exception: Unknown table expression identifier 'icecat_api.x' in scope SELECT * FROM icecat_api.x. (UNKNOWN_TABLE) Reference: 0a1b2c3d4e";
        let trino_text = "line 1:15: Table 'icecat.api.x' does not exist Reference: 0a1b2c3d4f";
        for row in [
            ("old-stop", "failed", Some("Stopped")),
            ("old-engine", "failed", Some(engine_text)),
            (
                "old-engine-no-ref",
                "failed",
                Some("Code: 47. DB::Exception: x"),
            ),
            ("old-trino", "failed", Some(trino_text)),
            (
                "old-near-stop",
                "failed",
                Some("Stopped by the administrator"),
            ),
            (
                "old-time",
                "failed",
                Some("This query ran longer than 5 seconds."),
            ),
            ("old-refused", "blocked", Some("Stopped")),
            ("old-cancelled", "cancelled", Some("Stopped")),
            ("old-done", "completed", None),
        ] {
            seed(&app.pool, owner, row).await;
        }

        sqlx::raw_sql(MIGRATION).execute(&app.pool).await.unwrap();

        assert_eq!(
            stored(&app.pool, "old-stop").await,
            ("stopped".to_owned(), Some("Stopped".to_owned()))
        );
        assert_eq!(
            stored(&app.pool, "old-engine").await,
            (
                "failed".to_owned(),
                Some("The engine could not run this statement. Reference: 0a1b2c3d4e".to_owned())
            ),
            "the engine's text goes, the reference to the log stays"
        );
        assert_eq!(
            stored(&app.pool, "old-engine-no-ref").await,
            (
                "failed".to_owned(),
                Some("The engine could not run this statement.".to_owned())
            )
        );
        for (id, status, message) in [
            ("old-trino", "failed", Some(trino_text)),
            (
                "old-near-stop",
                "failed",
                Some("Stopped by the administrator"),
            ),
            (
                "old-time",
                "failed",
                Some("This query ran longer than 5 seconds."),
            ),
            ("old-refused", "blocked", Some("Stopped")),
            ("old-cancelled", "cancelled", Some("Stopped")),
            ("old-done", "completed", None),
        ] {
            assert_eq!(
                stored(&app.pool, id).await,
                (status.to_owned(), message.map(str::to_owned)),
                "{id} is not the migration's to change"
            );
        }
        // The check accepts the new status and still refuses an unknown one.
        seed(&app.pool, owner, ("new-stop", "stopped", Some("Stopped"))).await;
        let refused =
            sqlx::query("UPDATE query_history SET status = 'paused' WHERE id = 'new-stop'")
                .execute(&app.pool)
                .await;
        assert!(
            refused.is_err(),
            "the status check still refuses a stranger"
        );
    }
}
