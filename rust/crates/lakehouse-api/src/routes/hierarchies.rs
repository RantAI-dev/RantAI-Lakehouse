//! `GET/POST/PUT/DELETE /api/dashboard/hierarchies` — hierarchies (`BI-10`,
//! with `AI-6`): a named, ordered list of 2 to 6 columns of one table or SQL
//! source, used to drill down on a chart.
//!
//! # Posture
//!
//! - A hierarchy is column names only. These routes read a source's *column
//!   list* (schema metadata, like `GET /api/dashboard/fields`) and write
//!   `console.bi_hierarchy`; they never run a statement over data.
//! - Reading needs `dashboard:read`; defining, changing and deleting need
//!   `dashboard:write` (feature page, decision 4).
//! - Charts do not store a hierarchy, so deleting one cannot break a chart and
//!   there is no "in use" rule.
//! - A `ClickHouse` failure is classified; its text never leaves the log.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::fields::SourceKind;
use lakehouse_bi::hierarchies::{self, HierarchyDef};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::calc_fields::{author_name, classify_ch, resolve};
use crate::routes::dashboard_access::require_entitled;
use crate::state::AppState;

/// `GET` query: exactly one of `mart` and `source`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    mart: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `DELETE` query.
#[derive(Debug, Deserialize)]
pub struct IdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// `POST` / `PUT` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    /// `PUT`: the hierarchy to change.
    #[serde(default)]
    id: Option<String>,
    /// `POST`: the mart, or ...
    #[serde(default)]
    mart: Option<String>,
    /// ... the SQL source id.
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    levels: Vec<String>,
}

fn parse_body(body: &Bytes) -> Result<Body, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_unparsed| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

/// The route's handler: [`list`] behind the entry check of `BI-19` part B1
/// (plan D8). The router floor is "signed in"; this refuses somebody who holds
/// neither `dashboard:read` nor a rule naming them or one of their roles. Kept
/// apart from [`list`] because the assistant calls that one directly, after
/// its own gate.
pub async fn list_guarded(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    query: Query<ListQuery>,
) -> ApiResult<ApiJson<Value>> {
    require_entitled(&state, &principal).await?;
    list(State(state), query).await
}

/// `GET /api/dashboard/hierarchies?mart=|source=` — the hierarchies of one
/// source.
///
/// # Errors
///
/// 400 unless exactly one of `mart` and `source` is given; a fixed 422/503
/// for a database failure.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<ApiJson<Value>> {
    let (kind, id) = match (q.mart.as_deref(), q.source.as_deref()) {
        (Some(m), None) if !m.trim().is_empty() => (
            SourceKind::Mart,
            m.trim()
                .strip_prefix("serving.")
                .unwrap_or(m.trim())
                .to_owned(),
        ),
        (None, Some(s)) if !s.trim().is_empty() => (SourceKind::SqlSource, s.trim().to_owned()),
        _ => {
            return Err(
                ApiError::BadRequest("choose either a mart or a SQL source.".to_owned()).into(),
            );
        }
    };
    let found = hierarchies::list_for(&state.clickhouse, kind, &id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({
        "sourceKind": kind.as_str(),
        "sourceId": id,
        "hierarchies": found,
    })))
}

/// `POST /api/dashboard/hierarchies` — define a hierarchy.
///
/// # Errors
///
/// 400 for a bad body, name, levels or source; 409 for a name already used on
/// that source; a fixed 422/503 for a database failure.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let ch = &state.clickhouse;
    let source = resolve(ch, parsed.mart.as_deref(), parsed.source.as_deref()).await?;
    let name = parsed.name.trim().to_owned();
    hierarchies::validate(&name, &parsed.levels, &source.cols).map_err(ApiError::BadRequest)?;
    let siblings = hierarchies::list_for(ch, source.kind, &source.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    if siblings.iter().any(|h| h.name == name) {
        return Err(ApiError::Conflict(format!(
            "a hierarchy called '{name}' already exists on this source."
        ))
        .into());
    }
    if siblings.len() >= hierarchies::MAX_PER_SOURCE {
        return Err(ApiError::BadRequest(format!(
            "a source has at most {} hierarchies.",
            hierarchies::MAX_PER_SOURCE
        ))
        .into());
    }
    let hierarchy = HierarchyDef {
        id: hierarchies::new_id(),
        source_kind: source.kind,
        source_id: source.id,
        name,
        levels: parsed.levels,
        created_by: author_name(principal.as_ref()),
        updated_at: None,
    };
    hierarchies::save(ch, &hierarchy)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "hierarchy": hierarchy })))
}

/// `PUT /api/dashboard/hierarchies` — change a hierarchy's name and levels.
///
/// # Errors
///
/// 400 for a bad body, 404 for an unknown id, 409 for a name already used by
/// another hierarchy of the source.
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let ch = &state.clickhouse;
    let id = parsed
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let current = hierarchies::get(ch, id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("hierarchy '{id}' not found.")))?;
    let source = match current.source_kind {
        SourceKind::Mart => resolve(ch, Some(&current.source_id), None).await?,
        SourceKind::SqlSource => resolve(ch, None, Some(&current.source_id)).await?,
    };
    let name = parsed.name.trim().to_owned();
    hierarchies::validate(&name, &parsed.levels, &source.cols).map_err(ApiError::BadRequest)?;
    let siblings = hierarchies::list_for(ch, source.kind, &source.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    if siblings
        .iter()
        .any(|h| h.name == name && h.id != current.id)
    {
        return Err(ApiError::Conflict(format!(
            "a hierarchy called '{name}' already exists on this source."
        ))
        .into());
    }
    let hierarchy = HierarchyDef {
        name,
        levels: parsed.levels,
        created_by: author_name(principal.as_ref()),
        updated_at: None,
        ..current
    };
    hierarchies::save(ch, &hierarchy)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true, "hierarchy": hierarchy })))
}

/// `DELETE /api/dashboard/hierarchies?id=` — remove a hierarchy. Nothing can
/// be using it: charts do not store one.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one.
pub async fn delete(
    State(state): State<AppState>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let ch = &state.clickhouse;
    let id =
        q.id.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))?;
    let current = hierarchies::get(ch, id)
        .await
        .map_err(|failure| classify_ch(&failure))?
        .ok_or_else(|| ApiError::NotFound(format!("hierarchy '{id}' not found.")))?;
    hierarchies::delete(ch, &current.id)
        .await
        .map_err(|failure| classify_ch(&failure))?;
    Ok(ApiJson(json!({ "ok": true })))
}
