//! `GOV-7` slice 1: read access per schema and table, enforced everywhere
//! data is read.
//!
//! A role narrowed to `'listed'` may read only the tables and schemas its
//! `role_read_grant` rows name. Every governed read path refuses a table
//! outside them with `SEC-20`'s own 404 and sentence (`D2`), never
//! "forbidden": `Table not found or not available to your organisation:
//! <schema.table>`. A listing (`SHOW TABLES`) is refused with the gate's 422
//! sentence because no table is named to check (`D5`).
//!
//! The harness is `tests/query_tenant_scope.rs`'s: the real router over a real
//! per-test Postgres, and a `wiremock` `ClickHouse` that records every
//! statement, so a test can say exactly what the engine was and was not asked.
//! Masks keep working on an allowed table. Embedding and the assistant's
//! prompt follow the same rule as the console.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::governance::{self, CreatePolicyInput};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use lakehouse_store::queries::{RecordHistoryInput, record_history};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The tenant that owns the shared catalog; every narrowed caller here is a
/// member of it, so the tenant gate is never the refusal under test.
const CATALOG_TENANT: &str = common::CATALOG_TENANT;
/// A schema the narrowed role may read, and one it may not.
const ALLOWED: &str = "finance.orders";
const HIDDEN: &str = "silver.orders";
/// The refusal of [`HIDDEN`], byte for byte.
const REFUSAL: &str = "Table not found or not available to your organisation: silver.orders";
/// The refusal of a statement that names no resolvable table.
const LISTING_REFUSAL: &str = "This statement is not available to your organisation.";

/// A `ClickHouse` answer with `meta` and `data`.
fn rows(meta: &[&str], data: &[Value]) -> ResponseTemplate {
    let count = data.len();
    let meta: Vec<Value> = meta
        .iter()
        .map(|name| json!({ "name": name, "type": "String" }))
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({
        "meta": meta, "data": data, "rows": count,
    }))
}

/// A recording `ClickHouse` that answers the policy rewrite's own lookups and
/// the statements the tests run, and keeps every statement it was sent.
#[derive(Clone, Default)]
struct FakeCh {
    log: Arc<Mutex<Vec<String>>>,
}

impl FakeCh {
    fn statements(&self) -> Vec<String> {
        self.log.lock().expect("the statement log").clone()
    }

    /// The statements that read `table` — never the rewrite's own
    /// `system.columns`/`system.tables` lookups.
    fn reads_of(&self, table: &str) -> Vec<String> {
        self.statements()
            .into_iter()
            .filter(|sql| sql.contains(&format!("FROM {table}")) && !sql.contains("system."))
            .collect()
    }

    async fn serve(&self) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        server
    }
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log
            .lock()
            .expect("the statement log")
            .push(sql.clone());
        // The columns of whichever table the rewrite asks about. Both test
        // tables have the same shape, which the tests never need to tell
        // apart: a mask is about the column names, not the table.
        if sql.contains("FROM system.columns") {
            return rows(
                &["name", "default_kind", "default_expression"],
                &[
                    json!({"name": "id", "default_kind": "", "default_expression": ""}),
                    json!({"name": "email", "default_kind": "", "default_expression": ""}),
                ],
            );
        }
        // No table is a view: the rewrite's view refusal stays out of the way.
        if sql.contains("FROM system.tables") {
            return rows(&["database", "name", "engine", "create_table_query"], &[]);
        }
        if sql.contains(&format!("FROM {ALLOWED}")) || sql.contains(&format!("FROM {HIDDEN}")) {
            return rows(&["id", "email"], &[json!({"id": "1", "email": "***"})]);
        }
        // The store's own table checks.
        if sql.starts_with("CREATE") || sql.starts_with("ALTER") || sql.starts_with("DROP") {
            return ResponseTemplate::new(200).set_body_string("Ok.\n");
        }
        rows(&[], &[])
    }
}

async fn app(ch: &MockServer) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    spin_up_with_env(&overrides).await
}

/// A person holding `permissions`, in the catalog's tenant, whose every role
/// is narrowed to exactly `grants` (`(schema, table)`; `table` `*` is a whole
/// schema).
async fn narrowed(pool: &PgPool, permissions: &str, grants: &[(&str, &str)]) -> Uuid {
    let user = create_principal_with_permissions(pool, permissions).await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(Uuid::parse_str(CATALOG_TENANT).expect("a seeded tenant id"))
        .execute(pool)
        .await
        .expect("add the membership");
    narrow(pool, user, grants).await;
    user
}

/// Narrow every role `user` holds to exactly `grants`. A `*:*` role cannot be
/// narrowed (`GOV-7` D4), so the tests that narrow never mint one.
async fn narrow(pool: &PgPool, user: Uuid, grants: &[(&str, &str)]) {
    let roles: Vec<Uuid> =
        sqlx::query_scalar("SELECT role_id FROM app_user_role WHERE user_id = $1")
            .bind(user)
            .fetch_all(pool)
            .await
            .expect("the user's roles");
    assert!(!roles.is_empty(), "the user must hold a role to narrow");
    for role in roles {
        sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
            .bind(role)
            .execute(pool)
            .await
            .expect("narrow the role");
        sqlx::query("DELETE FROM role_read_grant WHERE role_id = $1")
            .bind(role)
            .execute(pool)
            .await
            .expect("clear the grants");
        for (schema, table) in grants {
            sqlx::query(
                "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, $2, $3)",
            )
            .bind(role)
            .bind(schema)
            .bind(table)
            .execute(pool)
            .await
            .expect("grant the table");
        }
    }
}

/// [`narrow`] a role by name, for the fixed `Dashboard Viewer` role an embed
/// reads as.
async fn narrow_role_by_name(pool: &PgPool, name: &str, grants: &[(&str, &str)]) {
    let role: Uuid = sqlx::query_scalar("SELECT id FROM role WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("seeded role {name}: {e}"));
    sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
        .bind(role)
        .execute(pool)
        .await
        .expect("narrow the role");
    for (schema, table) in grants {
        sqlx::query(
            "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(role)
        .bind(schema)
        .bind(table)
        .execute(pool)
        .await
        .expect("grant the table");
    }
}

async fn post_sql(
    router: &axum::Router,
    cookie: &str,
    uri: &str,
    sql: &str,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "sql": sql }).to_string()))
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

/// A run the caller owns, so a download's own ownership check passes.
async fn insert_history_row(pool: &PgPool, id: &str, sql: &str, owner: Uuid) {
    record_history(
        pool,
        &RecordHistoryInput {
            id,
            sql,
            user: &owner.to_string(),
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 12,
            scanned_bytes: 34,
            cost_units: 1.0,
            workload_class: "hot-analytics",
            engine: "clickhouse",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .expect("insert a query_history fixture row");
}

async fn download_csv(router: &axum::Router, cookie: &str, id: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/query/run/{id}/download?format=csv"))
                .header("cookie", cookie)
                .body(Body::empty())
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

/// D2/D5: a hidden table is refused with the tenant gate's 404 and sentence,
/// before the engine is asked anything.
#[tokio::test]
async fn query_run_on_a_hidden_table_is_not_found_and_on_an_allowed_one_returns_rows() {
    let ch = FakeCh::default();
    let server = ch.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "query:read", &[("finance", "orders")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "SELECT * FROM silver.orders",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert!(
        ch.reads_of(HIDDEN).is_empty(),
        "the refusal precedes the engine: {:?}",
        ch.statements()
    );

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "SELECT * FROM finance.orders",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ch.reads_of(ALLOWED).len(), 1, "{:?}", ch.statements());
    assert_eq!(body["rows"].as_array().map(Vec::len), Some(1), "{body}");
}

/// `estimate` and `download` take the same path, so they answer the same.
#[tokio::test]
async fn estimate_and_download_refuse_a_hidden_table() {
    let ch = FakeCh::default();
    let server = ch.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "query:read", &[("finance", "orders")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/estimate",
        "SELECT * FROM silver.orders",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");

    insert_history_row(
        &pool,
        "q-1700000000000",
        "SELECT * FROM silver.orders",
        user,
    )
    .await;
    let (status, body) = download_csv(&router, &cookie, "q-1700000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert!(
        ch.reads_of(HIDDEN).is_empty(),
        "neither refusal may reach the engine: {:?}",
        ch.statements()
    );
}

/// A statement that names no resolvable table is refused, and an
/// unrestricted principal keeps today's answer.
#[tokio::test]
async fn a_narrowed_principals_listing_is_refused_and_an_unrestricted_one_is_unchanged() {
    let ch = FakeCh::default();
    let server = ch.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "query:read", &[("finance", "orders")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    for statement in ["SHOW TABLES FROM silver", "SHOW DATABASES"] {
        let (status, body) = post_sql(&router, &cookie, "/api/query/run", statement).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{statement}: {body}"
        );
        assert_eq!(body["error"], LISTING_REFUSAL, "{statement}: {body}");
    }
    // A system table names itself, so it is a table the role does not hold.
    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "SELECT * FROM system.tables",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["error"], "Table not found or not available to your organisation: system.tables",
        "{body}"
    );

    let admin = create_principal_with_permissions(&pool, "*:*").await;
    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = post_sql(
        &router,
        &admin_cookie,
        "/api/query/run",
        "SHOW TABLES FROM silver",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Masks still apply on an allowed table: the access check is in front of the
/// policy rewrite, not instead of it.
#[tokio::test]
async fn masks_still_apply_on_an_allowed_table() {
    let ch = FakeCh::default();
    let server = ch.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "query:read", &[("finance", "orders")]).await;
    // The mask names the ROLE, which is how policy conditions match.
    let role: String = sqlx::query_scalar(
        "SELECT r.name FROM role r JOIN app_user_role a ON a.role_id = r.id WHERE a.user_id = $1",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .expect("the user's role name");
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "gov7-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: role.clone(),
            resources: ALLOWED.to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({"roles": [role], "table": ALLOWED, "mask": ["email"]}).to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .expect("seed the mask");
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "SELECT * FROM finance.orders",
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let reads = ch.reads_of(ALLOWED);
    assert_eq!(reads.len(), 1, "{:?}", ch.statements());
    assert!(
        reads[0].contains("replaceRegexpOne"),
        "the mask must still wrap the allowed table: {}",
        reads[0]
    );
}

// ── Dashboards and embeds ───────────────────────────────────────────────

/// The two marts one board's tiles read: one the narrowed role holds, one it
/// does not (`GOV-7-AC4`).
const OK_MART: &str = "mart_gov7_ok";
const HIDDEN_MART: &str = "mart_gov7_hidden";
const OK_TILE: &str = "c_gold";
const HIDDEN_TILE: &str = "c_silver";

/// A board tile on `mart`. A dashboard tile always reads `serving.<mart>`
/// (`support::stored_chart_sql` rebuilds the statement from the definition),
/// so the two tiles differ by mart, not by stored SQL.
fn board_chart(id: &str, mart: &str) -> Value {
    let spec = json!({
        "id": id, "title": id, "kind": "kpi", "mart": mart,
        "sql": format!("SELECT count() AS n FROM serving.{mart}"), "x": "", "y": "n"
    });
    let def = json!({
        "title": id, "mart": mart, "kind": "kpi", "measures": ["n"], "aggregate": "count"
    });
    json!({
        "id": id, "board": "", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

/// A `ClickHouse` serving one board with a Gold tile and a Silver tile.
#[derive(Clone)]
struct BoardFake {
    log: Arc<Mutex<Vec<String>>>,
}

impl BoardFake {
    fn new() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn statements(&self) -> Vec<String> {
        self.log.lock().expect("the statement log").clone()
    }

    async fn serve(&self) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        server
    }
}

impl Respond for BoardFake {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log
            .lock()
            .expect("the statement log")
            .push(sql.clone());
        if sql.contains("FROM console.bi_chart") {
            return rows(
                &["id", "board", "created_by", "created_at", "spec_json"],
                &[
                    board_chart(OK_TILE, OK_MART),
                    board_chart(HIDDEN_TILE, HIDDEN_MART),
                ],
            );
        }
        if sql.contains("FROM console.bi_board") {
            return rows(&["id", "name"], &[]);
        }
        if sql.contains("FROM system.columns") {
            return rows(
                &[
                    "table",
                    "name",
                    "type",
                    "default_kind",
                    "default_expression",
                ],
                &[json!({"table": "orders", "name": "n", "type": "UInt64",
                          "default_kind": "", "default_expression": ""})],
            );
        }
        if sql.contains(&format!("FROM serving.{OK_MART}"))
            || sql.contains(&format!("FROM serving.{HIDDEN_MART}"))
        {
            return rows(&["n"], &[json!({"n": 7})]);
        }
        rows(&[], &[])
    }
}

#[tokio::test]
async fn a_dashboard_tile_on_a_hidden_table_is_not_found_while_the_other_tiles_render() {
    let fake = BoardFake::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "dashboard:read", &[("serving", OK_MART)]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");

    assert_eq!(
        body["results"][HIDDEN_TILE]["error"],
        format!("Table not found or not available to your organisation: serving.{HIDDEN_MART}"),
        "{body}"
    );
    assert!(
        body["results"][OK_TILE].get("rows").is_some(),
        "the allowed tile still renders: {body}"
    );
    assert!(
        fake.statements()
            .iter()
            .all(|sql| !sql.contains(&format!("FROM serving.{HIDDEN_MART}"))),
        "the hidden tile never reaches the engine: {:?}",
        fake.statements()
    );
}

// ── Combined charts: a question outside the viewer's access ─────────────

/// QA `LH-BI-13-03` (`BI-51`, plan T7): a board with one bar question on
/// [`LAYER_BASE`] and one line question on [`LAYER_MART`], so the second
/// question can be hidden from the viewer or masked for their role.
const LAYER_BASE: &str = "mart_bi51_base";
const LAYER_MART: &str = "mart_bi51_cost";
const LAYER_TILE: &str = "c_bi51";

fn layered_board_chart() -> Value {
    let spec = json!({
        "id": LAYER_TILE, "title": LAYER_TILE, "kind": "bar",
        "mart": LAYER_BASE,
        "sql": format!("SELECT kota, round(sum(n)) AS n FROM serving.{LAYER_BASE} GROUP BY kota"),
        "x": "kota", "y": "n",
    });
    let def = json!({
        "title": LAYER_TILE, "kind": "bar", "mart": LAYER_BASE, "dimension": "kota",
        "measures": ["n"], "aggregate": "sum", "limit": 20,
        "layers": [{
            "mart": LAYER_MART, "dimension": "kota", "measures": ["omzet"],
            "aggregate": "sum", "draw": "line", "axis": "right",
        }],
    });
    json!({
        "id": LAYER_TILE, "board": "", "created_by": "ui",
        "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

/// The engine's refusal of a sum over the `***` a mask produces (`BI-51`).
const MASKED_SUM_REFUSAL: &str = "Code: 43. DB::Exception: Illegal type String of argument for \
     aggregate function sum. (ILLEGAL_TYPE_OF_ARGUMENT)";

/// How the fake answers a read of the second mart (`R2`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MartRead {
    /// Only a statement that carries the mask's own rewrite
    /// (`replaceRegexpOne`) is refused; any other read of the mart gets no
    /// rows. A test on this mode fails when masking stops, which an
    /// unconditional refusal would hide.
    MaskedOnly,
    /// Every read of the mart is refused, masked or not.
    Always,
}

/// A `ClickHouse` serving the one combined chart: both marts' columns, the
/// base question with one row, and a refusal for the second question's read.
#[derive(Clone)]
struct LayerFake {
    log: Arc<Mutex<Vec<String>>>,
    mart_read: MartRead,
}

impl LayerFake {
    fn new() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            mart_read: MartRead::Always,
        }
    }

    /// R2: only the masked read of the second mart fails.
    fn masked_only() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            mart_read: MartRead::MaskedOnly,
        }
    }

    fn statements(&self) -> Vec<String> {
        self.log.lock().expect("the statement log").clone()
    }

    async fn serve(&self) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        server
    }
}

impl Respond for LayerFake {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log
            .lock()
            .expect("the statement log")
            .push(sql.clone());
        if sql.contains("FROM console.bi_chart") {
            return rows(
                &["id", "board", "created_by", "created_at", "spec_json"],
                &[layered_board_chart()],
            );
        }
        if sql.contains("FROM console.bi_board") {
            return rows(&["id", "name"], &[]);
        }
        if sql.contains("FROM system.columns") {
            let column = |table: &str, name: &str, ty: &str| {
                json!({"table": table, "name": name, "type": ty,
                       "default_kind": "", "default_expression": ""})
            };
            let meta = &[
                "table",
                "name",
                "type",
                "default_kind",
                "default_expression",
            ];
            // The rewrite asks one table at a time with `AND table=`: answer
            // with that mart's columns only.
            if sql.contains("AND table=") {
                if sql.contains(LAYER_BASE) {
                    return rows(
                        meta,
                        &[
                            column(LAYER_BASE, "kota", "String"),
                            column(LAYER_BASE, "n", "UInt64"),
                        ],
                    );
                }
                if sql.contains(LAYER_MART) {
                    return rows(
                        meta,
                        &[
                            column(LAYER_MART, "kota", "String"),
                            column(LAYER_MART, "omzet", "UInt64"),
                        ],
                    );
                }
            }
            return rows(
                meta,
                &[
                    column(LAYER_BASE, "kota", "String"),
                    column(LAYER_BASE, "n", "UInt64"),
                    column(LAYER_MART, "kota", "String"),
                    column(LAYER_MART, "omzet", "UInt64"),
                ],
            );
        }
        // No table is a view: the rewrite's view refusal stays out of the way.
        if sql.contains("FROM system.tables") {
            return rows(&["database", "name", "engine", "create_table_query"], &[]);
        }
        // The second question's read: what the engine says when a mask turned
        // the number into `***`. R2: in `MaskedOnly` mode only a statement
        // that carries the mask's rewrite is refused; a read without it gets
        // no rows, so a change that stops masking shows up as a pass-through.
        if sql.contains(&format!("FROM serving.{LAYER_MART}")) {
            return match self.mart_read {
                MartRead::MaskedOnly if !sql.contains("replaceRegexpOne") => rows(&[], &[]),
                MartRead::MaskedOnly | MartRead::Always => {
                    ResponseTemplate::new(500).set_body_string(MASKED_SUM_REFUSAL)
                }
            };
        }
        if sql.contains(&format!("FROM serving.{LAYER_BASE}")) {
            return rows(&["kota", "n"], &[json!({"kota": "A", "n": 1})]);
        }
        rows(&[], &[])
    }
}

/// A `GET /api/dashboard` as `cookie`, with its body and status.
async fn dashboard_as(router: &axum::Router, cookie: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");
    (status, body)
}

/// QA `LH-BI-13-03` (`BI-51`, D16, D18): a question on a table the viewer's
/// role does not read is marked as access, without the gate's text, and that
/// table is never read.
#[tokio::test]
async fn a_question_on_a_table_the_viewers_role_does_not_read_is_marked_as_access() {
    let fake = LayerFake::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "dashboard:read", &[("serving", LAYER_BASE)]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = dashboard_as(&router, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tile = &body["results"][LAYER_TILE];
    assert!(
        tile.get("rows").is_some(),
        "the base question renders: {body}"
    );
    let layer = &tile["layers"][0];
    assert_eq!(layer["cause"], json!("access"), "{layer}");
    assert_eq!(layer["error"], "Not available with your access.", "{layer}");
    assert!(layer.get("errorId").is_none(), "{layer}");
    let text = layer.to_string();
    assert!(!text.contains("Table not found"), "{text}");
    assert!(
        fake.statements()
            .iter()
            .all(|sql| !sql.contains(&format!("FROM serving.{LAYER_MART}"))),
        "the hidden question never reaches the engine: {:?}",
        fake.statements()
    );
}

/// QA `LH-BI-13-03` (`BI-51`, D16): a question that sums a column masked for
/// the viewer is marked as access, and the mask's name and the engine's text
/// stay out of the entry.
#[tokio::test]
async fn a_question_that_sums_a_column_masked_for_the_viewer_is_marked_as_access() {
    let fake = LayerFake::masked_only();
    let server = fake.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(
        &pool,
        "dashboard:read",
        &[("serving", LAYER_BASE), ("serving", LAYER_MART)],
    )
    .await;
    let role: String = sqlx::query_scalar(
        "SELECT r.name FROM role r JOIN app_user_role a ON a.role_id = r.id WHERE a.user_id = $1",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .expect("the user's role name");
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "bi51-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: role.clone(),
            resources: format!("serving.{LAYER_MART}"),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({
                    "roles": [role],
                    "table": format!("serving.{LAYER_MART}"),
                    "mask": ["omzet"],
                })
                .to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .expect("seed the mask");
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = dashboard_as(&router, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let layer = &body["results"][LAYER_TILE]["layers"][0];
    assert_eq!(layer["cause"], json!("access"), "{layer}");
    assert_eq!(layer["error"], "Not available with your access.", "{layer}");
    assert!(layer.get("errorId").is_none(), "{layer}");
    let text = layer.to_string();
    for needle in ["omzet", "Code: 43", "Illegal"] {
        assert!(!text.contains(needle), "{needle} leaked: {text}");
    }
    // R2: the refusal was the masked read's own — the statement the rewrite
    // produced carries `replaceRegexpOne` over the mart this question sums.
    assert!(
        fake.statements()
            .iter()
            .any(|sql| sql.contains("replaceRegexpOne")
                && sql.contains(&format!("FROM serving.{LAYER_MART}"))),
        "the masked read reached the engine: {:?}",
        fake.statements()
    );
}

/// QA `LH-BI-13-03` (`BI-51`, D16): an engine failure the viewer's access does
/// not explain keeps its sentence and its reference.
#[tokio::test]
async fn a_question_the_engine_refuses_for_a_viewer_without_a_mask_keeps_its_reference() {
    let fake = LayerFake::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(
        &pool,
        "dashboard:read",
        &[("serving", LAYER_BASE), ("serving", LAYER_MART)],
    )
    .await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = dashboard_as(&router, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let layer = &body["results"][LAYER_TILE]["layers"][0];
    assert_eq!(layer["error"], "This chart could not be loaded.", "{layer}");
    assert!(layer["errorId"].is_string(), "{layer}");
    assert!(layer.get("cause").is_none(), "{layer}");
}

// ── Embeds ──────────────────────────────────────────────────────────────

/// The board an embed token names, and the secret the token is signed with.
const EMBED_BOARD: &str = "b_embed";
const EMBED_SECRET: &str = "test-embed-secret-not-real";

/// A `ClickHouse` serving one embeddable board with the same two tiles.
#[derive(Clone)]
struct EmbedFake {
    log: Arc<Mutex<Vec<String>>>,
}

impl Respond for EmbedFake {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log
            .lock()
            .expect("the statement log")
            .push(sql.clone());
        // The board's state lookup (live, nobody's personal board).
        if sql.starts_with("SELECT is_deleted, owner_id FROM console.bi_board") {
            return rows(
                &["is_deleted", "owner_id"],
                &[json!({"is_deleted": 0, "owner_id": ""})],
            );
        }
        // No withdrawal, no origin list, no filter modes: the embed is open.
        if sql.contains("FROM console.bi_board_embed_event") {
            return rows(&["kind"], &[]);
        }
        if sql.contains("FROM console.bi_chart") {
            let on_board = |id: &str, mart: &str| {
                let mut row = board_chart(id, mart);
                row["board"] = json!(EMBED_BOARD);
                row
            };
            return rows(
                &["id", "board", "created_by", "created_at", "spec_json"],
                &[
                    on_board(OK_TILE, OK_MART),
                    on_board(HIDDEN_TILE, HIDDEN_MART),
                ],
            );
        }
        if sql.contains("FROM console.bi_board") {
            return rows(
                &[
                    "id",
                    "name",
                    "description",
                    "created_by",
                    "layout_json",
                    "filters_json",
                    "public_token",
                    "embed_enabled",
                    "folder_id",
                    "created_at",
                    "owner_id",
                    "tabs_json",
                    "embed_revoked_before",
                    "embed_revoked_jti_json",
                    "embed_origins_json",
                ],
                &[json!({
                    "id": EMBED_BOARD, "name": "Embedded", "description": "", "created_by": "t",
                    "layout_json": "", "filters_json": "", "public_token": "p_b_embed",
                    "embed_enabled": "1", "folder_id": "", "created_at": "2026-10-10 00:00:00",
                    "owner_id": "", "tabs_json": "", "embed_revoked_before": "0",
                    "embed_revoked_jti_json": "[]", "embed_origins_json": "[]"
                })],
            );
        }
        if sql.contains("FROM system.columns") {
            return rows(
                &[
                    "table",
                    "name",
                    "type",
                    "default_kind",
                    "default_expression",
                ],
                &[json!({"table": "orders", "name": "n", "type": "UInt64",
                          "default_kind": "", "default_expression": ""})],
            );
        }
        if sql.contains(&format!("FROM serving.{OK_MART}"))
            || sql.contains(&format!("FROM serving.{HIDDEN_MART}"))
        {
            return rows(&["n"], &[json!({"n": 7})]);
        }
        rows(&[], &[])
    }
}

/// `GOV-7-AC9` and D5: an embed reads as the fixed `Dashboard Viewer` role, so
/// narrowing that role narrows every embed — the hidden tile is refused with
/// the tenant gate's sentence, the other still renders.
#[tokio::test]
async fn an_embed_follows_the_dashboard_viewer_roles_access() {
    use lakehouse_embed::{EmbedClaims, EmbedResource, sign_embed, unix_now};

    let fake = EmbedFake {
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    env.insert("EMBED_SECRET".to_owned(), EMBED_SECRET.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    // `GOV-7-AC9`: the Dashboard Viewer role is narrowed to one schema.
    narrow_role_by_name(&pool, "Dashboard Viewer", &[("serving", OK_MART)]).await;

    let issued = unix_now() - 5.0;
    let jwt = sign_embed(
        &EmbedClaims {
            resource: Some(EmbedResource {
                chart: None,
                dashboard: Some(EMBED_BOARD.to_owned()),
            }),
            params: None,
            exp: Some(issued + 3600.0),
            iat: Some(issued),
            jti: None,
        },
        EMBED_SECRET,
    );
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/embed/data")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "jwt": jwt }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");

    assert_eq!(
        body["results"][HIDDEN_TILE]["error"],
        format!("Table not found or not available to your organisation: serving.{HIDDEN_MART}"),
        "{body}"
    );
    assert!(
        body["results"][OK_TILE].get("rows").is_some(),
        "the allowed tile still renders in the embed: {body}"
    );
    assert!(
        fake.log
            .lock()
            .expect("the statement log")
            .iter()
            .all(|sql| !sql.contains(&format!("FROM serving.{HIDDEN_MART}"))),
        "the hidden tile never reaches the engine"
    );
}

// ── Alerts ──────────────────────────────────────────────────────────────

/// The one alert rule these tests hold.
const RULE: &str = "al_gov7";
const RUN_TOKEN: &str = "run-token-not-real";

fn rule_row(owner: Uuid) -> Value {
    json!({
        "id": RULE, "name": "Visitors over the line", "type": "alert", "mart": "mart_x",
        "measure": "visitors", "agg": "sum", "op": ">", "threshold": "100", "board": "",
        "channel": "email", "target": "owner@test.invalid", "enabled": "1",
        "created_at": "2026-11-02 00:00:00", "severity": "", "pipeline": "", "metric": "",
        "connector": "", "asset": "", "chart": "", "series": "", "condition": "",
        "owner_id": owner.to_string(), "filters_json": "[]",
    })
}

#[derive(Clone)]
struct AlertFake {
    owner: Uuid,
    log: Arc<Mutex<Vec<String>>>,
}

impl Respond for AlertFake {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log
            .lock()
            .expect("the statement log")
            .push(sql.clone());
        if sql.contains("FROM console.alert_rule") {
            return rows(&["id"], &[rule_row(self.owner)]);
        }
        if sql.contains("FROM console.bi_chart") {
            return rows(&["id"], &[]);
        }
        if sql.contains("FROM system.columns") {
            return rows(
                &[
                    "table",
                    "name",
                    "type",
                    "default_kind",
                    "default_expression",
                ],
                &[
                    json!({"table": "mart_x", "name": "visitors", "type": "Float64",
                          "default_kind": "", "default_expression": ""}),
                ],
            );
        }
        if sql.contains("FROM serving.mart_x") {
            return rows(&["v"], &[json!({"v": 150.0})]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

/// D5 and `GOV-7-AC10`: a rule owned by a narrowed person records the
/// not-found sentence on its run instead of reading the table.
#[tokio::test]
async fn an_alert_owned_by_a_narrowed_user_records_not_found() {
    let owner = Uuid::new_v4();
    let fake = AlertFake {
        owner,
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    env.insert("ALERTS_RUN_TOKEN".to_owned(), RUN_TOKEN.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    // The rule's owner: `alert:write` (the gate requires it), in the catalog's
    // tenant, narrowed to the finance schema, so `serving.mart_x` is hidden.
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Alert Owner', \
         'alert:write, alert:read, dashboard:read', 'test')",
    )
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert the owner role");
    sqlx::query("INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Owner', 'owner-gov7', 'owner@test.invalid', 'active')")
        .bind(owner)
        .execute(&pool)
        .await
        .expect("insert the owner");
    sqlx::query(
        "INSERT INTO app_user_role (user_id, role_id) SELECT $1, id FROM role WHERE name = 'Alert Owner'",
    )
    .bind(owner)
    .execute(&pool)
    .await
    .expect("give the owner its role");
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(owner)
        .bind(Uuid::parse_str(CATALOG_TENANT).expect("a seeded tenant id"))
        .execute(&pool)
        .await
        .expect("add the membership");
    narrow(&pool, owner, &[("finance", "orders")]).await;
    let cookie = session_cookie_for_user(&pool, owner).await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/alerts/run?id={RULE}"))
                .header("cookie", cookie)
                .header("x-run-token", RUN_TOKEN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");

    assert_eq!(
        body["results"][0]["skipped"],
        "Table not found or not available to your organisation: serving.mart_x",
        "the run records the not-found sentence, not a generic failure: {body}"
    );
    assert!(
        fake.log
            .lock()
            .expect("the statement log")
            .iter()
            .all(|sql| !sql.contains("FROM serving.mart_x")),
        "the refusal precedes the engine: {:?}",
        fake.log
    );
}

// ── Authored pipelines ──────────────────────────────────────────────────

/// D5: an authored pipeline's compile call reads as its author, so a narrowed
/// author's hidden table is the tenant gate's not-found.
///
/// BLD-1 part 1 review R1-B2: the route is `.../compile` now — it compiles the
/// stored definition (whose input is the hidden table) instead of taking SQL.
#[tokio::test]
async fn an_authored_pipeline_compile_call_for_a_narrowed_author_is_not_found() {
    let ch = FakeCh::default();
    let server = ch.serve().await;
    let TestApp { router, pool } = app(&server).await;
    // The author: narrowed to finance, in the catalog's tenant.
    let author = narrowed(&pool, "query:read", &[("finance", "orders")]).await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let tenant = Uuid::parse_str(CATALOG_TENANT).expect("a seeded tenant id");
    let pipeline = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            // BLD-1 part 1 review R1-B2: the run route compiles the stored
            // definition, so the table the author may not read is the
            // pipeline's input.
            inputs: vec![pipelines::PipelineInput {
                name: "source".to_owned(),
                zone: "silver".to_owned(),
                table: "orders".to_owned(),
            }],
            steps: Vec::new(),
            name: "gov7-enforce".to_owned(),
            kind: "batch".to_owned(),
            source_zone: "silver".to_owned(),
            source_table: "orders".to_owned(),
            target_zone: "silver".to_owned(),
            target_table: "orders_clean".to_owned(),
            schedule: "0 3 * * *".to_owned(),
            description: None,
            incremental_column: None,
            transforms: Vec::new(),
            fbic_enabled: false,
            max_retries: None,
            owner: None,
            tenant_id: Some(tenant),
            depends_on: Vec::new(),
            connector_id: None,
            id: None,
            run_as: Some(author),
            target_lag: None,
        },
        Some(author),
    )
    .await
    .expect("seed the pipeline")
    .id;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/pipelines/runnable/{pipeline}/compile"))
                .header("cookie", admin)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
}

// ── Catalog lists and details (GOV-7 D6, AC1, AC6) ──────────────────────

/// The Bronze registry's `dataset_catalog` read, whose result rows carry the
/// `table_name` a row's `tableKey` is built from (`catalog.rs`
/// `assemble_bronze_rows`). Two datasets, so a grant for one can be told
/// apart from a grant for neither.
const BRONZE_CATALOG_SQL_PREFIX: &str =
    "SELECT slug, title, description, tier, updated_at, table_name";

/// A `ClickHouse` serving the catalog assembly with two Silver tables, one of
/// which the narrowed role may not read, and two Bronze registry rows.
#[derive(Clone)]
struct CatalogFake {
    log: Arc<Mutex<Vec<String>>>,
}

impl Respond for CatalogFake {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log
            .lock()
            .expect("the statement log")
            .push(sql.clone());
        // `GOV-7` D9: the Bronze registry read, which is what gives a Bronze
        // row the table it is addressed by. Its own columns are the registry
        // names; the secondary registry's copy of the same read and the clash
        // count are answered empty by the fall-through below.
        if sql.starts_with(BRONZE_CATALOG_SQL_PREFIX) {
            return rows(
                &[
                    "slug",
                    "title",
                    "description",
                    "tier",
                    "updated_at",
                    "table_name",
                    "lag",
                ],
                &[
                    json!({
                        "slug": "ds_commerce_orders",
                        "title": "Commerce orders",
                        "description": "Bronze orders",
                        "tier": "primer",
                        "updated_at": "2026-11-01T00:00:00Z",
                        "table_name": "commerce_orders",
                        "lag": "60",
                    }),
                    json!({
                        "slug": "ds_finance_payments",
                        "title": "Finance payments",
                        "description": "Bronze payments",
                        "tier": "primer",
                        "updated_at": "2026-11-01T00:00:00Z",
                        "table_name": "finance_payments",
                        "lag": "60",
                    }),
                ],
            );
        }
        // The catalog's own reads alias the columns (`database db`), so the
        // fake answers with the names the real engine would send back.
        if sql.contains("FROM system.tables") {
            return rows(
                &["db", "name", "engine"],
                &[
                    json!({"db": "silver", "name": "visible_x", "engine": "MergeTree"}),
                    json!({"db": "silver", "name": "hidden_x", "engine": "MergeTree"}),
                ],
            );
        }
        if sql.contains("FROM system.columns") {
            return rows(
                &["db", "table", "n"],
                &[
                    json!({"db": "silver", "table": "visible_x", "n": "2"}),
                    json!({"db": "silver", "table": "hidden_x", "n": "2"}),
                ],
            );
        }
        rows(&[], &[])
    }
}

async fn send_get(router: &axum::Router, cookie: &str, uri: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
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

/// `GOV-7-AC1`/`AC6`: the catalog list omits a hidden table, and its detail is
/// the SAME 404 an unknown id answers.
#[tokio::test]
async fn the_catalog_list_omits_a_hidden_table_and_its_detail_is_the_unknown_404() {
    let fake = CatalogFake {
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(
        &pool,
        "catalog:read, query:read",
        &[("silver", "visible_x")],
    )
    .await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = send_get(&router, &cookie, "/api/catalog").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["assets"]
        .as_array()
        .expect("assets")
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(ids.contains(&"silver.visible_x"), "{body}");
    assert!(
        !ids.contains(&"silver.hidden_x"),
        "a hidden table must not be listed: {body}"
    );
    // The namespace's own count is of the visible tables.
    assert_eq!(body["namespaces"][0]["assetCount"], json!(1), "{body}");

    let (status, hidden) = send_get(&router, &cookie, "/api/catalog/silver.hidden_x").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
    let (status, unknown) = send_get(&router, &cookie, "/api/catalog/silver.no_such_x").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    assert_eq!(
        hidden, unknown,
        "a hidden table answers the same 404 body an unknown id answers"
    );
}

/// `GOV-7` D9 (`S2 review 1 B2`): a narrowed role granted one Bronze table
/// finds that Bronze row in both catalog lists, and only that one.
#[tokio::test]
async fn the_catalog_lists_the_bronze_table_a_narrowed_role_is_granted() {
    let fake = CatalogFake {
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "catalog:read", &[("bronze", "commerce_orders")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = send_get(&router, &cookie, "/api/catalog").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["assets"]
        .as_array()
        .expect("assets")
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"ds_commerce_orders"),
        "a granted Bronze table must be listed: {body}"
    );
    assert!(
        !ids.contains(&"ds_finance_payments"),
        "a Bronze table the role is not granted must not be listed: {body}"
    );

    let (status, paged) = send_get(&router, &cookie, "/api/catalog/query").await;
    assert_eq!(status, StatusCode::OK, "{paged}");
    let items = paged["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{paged}");
    assert_eq!(items[0]["id"], json!("ds_commerce_orders"), "{paged}");
    assert_eq!(paged["totalItems"], json!(1), "{paged}");
}

/// `GOV-7` D9 (`S2 review 1 B2`): a grant for a Silver table does not cover a
/// Bronze table, so such a role still sees no Bronze row.
#[tokio::test]
async fn a_bronze_row_stays_hidden_from_a_role_granted_only_a_silver_table() {
    let fake = CatalogFake {
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "catalog:read", &[("silver", "visible_x")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = send_get(&router, &cookie, "/api/catalog").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["assets"]
        .as_array()
        .expect("assets")
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(ids.contains(&"silver.visible_x"), "{body}");
    assert!(
        !ids.contains(&"ds_commerce_orders") && !ids.contains(&"ds_finance_payments"),
        "a Silver grant must not reveal a Bronze row: {body}"
    );

    let (status, paged) = send_get(&router, &cookie, "/api/catalog/query").await;
    assert_eq!(status, StatusCode::OK, "{paged}");
    let items = paged["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{paged}");
    assert_eq!(items[0]["id"], json!("silver.visible_x"), "{paged}");
}

/// The paged Data Explorer query filters the same snapshot before it counts.
#[tokio::test]
async fn the_paged_catalog_query_omits_a_hidden_table() {
    let fake = CatalogFake {
        log: Arc::new(Mutex::new(Vec::new())),
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "catalog:read", &[("silver", "visible_x")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = send_get(&router, &cookie, "/api/catalog/query").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["id"], json!("silver.visible_x"), "{body}");
    assert_eq!(body["totalItems"], json!(1), "{body}");
}

/// D6: the chart builder's mart picker drops a hidden mart, and a mart
/// request for it answers the mart-not-found 404.
#[tokio::test]
async fn the_dashboard_field_picker_drops_a_hidden_mart() {
    #[derive(Clone)]
    struct MartFake;
    impl Respond for MartFake {
        fn respond(&self, request: &MockRequest) -> ResponseTemplate {
            let sql = String::from_utf8_lossy(&request.body).into_owned();
            if sql.contains("FROM system.tables") {
                return rows(
                    &["name", "total_rows"],
                    &[
                        json!({"name": "mart_ok", "total_rows": "10"}),
                        json!({"name": "mart_hidden", "total_rows": "10"}),
                    ],
                );
            }
            rows(&[], &[])
        }
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(MartFake)
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server).await;
    let user = narrowed(&pool, "dashboard:read", &[("serving", "mart_ok")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = send_get(&router, &cookie, "/api/dashboard/fields").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["marts"]
        .as_array()
        .expect("marts")
        .iter()
        .filter_map(|m| m["name"].as_str())
        .collect();
    assert_eq!(names, vec!["mart_ok"], "{body}");

    let (status, hidden) = send_get(
        &router,
        &cookie,
        "/api/dashboard/fields?mart=serving.mart_hidden",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
    let (status, unknown) = send_get(
        &router,
        &cookie,
        "/api/dashboard/fields?mart=serving.mart_nope",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
}
