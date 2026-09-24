//! Small helpers shared by the route modules.

use std::collections::{HashMap, HashSet};
use std::fmt::Display;

use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store;
use lakehouse_clickhouse::{ChClient, ChError};
use serde_json::{Map, Value, json};

/// Render any JSON value the way `String(x)` renders it in `TypeScript`,
/// with `null`/missing treated as `""` (matching `String(row[name] ?? "")`
/// in `src/app/api/catalog/[id]/route.ts`).
#[must_use]
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other @ (Value::Array(_) | Value::Object(_))) => other.to_string(),
    }
}

/// Parse a `ClickHouse` `toString(...)` column as an integer, defaulting to
/// `0` on a missing row, a missing column, or a value that doesn't parse —
/// matching `Number(x) || 0` in `TypeScript`, where `NaN` (a failed parse)
/// is falsy and also becomes `0`.
#[must_use]
pub(crate) fn num_or_zero(row: Option<&Map<String, Value>>, key: &str) -> i64 {
    row.and_then(|r| r.get(key))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// A `ClickHouse` string column, defaulting to `""` when the row or column
/// is missing.
#[must_use]
pub(crate) fn str_col<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `s.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase())` —
/// underscores become spaces, then the first letter of every word is
/// capitalized. Ported identically in `catalog/route.ts`,
/// `catalog/[id]/route.ts`.
#[must_use]
pub(crate) fn prettify(s: &str) -> String {
    let spaced = s.replace('_', " ");
    let mut out = String::with_capacity(spaced.len());
    let mut at_word_start = true;
    for c in spaced.chars() {
        if at_word_start && c.is_ascii_alphanumeric() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        // `\w` is `[A-Za-z0-9_]`; since underscores were already replaced
        // with spaces, any non-alphanumeric character starts a new word.
        at_word_start = !c.is_ascii_alphanumeric();
    }
    out
}

/// Render an error the way `String(e)` renders a thrown `Error` in
/// `TypeScript`: `"Error: <message>"`. Every ported route's outer
/// `catch (e)` block builds its 503 body with `String(e)`.
#[must_use]
pub(crate) fn js_error(err: impl Display) -> String {
    format!("Error: {err}")
}

/// `ClickHouse` type-name substrings treated as numeric, matching
/// `/Int|Float|Decimal/` in `dashboard/fields/route.ts` (and reused,
/// identically, by `ai-tools.ts`'s `describe_mart`/`suggest_dashboard`
/// tools).
const NUMERIC_TYPE_MARKERS: [&str; 3] = ["Int", "Float", "Decimal"];

/// `s.replace(/[^a-zA-Z0-9_]/g, "")` — strip (not reject) anything outside
/// `[A-Za-z0-9_]`. Shared by every route that sanitizes a mart/column name
/// this way rather than rejecting it outright.
#[must_use]
pub(crate) fn strip_non_ident(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// `NUMERIC.test(type)` — `/Int|Float|Decimal/` substring match.
#[must_use]
pub(crate) fn is_numeric_type(ty: &str) -> bool {
    NUMERIC_TYPE_MARKERS.iter().any(|m| ty.contains(m))
}

/// Render a *stored* chart spec (owned strings, `lakehouse_bi::store::ChartSpec`)
/// the same way `lakehouse_bi::specs::to_render_spec` renders a built-in
/// `&'static` one: everything except `sql`, plus its origin. There is no
/// shared `to_render_spec` overload across the two `ChartSpec` types (one
/// `&'static str`-backed for compile-time specs, one owned for
/// runtime-assembled ones), so this mirrors that function's field set by
/// hand. Shared by `dashboard` and `embed`, which both render stored chart
/// lists.
#[must_use]
pub(crate) fn render_stored_spec(spec: &store::ChartSpec, source: ChartSource) -> Value {
    let mut map = Map::new();
    map.insert("id".to_owned(), json!(spec.id));
    map.insert("title".to_owned(), json!(spec.title));
    map.insert("kind".to_owned(), json!(spec.kind));
    map.insert("mart".to_owned(), json!(spec.mart));
    map.insert("x".to_owned(), json!(spec.x));
    map.insert("y".to_owned(), json!(spec.y));
    map.insert("source".to_owned(), json!(source));
    if let Some(v) = &spec.subtitle {
        map.insert("subtitle".to_owned(), json!(v));
    }
    if let Some(v) = &spec.series {
        map.insert("series".to_owned(), json!(v));
    }
    if let Some(v) = spec.format {
        map.insert("format".to_owned(), json!(v));
    }
    if let Some(v) = spec.span {
        map.insert("span".to_owned(), json!(v));
    }
    if let Some(v) = &spec.text {
        map.insert("text".to_owned(), json!(v));
    }
    if let Some(v) = &spec.caption {
        map.insert("caption".to_owned(), json!(v));
    }
    if let Some(v) = spec.target {
        map.insert("target".to_owned(), json!(v));
    }
    Value::Object(map)
}

/// `runSpec` in `dashboard/route.ts` (and its `embed`/`public` siblings): an
/// empty `sql` (text tiles) needs no query; any other failure is captured
/// per-tile rather than failing the whole response.
///
/// WS7 item D1: `sql` is rewritten through
/// `policy_engine::rewrite_sql_for_roles` — the SAME shared entry point
/// `routes::query::run` (WS7 item C2) and `gold_export::export_batch_sql`
/// (WS7 item D2) call — for `roles`/`placeholders` BEFORE it ever reaches
/// `ch.query`. A refusal (a table function, a masking/row-filter policy
/// this module cannot prove safe, an authored-but-unenforceable
/// condition, ...) is captured into this tile's own `{"error": ...}`
/// slot, exactly like a `ClickHouse` query failure already is — the
/// dashboard still renders its other tiles — but `ch.query` is never
/// called for a refused tile, so a masked/filtered chart can never
/// silently execute the unrewritten `sql` instead.
pub(crate) async fn run_spec_sql(
    ch: &ChClient,
    id: &str,
    sql: &str,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &crate::policy_engine::PolicyEngineObligations<'_>,
) -> (String, Value) {
    if sql.is_empty() {
        return (id.to_owned(), json!({ "columns": [], "rows": [] }));
    }
    let rewritten = match crate::policy_engine::rewrite_sql_for_roles(
        sql,
        &sqlparser::dialect::ClickHouseDialect {},
        roles,
        placeholders,
        obligations,
    )
    .await
    {
        Ok(rewritten) => rewritten,
        Err(err) => {
            return (
                id.to_owned(),
                json!({ "error": crate::policy_engine::enforcement_error_message(&err) }),
            );
        }
    };
    match ch.query(&rewritten, None).await {
        Ok(r) => {
            let columns: Vec<String> = r.meta.iter().map(|m| m.name.clone()).collect();
            (id.to_owned(), json!({ "columns": columns, "rows": r.data }))
        }
        Err(err) => (id.to_owned(), json!({ "error": err.to_string() })),
    }
}

/// `SELECT table, name FROM system.columns WHERE database='serving'`,
/// grouped into a mart → column-set map — used to decide which dashboard
/// filters apply to which tile.
pub(crate) async fn mart_columns(
    ch: &ChClient,
) -> Result<HashMap<String, HashSet<String>>, ChError> {
    let rows = ch
        .rows(
            "SELECT table, name FROM system.columns WHERE database='serving'",
            None,
        )
        .await?;
    let mut m: HashMap<String, HashSet<String>> = HashMap::new();
    for r in &rows {
        let table = r.get("table").and_then(Value::as_str).unwrap_or("");
        let name = r.get("name").and_then(Value::as_str).unwrap_or("");
        m.entry(table.to_owned())
            .or_default()
            .insert(name.to_owned());
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn prettify_replaces_underscores_and_titlecases() {
        assert_eq!(prettify("mart_wisman"), "Mart Wisman");
    }

    #[test]
    fn prettify_handles_single_word() {
        assert_eq!(prettify("atlas"), "Atlas");
    }

    #[test]
    fn prettify_handles_leading_and_trailing_underscores() {
        assert_eq!(prettify("_foo_bar_"), " Foo Bar ");
    }

    #[test]
    fn js_string_converts_null_to_empty_string() {
        assert_eq!(js_string(Some(&Value::Null)), "");
        assert_eq!(js_string(None), "");
    }

    #[test]
    fn js_string_passes_through_plain_string() {
        assert_eq!(js_string(Some(&Value::String("2014".to_owned()))), "2014");
    }

    #[test]
    fn js_string_stringifies_number() {
        assert_eq!(js_string(Some(&serde_json::json!(476))), "476");
    }

    #[test]
    fn num_or_zero_defaults_on_missing_row() {
        assert_eq!(num_or_zero(None, "n"), 0);
    }

    #[test]
    fn num_or_zero_defaults_on_unparseable_value() {
        let mut m = Map::new();
        m.insert("n".to_owned(), Value::String("not-a-number".to_owned()));
        assert_eq!(num_or_zero(Some(&m), "n"), 0);
    }

    #[test]
    fn num_or_zero_parses_valid_string() {
        let mut m = Map::new();
        m.insert("n".to_owned(), Value::String("42".to_owned()));
        assert_eq!(num_or_zero(Some(&m), "n"), 42);
    }

    #[test]
    fn str_col_defaults_to_empty_string() {
        let m = Map::new();
        assert_eq!(str_col(&m, "missing"), "");
    }

    #[test]
    fn js_error_formats_like_string_of_error() {
        assert_eq!(js_error("bad sql"), "Error: bad sql");
    }
}

// ── WS7 item D1: `run_spec_sql` goes through the same
//    `policy_engine::rewrite_sql_for_roles` Query Studio uses ──────────────
#[cfg(test)]
mod run_spec_sql_enforcement {
    //! Real Postgres (`#[sqlx::test]`, a real authored `policy` row) plus a
    //! wiremock `ClickHouse` — the exact same two-harness shape
    //! `routes::query`'s `enforcement` test module already established for
    //! WS7 item C2 (`run_rewrites_sql_for_a_governed_table_before_executing`).
    //! The WS7 plan names `fake_clickhouse_returning_columns`/
    //! `fake_policy_engine_obligations_masking` helpers that do not exist
    //! anywhere in this codebase — `PolicyEngineObligations` is a concrete
    //! struct over a real `PgPool`/`ChClient`, not a mockable trait, so a
    //! "fake" obligations source can only mean a real (ephemeral)
    //! Postgres seeded with a real policy row, which is what this module
    //! builds instead.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::{self, CreatePolicyInput};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::run_spec_sql;
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    /// Mounts the `system.columns` response `PolicyEngineObligations`
    /// needs to resolve `silver.customers`'s real column list, and the
    /// masked-projection query's own response — the ONLY two `ClickHouse`
    /// requests a masked `run_spec_sql` call makes.
    async fn mount_masked_customers_responses(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
            .and(body_string_contains("replaceRegexpOne"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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

    /// Failing-test-first for WS7 item D1: before `run_spec_sql` grew its
    /// `roles`/`placeholders`/`obligations` parameters and its internal
    /// `policy_engine::rewrite_sql_for_roles` call, it took only
    /// `(ch, id, sql)` and sent `sql` to `ClickHouse` completely
    /// unrewritten. Quoted failure text (captured against the pre-D1
    /// signature, `cargo test -p lakehouse-api support::run_spec_sql
    /// 2>&1 | tail -40`): `error[E0061]: this function takes 3 arguments
    /// but 6 arguments were supplied` — the call this test makes does not
    /// even compile against the old signature, let alone mask `email`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_spec_sql_is_rewritten_through_sql_rewrite_enforce_for_the_given_roles(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "dashboard-tile-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "silver.customers".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"silver.customers","mask":["email"]}"#
                        .to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");

        let server = MockServer::start().await;
        mount_masked_customers_responses(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let (_id, _val) = run_spec_sql(
            &ch,
            "chart1",
            "SELECT id, email FROM silver.customers",
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;

        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            requests
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body)
                    .contains("replaceRegexpOne(toString(`email`)")),
            "expected ClickHouse to receive the masked/rewritten query, never the raw one"
        );
        Ok(())
    }

    /// Failing-test-first for WS7 item D1's other half (Hard Requirement
    /// 2): before this task, `run_spec_sql` had no rewrite/refusal step at
    /// all, so a table-function chart SQL would have reached `ClickHouse`
    /// unrefused. Quoted failure text (same pre-D1 signature): `error[E0061]:
    /// this function takes 3 arguments but 6 arguments were supplied`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_spec_sql_refuses_and_never_executes_when_enforce_refuses(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        // No mocks mounted at all — any request that reaches ClickHouse
        // fails loudly (connection/404), which would itself prove the
        // refusal happened too late.
        let server = MockServer::start().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let (_id, val) = run_spec_sql(
            &ch,
            "chart1",
            "SELECT * FROM url('h', 'CSV')",
            &[],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;

        assert!(val.get("error").is_some(), "expected a refusal: {val:?}");
        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            requests.is_empty(),
            "a refused chart SQL must never reach ClickHouse: {requests:?}"
        );
        Ok(())
    }
}
