//! Read-only lakehouse-data tools: `run_sql`, `list_datasets`,
//! `describe_dataset`, `get_lineage`, `get_quality`, `describe_mart`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use lakehouse_auth::Principal;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::SqlLiteral;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::routes::support::{is_numeric_type, strip_non_ident};
use crate::state::AppState;

// WS7 item F5: `pub(in crate::routes)` so `routes::agent::schema_context`
// can run this SAME union query for its new Bronze catalog section,
// rather than re-deriving an equivalent string that could drift from this
// one.
pub(in crate::routes) const CATALOG_UNION: &str = "(SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta.dataset_catalog` \
     UNION ALL SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta_sec.dataset_catalog`)";

/// WS7 item C3: delegates to the SAME `routes::query::run` Query Studio
/// and saved queries already call — closing the divergent-guard gap this
/// task's own name cites (`WS7 plan §0 item 7`): before this change,
/// `run_sql` called `ch.query` directly, so `sql_rewrite::enforce`'s
/// masking/row-filter/table-function/sensitive-table/view refusal (WS7
/// items B3-B6, wired into `routes::query::run` by WS7 item C2) never ran
/// for a copilot-issued query at all, even though the exact same
/// `Extension(principal)` reached this function. See
/// `tools::queries::run_saved_query`'s doc comment for why "call the real
/// handler, read its JSON back" is the preferred reuse shape over
/// re-implementing the guard a second time — this function now follows
/// that exact same shape.
///
/// `is_read_only_sql`/`dry_run_sql` below stay in place as an ADDITIONAL,
/// cheaper first filter (WS7 item F4) — never a substitute for
/// `routes::query::run`'s own `is_read_only`/`sql_rewrite::enforce`,
/// which still run on every call reaching this function, principal
/// present or not.
pub(super) async fn run_sql(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let sql = arg_str(args, "sql");
    if !crate::routes::agent::is_read_only_sql(&sql) {
        return json!({ "error": "Hanya SELECT diizinkan." });
    }
    // Same fail-closed shape `tools::queries::run_saved_query` uses for a
    // headless (schedule/service-token) caller with no interactive
    // principal to run as — `routes::query::run` 401s with none, for
    // every engine, so there is no honest value to forward here either.
    // Checked BEFORE `dry_run_sql` below (a real `ClickHouse` round trip)
    // so a headless caller is refused without spending that call at all.
    let Some(principal) = principal else {
        return json!({
            "error": "menjalankan SQL memerlukan pengguna yang terautentikasi; panggilan ini \
                       dipicu oleh jadwal, yang tidak memiliki pengguna untuk dijalankan",
        });
    };
    // WS7 item F4: `is_read_only_sql` above is a regex-shaped first
    // filter (unchanged, still checked first, defense-in-depth) — it can
    // disagree with what `ClickHouse` itself will actually execute for a
    // dialect construct the regex never anticipated. `dry_run_sql` is an
    // ADDITIONAL, engine-verified check: it asks `ClickHouse`'s own
    // `EXPLAIN AST` what KIND of statement this really is (never executed
    // by `EXPLAIN AST` itself) and refuses whenever the engine's own
    // answer is not a `SELECT`, closing the gap where a model-composed
    // SQL string could smuggle a non-`SELECT` construct past the regex.
    let dry_run = dry_run_sql(&state.clickhouse, args).await;
    if dry_run.get("error").is_some() {
        return dry_run;
    }
    let body = axum::body::Bytes::from(json!({ "sql": sql }).to_string());
    let extension = Some(axum::Extension(principal.clone()));
    api_result_to_value(
        crate::routes::query::run(axum::extract::State(state.clone()), extension, body).await,
    )
    .await
}

/// Dry-runs `args["sql"]` via `ClickHouse`'s own `EXPLAIN AST`, refusing
/// whenever the engine's OWN reported statement kind is not a `SELECT`
/// (WS7 plan §0 item 6, verified live against `lake-clickhouse` during
/// planning: `EXPLAIN AST` never executes the statement, and its root
/// node's label names the real statement kind — `SelectWithUnionQuery`
/// for every `SELECT` form including `WITH`/`UNION`, `InsertQuery` for an
/// `INSERT`, `AlterQuery` for an `ALTER`, `DropQuery` for a `DROP`, …).
///
/// This is an ADDITIONAL, engine-verified check ahead of
/// [`crate::routes::agent::is_read_only_sql`]'s regex-based guard
/// (unchanged, still checked first by [`run_sql`] itself) — never a
/// replacement for it: a third-party parser's dialect (or a hand-rolled
/// regex) can disagree with what `ClickHouse` will actually execute, so
/// this asks the engine itself rather than guessing. Returns
/// `{"error": "..."}` (naming the REAL reported statement kind, never
/// echoing the user's raw SQL back, per `AGENTS.md` rule 4) when refused,
/// or `{}` when the statement is confirmed a `SELECT`.
pub(super) async fn dry_run_sql(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let sql = arg_str(args, "sql");
    let rows = match ch.rows(&format!("EXPLAIN AST {sql}"), None).await {
        Ok(r) => r,
        Err(err) => return json!({ "error": format!("dry run gagal: {err}") }),
    };
    let first_line = rows
        .first()
        .and_then(|row| row.values().next())
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if first_line.starts_with("SelectWithUnionQuery") {
        return json!({});
    }
    let kind = first_line.split_whitespace().next().unwrap_or("unknown");
    json!({ "error": format!("hanya SELECT yang diizinkan (EXPLAIN AST melaporkan {kind})") })
}

pub(super) async fn list_datasets(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let rows = match ch
        .rows(
            &format!("SELECT slug, title, tier FROM {CATALOG_UNION} LIMIT 500"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let term = arg_str(args, "search").to_lowercase();
    let tier = arg_str(args, "tier");
    let hits: Vec<&Map<String, Value>> = rows
        .iter()
        .filter(|r| {
            let row_tier = r.get("tier").and_then(Value::as_str).unwrap_or("");
            if !tier.is_empty() && row_tier != tier {
                return false;
            }
            if term.is_empty() {
                return true;
            }
            let title = r.get("title").and_then(Value::as_str).unwrap_or("");
            let slug = r.get("slug").and_then(Value::as_str).unwrap_or("");
            format!("{title} {slug}").to_lowercase().contains(&term)
        })
        .take(40)
        .collect();
    json!({ "total": hits.len(), "datasets": hits })
}

pub(super) async fn describe_dataset(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let slug = SqlLiteral::from(arg_str(args, "slug"));
    let meta_rows = match ch
        .rows(
            &format!(
                "SELECT title, table_name, tier FROM {CATALOG_UNION} WHERE slug={slug} LIMIT 1"
            ),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let Some(meta) = meta_rows.first() else {
        return json!({ "error": "dataset tidak ditemukan" });
    };
    let table = meta.get("table_name").and_then(Value::as_str).unwrap_or("");
    let cols = match ch
        .rows(
            &format!(
                "SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta.dataset_column` WHERE slug={slug} \
                 UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta_sec.dataset_column` WHERE slug={slug}"
            ),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let rows = ch
        .rows(
            &format!("SELECT toString(count()) n FROM silver.`{table}`"),
            None,
        )
        .await
        .ok()
        .and_then(|r| {
            r.first()
                .and_then(|row| row.get("n").and_then(Value::as_str))
                .and_then(|s| s.parse::<i64>().ok())
        })
        .unwrap_or(0);
    json!({
        "title": meta.get("title"),
        "tier": meta.get("tier"),
        "table": table,
        "rows": rows,
        "columns": cols,
    })
}

pub(super) async fn get_lineage(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let slug = SqlLiteral::from(arg_str(args, "slug"));
    let meta_rows = match ch
        .rows(
            &format!("SELECT table_name, tier FROM {CATALOG_UNION} WHERE slug={slug} LIMIT 1"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let Some(meta) = meta_rows.first() else {
        return json!({ "error": "dataset tidak ditemukan" });
    };
    let table = meta
        .get("table_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let sekunder = meta.get("tier").and_then(Value::as_str) == Some("sekunder");
    let escaped_table = SqlLiteral::from(table.as_str());
    let cols = match ch
        .rows(
            &format!("SELECT kolom, tipe FROM _silver_meta.kolom_tipe WHERE tabel={escaped_table} LIMIT 100"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    let source_label = if sekunder {
        "Sumber sekunder"
    } else {
        "Satu Data Jakarta"
    };
    let mappings: Vec<String> = cols
        .iter()
        .map(|c| {
            format!(
                "{} → {}",
                c.get("kolom").and_then(Value::as_str).unwrap_or(""),
                c.get("tipe").and_then(Value::as_str).unwrap_or("")
            )
        })
        .collect();
    json!({
        "chain": format!("{source_label} → bronze.{table} → silver.{table}"),
        "columnMappings": mappings,
    })
}

pub(super) async fn get_quality(ch: &ChClient) -> Value {
    match ch
        .rows(
            "SELECT verdict, toString(count()) n FROM ( \
               SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict FROM _silver_meta.quality GROUP BY tabel, cek \
             ) GROUP BY verdict",
            None,
        )
        .await
    {
        Ok(rows) => json!({ "summary": rows }),
        Err(err) => json!({ "error": format!("quality belum tersedia: {err}") }),
    }
}

pub(super) async fn describe_mart(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let mart = strip_non_ident(&arg_str(args, "mart"));
    if mart.is_empty() {
        let rows = match ch
            .rows(
                "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
                 WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
                None,
            )
            .await
        {
            Ok(r) => r,
            Err(err) => return json!({ "error": err.to_string() }),
        };
        let marts: Vec<Value> = rows
            .iter()
            .map(|r| {
                let rows_n: i64 = r
                    .get("total_rows")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                json!({ "mart": r.get("name"), "rows": rows_n })
            })
            .collect();
        return json!({ "marts": marts });
    }
    let cols = match ch
        .rows(
            &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{mart}' ORDER BY position"),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "error": err.to_string() }),
    };
    if cols.is_empty() {
        return json!({ "error": format!("mart '{mart}' tidak ditemukan di serving.") });
    }
    let dimensions: Vec<&str> = cols
        .iter()
        .filter(|c| !is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    let measures: Vec<&str> = cols
        .iter()
        .filter(|c| is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    json!({ "mart": mart, "dimensions": dimensions, "measures": measures })
}

#[cfg(test)]
mod dry_run {
    //! WS7 item F4: proves `dry_run_sql` asks `ClickHouse`'s own
    //! `EXPLAIN AST` — not a string guess — and refuses on the engine's
    //! own reported statement kind. Response shapes match `ClickHouse`
    //! `26.7.3.19`'s real `EXPLAIN AST` output, verified live during WS7
    //! planning (this plan's §0 item 6).
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A mocked `ClickHouse` that answers every request (this test only
    /// ever issues the one `EXPLAIN AST` query) with a single `explain`
    /// row holding `explain_text` verbatim. Returns the [`MockServer`]
    /// too — it must outlive the [`ChClient`] built from its URI, or the
    /// mock stops answering before the test runs.
    async fn fake_clickhouse_explain_ast_response(explain_text: &str) -> (MockServer, ChClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"explain": explain_text}],
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        (server, ch)
    }

    #[tokio::test]
    async fn run_sql_dry_run_refuses_a_non_select_statement() {
        let (_server, ch) =
            fake_clickhouse_explain_ast_response("InsertQuery   (children 2)\n Identifier t\n")
                .await;
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("INSERT INTO t VALUES (1)"));
        let result = dry_run_sql(&ch, &args).await;
        assert_eq!(
            result,
            json!({"error": "hanya SELECT yang diizinkan (EXPLAIN AST melaporkan InsertQuery)"})
        );
    }

    #[tokio::test]
    async fn run_sql_dry_run_allows_a_select_with_a_cte_and_union() {
        let (_server, ch) = fake_clickhouse_explain_ast_response(
            "SelectWithUnionQuery (children 1)\n ExpressionList ...\n",
        )
        .await;
        let mut args = Map::new();
        args.insert(
            "sql".to_owned(),
            json!("WITH x AS (SELECT 1) SELECT * FROM x UNION ALL SELECT 2"),
        );
        assert!(dry_run_sql(&ch, &args).await.get("error").is_none());
    }
}

#[cfg(test)]
mod run_sql_delegation {
    //! WS7 item C3: `run_sql` now enforces policy through the SAME path
    //! Query Studio uses (`routes::query::run`), closing the
    //! divergent-guard gap the WS7 plan's §0 item 7 names — before this
    //! change `run_sql` called `ch.query` directly, so a governed table's
    //! `email` column was never masked for a copilot-issued query even
    //! though the exact same principal reached it. Real Postgres
    //! (`#[sqlx::test]`, a real authored policy row) plus a wiremock
    //! `ClickHouse` (`ChClient` speaks plain HTTP; there is no
    //! `last_query()`-style test double on it — every assertion below
    //! reads the mock server's own recorded requests instead, the same
    //! pattern `routes::query::tests::trino_engine` already uses), the
    //! same two harnesses `tools::queries`'s `principal_forwarding` module
    //! relies on for the equivalent `run_saved_query` proof.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::CreatePolicyInput;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

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

    fn analyst_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "alice".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    /// Mounts every `ClickHouse` response `routes::query::run`'s
    /// enforcement path needs for `SELECT * FROM serving.mart_x`: the
    /// `EXPLAIN AST` dry run (WS7 item F4, still checked first by
    /// `run_sql` itself), `system.columns` (`PolicyEngineObligations`'s
    /// real-column resolution, WS7 item C1), `system.tables` (view
    /// detection), and finally the real, masked `SELECT` the derived-table
    /// substitution produces.
    async fn mount_governed_table_responses(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("EXPLAIN AST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"explain": "SelectWithUnionQuery (children 1)\n"}],
            })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "id", "default_kind": "", "default_expression": ""},
                    {"name": "email", "default_kind": "", "default_expression": ""},
                ],
                "rows": 2,
            })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.tables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "database", "type": "String"},
                    {"name": "name", "type": "String"},
                    {"name": "engine", "type": "String"},
                    {"name": "create_table_query", "type": "String"},
                ],
                "data": [
                    {"database": "serving", "name": "mart_x", "engine": "MergeTree", "create_table_query": ""},
                ],
                "rows": 1,
            })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("replaceRegexpAll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "id", "type": "UInt64"},
                    {"name": "email", "type": "String"},
                ],
                "data": [{"id": "1", "email": "***"}],
                "rows": 1,
            })))
            .mount(server)
            .await;
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn run_sql_is_masked_the_same_way_query_studio_is(pool: PgPool) -> sqlx::Result<()> {
        lakehouse_store::governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "run-sql-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");

        let server = MockServer::start().await;
        mount_governed_table_responses(&server).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("CH_URL".to_owned(), server.uri());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT * FROM serving.mart_x"));
        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(
            result.get("error").is_none(),
            "expected a successful, masked run, got {result}"
        );
        assert!(
            result["columns"]
                .as_array()
                .expect("columns array")
                .contains(&json!("email")),
            "expected the email column to still be present (masked, not dropped): {result}"
        );

        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            requests
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body).contains("replaceRegexpAll")),
            "expected the ACTUAL query ClickHouse received to be the rewritten/masked form, \
             never the original literal SQL — this is what closes the divergent-guard gap \
             (WS7 plan §0 item 7)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn run_sql_without_a_principal_refuses_with_a_named_reason() {
        // No `ClickHouse` mock is mounted, and `query::run` is never
        // called: same fail-closed shape
        // `tools::queries::run_saved_query_without_a_principal_refuses_with_a_named_reason`
        // proves for the saved-query path — a headless (schedule/service-
        // token) caller has no honest principal to forward.
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT 1"));
        let result = run_sql(&state, None, &args).await;
        assert_eq!(
            result,
            json!({
                "error": "menjalankan SQL memerlukan pengguna yang terautentikasi; panggilan \
                           ini dipicu oleh jadwal, yang tidak memiliki pengguna untuk \
                           dijalankan",
            }),
            "expected the honest no-principal refusal, got {result}"
        );
    }
}
