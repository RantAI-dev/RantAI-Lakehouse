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

// `pub(in crate::routes)` so `routes::lineage`, which sits outside
// `routes::ai`, runs this SAME union query rather than re-deriving an
// equivalent string that could drift from this one.
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
    if !is_read_only_sql(&sql) {
        return json!({ "error": "Only a read-only SELECT is allowed." });
    }
    // Same fail-closed shape `tools::queries::run_saved_query` uses for a
    // headless (schedule/service-token) caller with no interactive
    // principal to run as — `routes::query::run` 401s with none, for
    // every engine, so there is no honest value to forward here either.
    // Checked BEFORE `dry_run_sql` below (a real `ClickHouse` round trip)
    // so a headless caller is refused without spending that call at all.
    let Some(principal) = principal else {
        return json!({
            "error": "running SQL needs a signed-in user; this call came from a schedule, \
                       which has no user to run as",
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
    let result = api_result_to_value(
        crate::routes::query::run(axum::extract::State(state.clone()), extension, body).await,
    )
    .await;
    compact_query_result(result)
}

/// Rows a `run_sql` result hands the model.
const MAX_RESULT_ROWS: usize = 100;

/// `routes::query::run`'s response, reduced to what the model needs.
///
/// The full response carries an id, engine metrics and a plan the model
/// never uses, and a 700-row result was cut at 8,000 characters in the
/// middle of a JSON object, so the model read a broken fragment as data.
/// Now the rows are capped at [`MAX_RESULT_ROWS`] with an explicit note
/// saying how many more there were and what to do instead, and an error
/// keeps its message but loses the engine boilerplate
/// ([`clean_sql_error`]).
fn compact_query_result(result: Value) -> Value {
    let Value::Object(mut body) = result else {
        return result;
    };
    if let Some(Value::String(err)) = body.get("error") {
        return json!({
            "error": clean_sql_error(err),
            "hint": "Check table and column names against the DATA MAP and fix the query.",
        });
    }
    let rows = match body.remove("rows") {
        Some(Value::Array(rows)) => rows,
        _ => Vec::new(),
    };
    let total = rows.len();
    let shown: Vec<Value> = rows.into_iter().take(MAX_RESULT_ROWS).collect();
    let mut out = Map::new();
    out.insert(
        "columns".to_owned(),
        body.remove("columns").unwrap_or(Value::Null),
    );
    out.insert("rows".to_owned(), Value::Array(shown));
    out.insert("rowCount".to_owned(), json!(total));
    if total > MAX_RESULT_ROWS {
        out.insert(
            "note".to_owned(),
            json!(format!(
                "This display is cut off: only the first {MAX_RESULT_ROWS} of the {total} rows \
                 the query returned are shown. The data itself is complete; do not describe it \
                 as partial or missing. To report totals, rankings or per-period figures, run a \
                 new query that aggregates in SQL (GROUP BY, ORDER BY … LIMIT) instead of \
                 adding up these rows."
            )),
        );
    }
    if body.get("truncated") == Some(&Value::Bool(true)) {
        out.insert(
            "truncated".to_owned(),
            json!("the engine's row limit cut this result; aggregate in SQL"),
        );
    }
    Value::Object(out)
}

/// A `ClickHouse` error for the query's own author: the message and its
/// error name, without the `Code: N. DB::Exception:` prefix, the server
/// version suffix, or any stack trace, capped in length. The model needs
/// the message to fix its SQL; it never needs the rest.
pub(super) fn clean_sql_error(err: &str) -> String {
    let first_line = err.lines().next().unwrap_or("");
    let body = first_line
        .find("DB::Exception:")
        .map_or(first_line, |i| &first_line[i + "DB::Exception:".len()..])
        .trim();
    let body = body.find(" (version ").map_or(body, |i| &body[..i]).trim();
    body.chars().take(400).collect()
}

/// `/^\s*(with|select)\b/i.test(sql) && !/\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)`
/// — the cheap first filter [`run_sql`] applies to the model's SQL. Distinct
/// from `query/run`'s guard (`routes::query::is_read_only`): only
/// `with`/`select` are allowed here (no `show`/`describe`/`explain`).
fn is_read_only_sql(sql: &str) -> bool {
    starts_with_with_or_select(sql) && !contains_denied_keyword_full(sql)
}

fn starts_with_with_or_select(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    ["with", "select"].iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    })
}

fn contains_denied_keyword_full(sql: &str) -> bool {
    const DENIED: [&str; 12] = [
        "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename", "attach",
        "detach", "grant", "revoke",
    ];
    word_occurs_any(sql, &DENIED)
}

fn word_occurs_any(sql: &str, denied: &[&str]) -> bool {
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    denied.iter().any(|kw| word_occurs(&chars, kw))
}

fn word_occurs(chars: &[char], word: &str) -> bool {
    let word_chars: Vec<char> = word.chars().collect();
    let n = word_chars.len();
    if n == 0 || chars.len() < n {
        return false;
    }
    for start in 0..=(chars.len() - n) {
        if chars[start..start + n] == word_chars[..] {
            let before_ok = start == 0 || !is_word_char(chars[start - 1]);
            let after_ok = start + n == chars.len() || !is_word_char(chars[start + n]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
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
/// [`is_read_only_sql`]'s regex-based guard
/// (unchanged, still checked first by [`run_sql`] itself) — never a
/// replacement for it: a third-party parser's dialect (or a hand-rolled
/// regex) can disagree with what `ClickHouse` will actually execute, so
/// this asks the engine itself rather than guessing. Returns
/// `{"error": "..."}` (naming the REAL reported statement kind, never
/// echoing the user's raw SQL back, per `AGENTS.md` rule 4) when refused,
/// or `{}` when the statement is confirmed a `SELECT`.
pub(super) async fn dry_run_sql(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let sql = arg_str(args, "sql");
    // `raw_bytes`, not `rows`: `ChClient::rows` appends `\nFORMAT JSON`,
    // and `EXPLAIN AST … FORMAT JSON` does NOT answer in JSON —
    // `ClickHouse` parses the FORMAT clause as part of the statement being
    // explained (it appears in the dumped AST as `Identifier JSON`) and
    // still replies in the default text format. The JSON envelope
    // `rows` expects was therefore never there: the row set came back
    // EMPTY, `first_line` was "", and EVERY statement — `SELECT` included
    // — was refused as "melaporkan unknown", so the `run_sql` tool could
    // not succeed at all. Verified live against `ClickHouse` 26.7.
    // `raw_bytes` sends the statement verbatim and hands back the text
    // body `EXPLAIN` actually produces.
    let body = match ch.raw_bytes(&format!("EXPLAIN AST {sql}"), None).await {
        Ok(b) => b,
        Err(err) => {
            return json!({ "error": format!("the query could not be parsed: {}", clean_sql_error(&err.to_string())) });
        }
    };
    let text = String::from_utf8_lossy(&body);
    let first_line = text.lines().next().unwrap_or("").trim().to_owned();
    if first_line.starts_with("SelectWithUnionQuery") {
        return json!({});
    }
    let kind = first_line.split_whitespace().next().unwrap_or("unknown");
    json!({ "error": format!("only SELECT is allowed (EXPLAIN AST reported {kind})") })
}

/// `primer`/`sekunder` names where a dataset comes from, not a layer. The
/// tools used to hand the raw `tier` word to the model, which then
/// described the lakehouse as "primer" and "sekunder" data instead of
/// Bronze/Silver/Gold; they now say what it means.
fn source_kind(tier: &str) -> &'static str {
    match tier {
        "primer" => "primary source",
        "sekunder" => "secondary source",
        _ => "not recorded",
    }
}

/// `"primary"`/`"secondary"` (or the stored words) to the stored tier.
fn tier_filter(raw: &str) -> &str {
    match raw {
        "primary" | "primary source" | "primer" => "primer",
        "secondary" | "secondary source" | "sekunder" => "sekunder",
        _ => "",
    }
}

/// Row counts of every table in `serving` and `silver`, keyed
/// `"db.table"`. `None` for a view (no stored row count).
async fn table_rows(
    ch: &ChClient,
) -> Result<std::collections::HashMap<String, Option<u64>>, String> {
    let rows = ch
        .rows(
            "SELECT database, name, toString(total_rows) AS n FROM system.tables \
             WHERE database IN ('serving', 'silver')",
            None,
        )
        .await
        .map_err(|_| "the table list could not be read from ClickHouse".to_owned())?;
    Ok(rows
        .iter()
        .map(|r| {
            let db = r.get("database").and_then(Value::as_str).unwrap_or("");
            let name = r.get("name").and_then(Value::as_str).unwrap_or("");
            let n = r
                .get("n")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok());
            (format!("{db}.{name}"), n)
        })
        .collect())
}

/// Which layers a dataset served from Gold table `table` is present in.
/// Bronze is where every registered dataset starts (the catalog record
/// itself). Gold is `serving.<table>`. Silver has no recorded link to a
/// dataset, so it is reported only when a Silver table of the same name
/// exists; otherwise `present: false` with no row count, never a made-up
/// zero (the old `describe_dataset` counted `silver.<table>`, found
/// nothing, and reported `rows: 0` for data that was really there).
fn layers_for(
    slug: &str,
    table: &str,
    counts: &std::collections::HashMap<String, Option<u64>>,
) -> Value {
    let layer = |db: &str| {
        let key = format!("{db}.{table}");
        match counts.get(&key) {
            Some(rows) => json!({ "table": key, "present": true, "rows": rows }),
            None => json!({ "present": false }),
        }
    };
    json!({
        "bronze": { "registered": true, "dataset": slug },
        "silver": layer("silver"),
        "gold": layer("serving"),
    })
}

pub(super) async fn list_datasets(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let Ok(rows) = ch
        .rows(
            &format!("SELECT slug, title, tier, table_name FROM {CATALOG_UNION} LIMIT 500"),
            None,
        )
        .await
    else {
        return json!({ "error": "the dataset catalog could not be read (the catalog registry is not available)" });
    };
    let term = arg_str(args, "search").to_lowercase();
    let wanted = tier_filter(&{
        let source = arg_str(args, "source");
        if source.is_empty() {
            arg_str(args, "tier")
        } else {
            source
        }
    })
    .to_owned();
    let hits: Vec<Value> = rows
        .iter()
        .filter(|r| {
            let row_tier = r.get("tier").and_then(Value::as_str).unwrap_or("");
            if !wanted.is_empty() && row_tier != wanted {
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
        .map(|r| {
            let table = r.get("table_name").and_then(Value::as_str).unwrap_or("");
            json!({
                "slug": r.get("slug"),
                "title": r.get("title"),
                "sourceKind": source_kind(r.get("tier").and_then(Value::as_str).unwrap_or("")),
                "goldTable": if table.is_empty() { Value::Null } else { json!(format!("serving.{table}")) },
            })
        })
        .collect();
    json!({ "total": hits.len(), "datasets": hits })
}

pub(super) async fn describe_dataset(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let slug_raw = arg_str(args, "slug");
    let slug = SqlLiteral::from(slug_raw.as_str());
    let Ok(meta_rows) = ch
        .rows(
            &format!(
                "SELECT title, description, table_name, tier FROM {CATALOG_UNION} WHERE slug={slug} LIMIT 1"
            ),
            None,
        )
        .await
    else {
        return json!({ "error": "the dataset catalog could not be read" });
    };
    let Some(meta) = meta_rows.first() else {
        return json!({ "error": format!("no dataset with slug '{slug_raw}'; call list_datasets for the slugs") });
    };
    let table = meta
        .get("table_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let cols = ch
        .rows(
            &format!(
                "SELECT key_asli AS name, tipe AS type, deskripsi AS description FROM lake.`bronze_meta.dataset_column` WHERE slug={slug} \
                 UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta_sec.dataset_column` WHERE slug={slug}"
            ),
            None,
        )
        .await
        .unwrap_or_default();
    let counts = table_rows(ch).await.unwrap_or_default();
    json!({
        "slug": slug_raw,
        "title": meta.get("title"),
        "description": meta.get("description"),
        "sourceKind": source_kind(meta.get("tier").and_then(Value::as_str).unwrap_or("")),
        "layers": layers_for(&slug_raw, &table, &counts),
        "columns": cols,
    })
}

/// One view of the whole lakehouse by layer: the registered Bronze
/// datasets, and every Silver and Gold table with its row count. The
/// question "what data do we have?" is answered from this, in layer terms,
/// instead of from a dataset list that only knows source kinds.
pub(super) async fn lakehouse_overview(state: &AppState, principal: Option<&Principal>) -> Value {
    // The same shared-catalog rule the catalog route and the DATA MAP
    // apply: this tool lists the shared catalog and every table in it.
    let Some(principal) = principal else {
        return json!({ "error": "the lakehouse overview needs a signed-in user" });
    };
    match crate::routes::catalog::catalog_tenant_refusal(
        state,
        principal,
        &axum::http::HeaderMap::new(),
    )
    .await
    {
        Ok(None) => {}
        Ok(Some(reason)) => return json!({ "supported": false, "reason": reason }),
        Err(_) => return json!({ "error": "the shared-catalog rule could not be evaluated" }),
    }
    let ch = &state.clickhouse;
    let counts = match table_rows(ch).await {
        Ok(c) => c,
        Err(err) => return json!({ "error": err }),
    };
    let datasets = ch
        .rows(
            &format!(
                "SELECT slug, title, tier, table_name FROM {CATALOG_UNION} ORDER BY slug LIMIT 500"
            ),
            None,
        )
        .await;
    let (bronze, catalog_note) = match &datasets {
        Ok(rows) => (
            rows.iter()
                .map(|r| {
                    let table = r.get("table_name").and_then(Value::as_str).unwrap_or("");
                    json!({
                        "slug": r.get("slug"),
                        "title": r.get("title"),
                        "sourceKind": source_kind(r.get("tier").and_then(Value::as_str).unwrap_or("")),
                        "servedFrom": format!("serving.{table}"),
                    })
                })
                .collect::<Vec<_>>(),
            Value::Null,
        ),
        Err(_) => (Vec::new(), json!("the dataset catalog could not be read, so Bronze datasets are not listed")),
    };
    let mut tables: Vec<(&String, &Option<u64>)> = counts.iter().collect();
    tables.sort();
    let layer = |db: &str| -> Vec<Value> {
        tables
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{db}.")))
            .map(|(k, rows)| json!({ "table": k, "rows": rows }))
            .collect()
    };
    json!({
        "bronze": { "description": "raw data landed from source systems (Iceberg)", "datasets": bronze, "note": catalog_note },
        "silver": { "description": "cleaned, typed detail tables", "tables": layer("silver") },
        "gold": { "description": "aggregated serving marts", "tables": layer("serving") },
    })
}

/// The recorded lineage around a dataset or table: the same graph
/// `GET /api/governance/lineage` serves (`routes::lineage`), so the
/// copilot and the Lineage page can never disagree. `chain` lists every
/// edge as one line for the model; `nodes`/`edges` carry the graph.
pub(super) async fn get_lineage(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let focus = {
        let slug = arg_str(args, "slug");
        if slug.is_empty() {
            arg_str(args, "focus")
        } else {
            slug
        }
    };
    if focus.is_empty() {
        return json!({ "error": "slug is required: a dataset slug or a table name such as serving.<table>" });
    }
    let Some(principal) = principal else {
        return json!({ "error": "lineage needs a signed-in user" });
    };
    let Ok(graph) =
        crate::routes::lineage::build(state, principal, &axum::http::HeaderMap::new(), &focus)
            .await
    else {
        return json!({ "error": "lineage could not be built for this user" });
    };
    let labels: std::collections::HashMap<&str, &str> = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("label")?.as_str()?)))
        .collect();
    let chain: Vec<String> = graph["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let from = e.get("from")?.as_str()?;
            let to = e.get("to")?.as_str()?;
            Some(format!(
                "{} -[{}]-> {}",
                labels.get(from).unwrap_or(&from),
                e.get("kind")?.as_str()?,
                labels.get(to).unwrap_or(&to)
            ))
        })
        .collect();
    let mut out = graph.clone();
    out["chain"] = if chain.is_empty() {
        json!(
            graph
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or("no recorded lineage")
        )
    } else {
        json!(chain.join("\n"))
    };
    out
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
        // Measured on the local stack: the results table simply does not
        // exist until a quality run writes it, and the raw
        // `UNKNOWN_DATABASE` text went straight into the answer. Say what
        // it means instead (AGENTS.md principle 4).
        Err(err) if err.to_string().contains("UNKNOWN_DATABASE") || err.to_string().contains("UNKNOWN_TABLE") => json!({
            "supported": false,
            "reason": "no data quality checks have been run yet: there are no quality results to report",
            "hint": "list_quality_rules shows the quality rules that are defined",
        }),
        Err(_) => json!({ "error": "the quality results could not be read from ClickHouse" }),
    }
}

pub(super) async fn describe_mart(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let mart = strip_non_ident(&arg_str(args, "mart"));
    if mart.is_empty() {
        let Ok(rows) = ch
            .rows(
                "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
                 WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
                None,
            )
            .await
        else {
            return json!({ "error": "the Gold mart list could not be read from ClickHouse" });
        };
        let marts: Vec<Value> = rows
            .iter()
            .map(|r| {
                // A view has no stored row count: report it as unknown,
                // never as an empty table.
                let rows_n: Option<i64> = r
                    .get("total_rows")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok());
                json!({ "mart": r.get("name"), "rows": rows_n })
            })
            .collect();
        return json!({ "marts": marts });
    }
    let Ok(cols) = ch
        .rows(
            &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{mart}' ORDER BY position"),
            None,
        )
        .await
    else {
        return json!({ "error": "the mart's columns could not be read from ClickHouse" });
    };
    if cols.is_empty() {
        return json!({ "error": format!("no Gold mart named '{mart}' in serving; call describe_mart with no argument for the list") });
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
    /// ever issues the one `EXPLAIN AST` query) with `explain_text` as a
    /// PLAIN TEXT body — the shape `EXPLAIN` really returns. These tests
    /// used to mock a `{"data": [{"explain": …}]}` JSON envelope, which
    /// `EXPLAIN AST` never produces (see [`dry_run_sql`]'s comment): they
    /// passed while the real tool refused every statement. Returns the
    /// [`MockServer`] too — it must outlive the [`ChClient`] built from
    /// its URI, or the mock stops answering before the test runs.
    async fn fake_clickhouse_explain_ast_response(explain_text: &str) -> (MockServer, ChClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(explain_text))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        (server, ch)
    }

    /// The regression this pair of tests missed: an EMPTY body (what the
    /// old `FORMAT JSON` path effectively produced) must not be read as a
    /// `SELECT`, and a real `SELECT`'s text dump must be accepted.
    #[tokio::test]
    async fn run_sql_dry_run_refuses_an_empty_explain_body() {
        let (_server, ch) = fake_clickhouse_explain_ast_response("").await;
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT 1"));
        assert_eq!(
            dry_run_sql(&ch, &args).await,
            json!({"error": "only SELECT is allowed (EXPLAIN AST reported unknown)"})
        );
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
            json!({"error": "only SELECT is allowed (EXPLAIN AST reported InsertQuery)"})
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
        // Plain text, not a JSON envelope: `EXPLAIN AST` answers in
        // `ClickHouse`'s default text format even when a `FORMAT JSON`
        // clause is appended (see `dry_run_sql`), and this mock has to
        // answer the way the real engine does or the test proves nothing.
        Mock::given(method("POST"))
            .and(body_string_contains("EXPLAIN AST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("SelectWithUnionQuery (children 1)\n"),
            )
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
            .and(body_string_contains("replaceRegexpOne"))
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
                .any(|r| String::from_utf8_lossy(&r.body).contains("replaceRegexpOne")),
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
                "error": "running SQL needs a signed-in user; this call came from a schedule, \
                           which has no user to run as",
            }),
            "expected the honest no-principal refusal, got {result}"
        );
    }
}

#[cfg(test)]
mod read_only_guard {
    use super::*;

    #[test]
    fn is_read_only_sql_allows_with_and_select_only() {
        assert!(is_read_only_sql("SELECT 1"));
        assert!(is_read_only_sql("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(!is_read_only_sql("SHOW TABLES"));
        assert!(!is_read_only_sql("EXPLAIN SELECT 1"));
    }

    #[test]
    fn is_read_only_sql_rejects_full_dml_list() {
        for kw in [
            "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename",
            "attach", "detach", "grant", "revoke",
        ] {
            assert!(!is_read_only_sql(&format!("SELECT 1; {kw} x")));
        }
    }
}
