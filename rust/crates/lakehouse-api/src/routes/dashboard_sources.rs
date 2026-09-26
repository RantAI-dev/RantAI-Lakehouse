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
use lakehouse_bi::store;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::policy_engine::PolicyEngineObligations;
use crate::sql_guard::check_sql_source;
use crate::state::AppState;

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

/// A `ClickHouse` failure as a fixed message: a server-side error means the
/// statement itself failed (422), anything else that `ClickHouse` is
/// unreachable (503). The detail is only logged.
fn classify_ch_error(err: &ChError) -> ApiError {
    match err {
        ChError::Server(_) => {
            tracing::warn!(%err, "dashboard SQL source failed to run");
            ApiError::Unprocessable("the SQL source failed to run".to_owned())
        }
        ChError::Transport(_) | ChError::Cancelled => {
            tracing::warn!(%err, "clickhouse unreachable for a dashboard SQL source");
            ApiError::Unavailable("clickhouse unavailable".to_owned())
        }
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
    let normalized =
        check_sql_source(sql).map_err(|r| ApiError::Unprocessable(r.message().to_owned()))?;
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    let rewritten = crate::policy_engine::rewrite_sql_for_roles(
        &normalized,
        &sqlparser::dialect::ClickHouseDialect {},
        &principal.role_names,
        &placeholders,
        &obligations,
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

/// `GET /api/dashboard/sources` — every live source.
///
/// # Errors
///
/// 422/503 on a `ClickHouse` failure (fixed text).
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let list = sources::list_sources(&state.clickhouse)
        .await
        .map_err(|err| classify_ch_error(&err))?;
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
    let (normalized, rewritten) = checked_and_rewritten(state, principal, &body.sql).await?;
    let columns = probe_columns(&state.clickhouse, &rewritten).await?;
    let source = SqlSource {
        id,
        title,
        sql: normalized,
        columns,
        folder_id: body.folder_id.unwrap_or_default().trim().to_owned(),
        created_by: principal.id.uuid().to_string(),
        updated_at: None,
    };
    sources::save_source(&state.clickhouse, &source)
        .await
        .map_err(|err| classify_ch_error(&err))?;
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
    let existing = sources::get_source(&state.clickhouse, &id)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    if existing.is_none() {
        return Err(ApiError::NotFound(format!("SQL source '{id}' not found.")).into());
    }
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
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let id =
        q.id.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?
            .to_owned();
    let ch = &state.clickhouse;
    if sources::get_source(ch, &id)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .is_none()
    {
        return Err(ApiError::NotFound(format!("SQL source '{id}' not found.")).into());
    }
    let charts = store::list_stored_charts(ch)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let users = sources::charts_using_source(&charts, &id);
    if !users.is_empty() {
        return Err(ApiError::Conflict(format!(
            "SQL source is still used by {} chart(s): {}",
            users.len(),
            users.join(", ")
        ))
        .into());
    }
    sources::delete_source(ch, &id)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// `POST /api/dashboard/sources/preview` — validate, rewrite and run a SQL
/// source without saving it; returns its columns and the first
/// [`PREVIEW_ROWS`] rows.
///
/// # Errors
///
/// 400 on a bad body, 422 when the SQL is refused or fails, 503 when
/// `ClickHouse` is unreachable.
pub async fn preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: PreviewBody = parse(&body)?;
    let (_, rewritten) = checked_and_rewritten(&state, &principal, &body.sql).await?;
    let columns = probe_columns(&state.clickhouse, &rewritten).await?;
    let sql =
        format!("SELECT * FROM (\n{rewritten}\n) AS src LIMIT {PREVIEW_ROWS}{SQL_SOURCE_SETTINGS}");
    let result = state
        .clickhouse
        .query(&sql, None)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({
        "columns": columns,
        "rows": result.data,
    })))
}
