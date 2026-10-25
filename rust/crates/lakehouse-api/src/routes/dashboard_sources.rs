//! `GET/POST/PUT/DELETE /api/dashboard/sources`, `POST
//! /api/dashboard/sources/preview` — dashboard SQL sources: a saved `SELECT`
//! (usually a join across several `serving` marts) a chart can read instead
//! of one mart. Plan: `docs/plans/DASHBOARD-SQL-SOURCES-FOLDERS-PLAN.md` §3.
//!
//! Posture, in the order every write and preview applies it:
//!
//! 1. `sql_guard::check_sql_source` — one read-only `SELECT`/`WITH`, only
//!    `serving.<table>` references, no `;`/`SETTINGS`/`FORMAT`.
//! 2. `policy_engine::rewrite_sql_for_roles` for the caller's roles — the
//!    same pass tiles and Query Studio use, which also refuses table
//!    functions and sensitive `system.*` reads. Nothing reaches
//!    `ClickHouse` before this succeeds, not even the column probe.
//! 3. Execution with `lakehouse_bi::builder::SQL_SOURCE_SETTINGS` (2 000
//!    rows, 30 s).
//!
//! The preview can also draw a chart over the SQL before it is saved
//! (`chart` in the body): the chart is validated against the probed columns
//! and its query is built by the same code as a stored source's chart
//! (`store::spec_from_inline_sql`), then runs through the same role rewrite.
//!
//! Authoring needs `dashboard:sql` (`POLICY_TABLE`), a permission no seeded
//! role holds except `*:*` — the same starting point as `dashboard:write`.
//! Reading the list needs only `dashboard:read`. `ClickHouse` error text
//! never reaches a response (AGENTS.md principle 4).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::builder::SQL_SOURCE_SETTINGS;
use lakehouse_bi::sources::{self, SourceColumn, SqlSource};
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{self, BiError, ChartInput, InlineSql};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::policy_engine::PolicyEngineObligations;
use crate::routes::support::{SpecRunFailure, render_stored_spec, try_run_spec_sql};
use crate::sql_guard::check_sql_source;
use crate::state::AppState;
use crate::upstream_error;

/// Rows a preview returns — enough to see the shape, not a result set.
const PREVIEW_ROWS: u32 = 50;

/// Longest title accepted.
const TITLE_MAX_CHARS: usize = 200;

/// `POST`/`PUT` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    sql: String,
    #[serde(default)]
    folder_id: Option<String>,
}

/// `POST /api/dashboard/sources/preview` body.
#[derive(Debug, Deserialize)]
pub struct PreviewBody {
    #[serde(default)]
    sql: String,
    /// A chart to draw over `sql`: the input the builder sends to
    /// `POST /api/dashboard/specs/preview`, without a mart or a source id.
    #[serde(default)]
    chart: Option<ChartInput>,
}

/// `DELETE` query.
#[derive(Debug, Deserialize)]
pub struct IdQuery {
    #[serde(default)]
    id: Option<String>,
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

/// Fixed words for a dashboard SQL source that failed to run (SEC-11).
const SQL_SOURCE: upstream_error::Context = upstream_error::Context::new(
    "dashboard SQL source",
    "The SQL source failed to run.",
    "ClickHouse is unavailable.",
);

/// A `ClickHouse` failure as a fixed message plus a reference id: a
/// server-side error means the statement itself failed (422), anything else
/// that `ClickHouse` is unreachable (503). The detail is only logged.
fn classify_ch_error(err: &ChError) -> ApiError {
    upstream_error::ch_error(&SQL_SOURCE, err)
}

/// The placeholder values (`{{principal_id}}` and friends) of `principal`.
fn placeholders_for(principal: &Principal) -> crate::sql_rewrite::PlaceholderValues {
    crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

/// Steps 1 and 2 of the module posture: the lexical gate, then the policy
/// rewrite for `principal`. Returns `(normalized, rewritten)`: the text to
/// store and the text that may be executed now.
async fn checked_and_rewritten(
    state: &AppState,
    principal: &Principal,
    sql: &str,
) -> Result<(String, String), ApiError> {
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    check_and_rewrite(
        sql,
        &principal.role_names,
        &placeholders_for(principal),
        &obligations,
    )
    .await
}

/// [`checked_and_rewritten`] without the state: the single place steps 1 and
/// 2 are applied, for a save and for a preview alike.
async fn check_and_rewrite(
    sql: &str,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &PolicyEngineObligations<'_>,
) -> Result<(String, String), ApiError> {
    let normalized =
        check_sql_source(sql).map_err(|r| ApiError::Unprocessable(r.message().to_owned()))?;
    let rewritten = crate::policy_engine::rewrite_sql_for_roles(
        &normalized,
        &sqlparser::dialect::ClickHouseDialect {},
        roles,
        placeholders,
        obligations,
    )
    .await
    .map_err(|err| {
        ApiError::Unprocessable(crate::policy_engine::enforcement_error_message(&err).to_owned())
    })?;
    Ok((normalized, rewritten))
}

/// The columns `rewritten` returns, via `DESCRIBE` (no rows are read).
async fn probe_columns(ch: &ChClient, rewritten: &str) -> Result<Vec<SourceColumn>, ApiError> {
    let sql = format!("DESCRIBE (SELECT * FROM (\n{rewritten}\n))");
    let rows = ch
        .rows(&sql, None)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let columns: Vec<SourceColumn> = rows
        .iter()
        .filter_map(|r| {
            Some(SourceColumn {
                name: r.get("name")?.as_str()?.to_owned(),
                ty: r.get("type")?.as_str()?.to_owned(),
            })
        })
        .collect();
    if columns.is_empty() {
        return Err(ApiError::Unprocessable(
            "the SQL source returns no columns.".to_owned(),
        ));
    }
    Ok(columns)
}

fn clean_title(title: &str) -> Result<String, ApiError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(ApiError::BadRequest("title is required.".to_owned()));
    }
    if title.chars().count() > TITLE_MAX_CHARS {
        return Err(ApiError::BadRequest(
            "title is too long (max 200 characters).".to_owned(),
        ));
    }
    Ok(title.to_owned())
}

/// Whether `viewer` may see `source`: a source at the root is for everyone
/// entitled to dashboards, one filed in a folder is for the people who may see
/// that folder (`BI-19` B2 review S4, owner decision 2026-10-25; the SQL text is
/// the thing a restricted folder keeps back).
#[must_use]
pub(crate) fn source_visible(viewer: &lakehouse_bi::access::Viewer, source: &SqlSource) -> bool {
    source.folder_id.is_empty() || viewer.sees_folder(&source.folder_id, "")
}

/// The same `404` for a source that does not exist and one in a folder the
/// caller cannot see, so an id guess learns nothing.
fn source_not_found(id: &str) -> ApiError {
    ApiError::NotFound(format!("SQL source '{id}' not found."))
}

/// The source `id` as `viewer` may see it.
///
/// # Errors
///
/// 404 for an unknown source or one in a folder the caller cannot see.
pub(crate) async fn visible_source(
    ch: &ChClient,
    viewer: &lakehouse_bi::access::Viewer,
    id: &str,
) -> Result<SqlSource, ApiError> {
    sources::get_source(ch, id)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .filter(|s| source_visible(viewer, s))
        .ok_or_else(|| source_not_found(id))
}

/// `GET /api/dashboard/sources` — every live source the caller may see.
///
/// # Errors
///
/// 422/503 on a `ClickHouse` failure (fixed text).
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    // BI-19 B2 (plan D8): the chart builder lists sources for somebody who can
    // edit a dashboard through a rule alone; the router asks only "signed in".
    // `viewer_for` refuses somebody with neither `dashboard:read` nor a rule.
    // BI-19 B2 review S4: and a source in a restricted folder is not listed.
    let viewer = crate::routes::dashboard_access::viewer_for(&state, &principal).await?;
    let list: Vec<SqlSource> = sources::list_sources(&state.clickhouse)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .into_iter()
        .filter(|s| source_visible(&viewer, s))
        .collect();
    Ok(ApiJson(json!({ "sources": list })))
}

/// Shared body of create and update.
async fn save(
    state: &AppState,
    principal: &Principal,
    body: SourceBody,
    id: String,
) -> Result<SqlSource, ApiError> {
    let title = clean_title(&body.title)?;
    let folder_id = body.folder_id.unwrap_or_default().trim().to_owned();
    crate::routes::dashboard_folders::ensure_folder_exists(&state.clickhouse, &folder_id).await?;
    // BI-19 B2 review S4: nor can a source be filed in a folder the caller
    // cannot see (the same 400 as for an unknown folder, so no id is confirmed).
    let viewer = crate::routes::dashboard_access::viewer_for_write(state, principal).await?;
    if !folder_id.is_empty() && !viewer.sees_folder(&folder_id, "") {
        return Err(ApiError::BadRequest("folder not found.".to_owned()));
    }
    let (normalized, rewritten) = checked_and_rewritten(state, principal, &body.sql).await?;
    let columns = probe_columns(&state.clickhouse, &rewritten).await?;
    let source = SqlSource {
        id,
        title,
        sql: normalized,
        columns,
        folder_id,
        created_by: principal.id.uuid().to_string(),
        updated_at: None,
    };
    sources::save_source(&state.clickhouse, &source)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    // DATA-13 (F4): the output columns are known from the probe above, so the
    // source's column lineage is recorded now; a failure there is logged and
    // never fails the save.
    super::lineage::columns::record_sql_source(state, &source).await;
    Ok(source)
}

/// `POST /api/dashboard/sources` — create a source.
///
/// # Errors
///
/// 400 on a bad body/title, 422 when the SQL is refused or fails to run,
/// 503 when `ClickHouse` is unreachable.
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: SourceBody = parse(&body)?;
    let source = save(&state, &principal, body, sources::new_source_id()).await?;
    Ok(ApiJson(json!({ "ok": true, "source": source })))
}

/// `PUT /api/dashboard/sources` — replace a source's title/SQL/folder,
/// keeping its id. Charts built on it pick up the new SQL on their next
/// render (`builder::sql_for_sql_source`).
///
/// # Errors
///
/// As [`create`], plus 400 without an id and 404 for an unknown one.
pub async fn update(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: SourceBody = parse(&body)?;
    let id = body
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?
        .to_owned();
    // BI-19 B2 review S4: a source in a folder the caller cannot see is as
    // unknown to them as one that does not exist, for a change as for a read.
    let viewer = crate::routes::dashboard_access::viewer_for_write(&state, &principal).await?;
    visible_source(&state.clickhouse, &viewer, &id).await?;
    let source = save(&state, &principal, body, id).await?;
    Ok(ApiJson(json!({ "ok": true, "source": source })))
}

/// `DELETE /api/dashboard/sources?id=` — tombstone a source nobody uses.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one, 409 (naming the charts) while
/// any chart still reads it, 422/503 on a `ClickHouse` failure.
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let id =
        q.id.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?
            .to_owned();
    let ch = &state.clickhouse;
    // BI-19 B2 review S4: as in `update`.
    let viewer = crate::routes::dashboard_access::viewer_for_write(&state, &principal).await?;
    visible_source(ch, &viewer, &id).await?;
    // BI-18 part C, part 2: a chart on somebody's personal board still blocks
    // the delete (it would break), but it is not named: the caller is told
    // only about the charts they may see.
    let all = store::list_stored_charts(ch, &lakehouse_bi::access::Viewer::system())
        .await
        .map_err(|err| classify_ch_error(&err))?;
    if !sources::charts_using_source(&all, &id).is_empty() {
        // BI-19 B1 review R3: the resolved viewer, not the baseline.
        let viewer = crate::routes::dashboard_access::usage_viewer_of(&state, &principal).await?;
        let mine = store::list_stored_charts(ch, &viewer)
            .await
            .map_err(|err| classify_ch_error(&err))?;
        let users = sources::charts_using_source(&mine, &id);
        let message = if users.is_empty() {
            "SQL source is still used by a dashboard you cannot see.".to_owned()
        } else {
            format!(
                "SQL source is still used by {} chart(s): {}",
                users.len(),
                users.join(", ")
            )
        };
        return Err(ApiError::Conflict(message).into());
    }
    sources::delete_source(ch, &id)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    // DATA-13 (F4): the source's column mappings go with it.
    super::lineage::columns::forget_sql_source(&state, &id).await;
    // BI-8: the source's calculated fields go with it (no chart uses the
    // source any more, so none can use them).
    let orphans =
        lakehouse_bi::fields::list_fields_for(ch, lakehouse_bi::fields::SourceKind::SqlSource, &id)
            .await
            .map_err(|err| classify_ch_error(&err))?;
    for field in orphans {
        lakehouse_bi::fields::delete_field(ch, &field.id)
            .await
            .map_err(|err| classify_ch_error(&err))?;
    }
    Ok(ApiJson(json!({ "ok": true })))
}

/// `POST /api/dashboard/sources/preview` — validate, rewrite and run a SQL
/// source without saving it; returns its columns and the first
/// [`PREVIEW_ROWS`] rows. With `chart` in the body the response also carries
/// `chart`: `{ spec, result }`, the shape `POST /api/dashboard/specs/preview`
/// answers with, drawn over this SQL instead of a stored source.
///
/// # Errors
///
/// 400 on a bad body or a chart that does not fit the SQL's columns, 422 when
/// the SQL is refused or fails, 503 when `ClickHouse` is unreachable.
pub async fn preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: PreviewBody = parse(&body)?;
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    let time = crate::routes::settings::time_context(&state).await?;
    let out = preview_for_roles(
        &state.clickhouse,
        &obligations,
        &principal.role_names,
        &placeholders_for(&principal),
        body,
        &time,
    )
    .await?;
    Ok(ApiJson(out))
}

/// The body of [`preview`], in the order the module posture fixes: guard and
/// rewrite, then the column probe, then (only for a chart) validation against
/// those columns, and only then any row is read.
///
/// # Errors
///
/// As [`preview`].
async fn preview_for_roles(
    ch: &ChClient,
    obligations: &PolicyEngineObligations<'_>,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    body: PreviewBody,
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<Value, ApiError> {
    let (normalized, rewritten) =
        check_and_rewrite(&body.sql, roles, placeholders, obligations).await?;
    let columns = probe_columns(ch, &rewritten).await?;
    let chart = match &body.chart {
        Some(input) => Some(
            store::spec_from_inline_sql(
                input,
                InlineSql {
                    sql: &normalized,
                    columns: &columns,
                },
                ChartSource::Ui,
                "ui",
                time,
            )
            .map_err(|err| match err {
                BiError::Validation(message) => ApiError::BadRequest(message),
                BiError::Clickhouse(err) => classify_ch_error(&err),
            })?,
        ),
        None => None,
    };
    let sql =
        format!("SELECT * FROM (\n{rewritten}\n) AS src LIMIT {PREVIEW_ROWS}{SQL_SOURCE_SETTINGS}");
    let result = ch
        .query(&sql, None)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let mut out = json!({
        "columns": columns,
        "rows": result.data,
    });
    if let Some(chart) = chart {
        // The chart's own query embeds the normalized SQL and is rewritten
        // for the same roles again, exactly as a stored source's chart is.
        let mut rows =
            match try_run_spec_sql(ch, &chart.spec.sql, roles, placeholders, obligations).await {
                Ok(rows) => rows,
                Err(SpecRunFailure::Refused(message)) => json!({ "error": message }),
                Err(SpecRunFailure::Clickhouse(err)) => return Err(classify_ch_error(&err)),
            };
        crate::routes::support::annotate_saved_grain(&mut rows, &chart);
        crate::routes::support::annotate_saved_table(&mut rows, &chart);
        let shown = crate::routes::support::preview_rows(
            ch,
            &chart,
            &rows,
            (roles, placeholders, obligations),
        )
        .await;
        out["chart"] = json!({
            "spec": render_stored_spec(&chart.spec, ChartSource::Ui),
            "result": rows,
            "previewRows": shown,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod preview_tests {
    //! `preview_for_roles` against a wiremock `ClickHouse`: the order the
    //! module posture fixes (guard, rewrite, probe, chart validation, rows),
    //! the unchanged no-`chart` answer, and fixed error text. No Postgres:
    //! `PolicyEngineObligations::new(None, ..)` means "no policies", which
    //! leaves the rewrite itself in place.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::sql_rewrite::PlaceholderValues;

    const SQL: &str = "SELECT place, lat, lon, visitors FROM serving.mart_x";

    fn describe_body() -> serde_json::Value {
        json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [
                {"name": "place", "type": "String"},
                {"name": "lat", "type": "Float64"},
                {"name": "lon", "type": "Float64"},
                {"name": "visitors", "type": "UInt64"},
            ],
            "rows": 4,
        })
    }

    fn rows_body(rows: &[serde_json::Value]) -> serde_json::Value {
        json!({
            "meta": [{"name": "place", "type": "String"}, {"name": "visitors", "type": "UInt64"}],
            "data": rows,
            "rows": rows.len(),
        })
    }

    /// DESCRIBE, the 50-row preview and the chart query, each answered by
    /// what its SQL contains.
    async fn mount(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(describe_body()))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("AS src LIMIT 50"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(rows_body(&[json!({"place": "Bali", "visitors": 7})])),
            )
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("GROUP BY"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(rows_body(&[json!({"place": "Bali", "visitors": 7})])),
            )
            .mount(server)
            .await;
    }

    async fn run(
        server: &MockServer,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(None, &ch);
        let body: PreviewBody = serde_json::from_value(body).unwrap();
        preview_for_roles(
            &ch,
            &obligations,
            &[],
            &PlaceholderValues::none(),
            body,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
    }

    async fn sent(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn chart(extra: &serde_json::Value) -> serde_json::Value {
        let mut base = json!({
            "title": "Visitors", "kind": "hbar", "dimension": "place", "measures": ["visitors"],
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        base
    }

    #[tokio::test]
    async fn a_preview_without_a_chart_answers_columns_and_rows_only() {
        let server = MockServer::start().await;
        mount(&server).await;

        let out = run(&server, json!({ "sql": SQL })).await.unwrap();

        assert_eq!(out["columns"].as_array().unwrap().len(), 4);
        assert_eq!(out["rows"].as_array().unwrap().len(), 1);
        assert!(out.get("chart").is_none(), "{out}");
        assert!(
            sent(&server).await.iter().all(|b| !b.contains("GROUP BY")),
            "no chart query without a chart"
        );
    }

    #[tokio::test]
    async fn a_chart_over_unsaved_sql_answers_the_spec_and_rows_a_stored_chart_would() {
        let server = MockServer::start().await;
        mount(&server).await;

        let out = run(&server, json!({ "sql": SQL, "chart": chart(&json!({})) }))
            .await
            .unwrap();

        // The same `{ spec, result }` shape `POST /api/dashboard/specs/preview` answers.
        let spec = &out["chart"]["spec"];
        assert_eq!(spec["kind"], "hbar");
        assert_eq!(spec["x"], "place");
        assert_eq!(spec["y"], "visitors");
        assert_eq!(spec["sqlSource"], "unsaved");
        assert_eq!(out["chart"]["result"]["rows"][0]["place"], "Bali");
        assert_eq!(out["chart"]["result"]["columns"][0], "place");
        let chart_queries: Vec<String> = sent(&server)
            .await
            .into_iter()
            .filter(|b| b.contains("GROUP BY"))
            .collect();
        assert_eq!(chart_queries.len(), 1);
        assert!(
            chart_queries[0].contains("serving.mart_x"),
            "{chart_queries:?}"
        );
        assert!(chart_queries[0].contains(") AS src"), "{chart_queries:?}");
    }

    #[tokio::test]
    async fn a_point_map_over_unsaved_sql_previews() {
        let server = MockServer::start().await;
        mount(&server).await;

        let out = run(
            &server,
            json!({ "sql": SQL, "chart": chart(&json!({
                "kind": "pointmap", "lat": "lat", "lon": "lon",
            })) }),
        )
        .await
        .unwrap();

        assert_eq!(out["chart"]["spec"]["kind"], "pointmap");
        assert_eq!(out["chart"]["spec"]["lat"], "lat");
        assert_eq!(out["chart"]["spec"]["lon"], "lon");
    }

    #[tokio::test]
    async fn a_chart_naming_a_column_the_sql_does_not_return_is_a_400_with_our_message() {
        let server = MockServer::start().await;
        mount(&server).await;

        let err = run(
            &server,
            json!({ "sql": SQL, "chart": chart(&json!({ "measures": ["not_returned"] })) }),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, ApiError::BadRequest(ref m) if m == "invalid or missing measure column."),
            "{err:?}"
        );
        // Refused after the probe and before a single row was read.
        let bodies = sent(&server).await;
        assert!(bodies.iter().any(|b| b.contains("DESCRIBE")));
        assert!(
            bodies
                .iter()
                .all(|b| !b.contains("AS src LIMIT 50") && !b.contains("GROUP BY"))
        );
    }

    #[tokio::test]
    async fn the_guard_refuses_a_statement_that_is_not_a_select_even_with_a_chart() {
        let server = MockServer::start().await;
        mount(&server).await;

        for sql in [
            "DROP TABLE serving.mart_x",
            "SELECT 1 FROM system.users",
            "SELECT a FROM serving.mart_x; SELECT 1",
        ] {
            let err = run(&server, json!({ "sql": sql, "chart": chart(&json!({})) }))
                .await
                .unwrap_err();
            assert!(matches!(err, ApiError::Unprocessable(_)), "{sql}: {err:?}");
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing may reach ClickHouse before the guard accepts the SQL"
        );
    }

    #[tokio::test]
    async fn clickhouse_text_from_the_chart_query_never_reaches_the_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(describe_body()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("AS src LIMIT 50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rows_body(&[])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("GROUP BY"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("Code: 60. DB::Exception: upstream-secret-detail"),
            )
            .mount(&server)
            .await;

        let err = run(&server, json!({ "sql": SQL, "chart": chart(&json!({})) }))
            .await
            .unwrap_err();

        assert!(
            !format!("{err:?}").contains("upstream-secret-detail"),
            "{err:?}"
        );
        assert!(
            // SEC-11: the fixed message now carries a reference id after it.
            matches!(err, ApiError::Unprocessable(ref m) if m.starts_with("The SQL source failed to run. Reference: ")),
            "{err:?}"
        );
    }
}
