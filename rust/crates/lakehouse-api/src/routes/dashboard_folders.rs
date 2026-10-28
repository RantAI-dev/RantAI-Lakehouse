//! `GET/POST/PUT/DELETE /api/dashboard/folders` — the nested folder tree
//! dashboards and SQL sources are filed in. Plan:
//! `docs/plans/DASHBOARD-SQL-SOURCES-FOLDERS-PLAN.md` §4 (stage 1).
//!
//! Listing needs `dashboard:read`; creating, renaming, moving and deleting
//! need `dashboard:write`, the same permission that edits boards
//! (`POLICY_TABLE`). The tree rules — at most four levels, no cycles, unique
//! sibling names — are `lakehouse_bi::folders::check_placement`, applied
//! here before every save. A folder is deleted only when empty (no
//! subfolders, dashboards or SQL sources): 409 otherwise, fail closed
//! rather than cascading. `ClickHouse` error text never reaches a response.
//!
//! # The personal space (`BI-18` part C, part 2)
//!
//! A folder with an owner is one user's personal space
//! (`lakehouse_bi::folders`). The folder list a caller gets holds the ordinary
//! folders and their own space (every space for `identity:write`); a space
//! holds dashboards only, cannot be renamed, moved or deleted, and takes only
//! its owner's dashboards: [`ensure_folder_usable`] is the one check boards go
//! through, and [`ensure_folder_exists`] (SQL sources, which are not private)
//! refuses a personal space outright.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::folders::{self, Folder};
use lakehouse_bi::sources;
use lakehouse_bi::store::{self, BiError};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;
use crate::upstream_error;

/// `POST` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    parent_id: Option<String>,
}

/// `PUT` body: rename and/or move. An absent field keeps its value.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
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

/// A `ClickHouse` failure as a fixed 503 with a reference id; the detail is
/// only logged (SEC-11). Kept as a named wrapper because "the dashboard store
/// is unavailable" is the one status every folder/board/source caller wants,
/// failed statement or outage alike.
pub(crate) fn classify_ch_error(err: &ChError) -> ApiError {
    upstream_error::report_ch(&DASHBOARD_STORE, err)
        .into_api_error(upstream_error::FailedAs::ServiceUnavailable)
}

/// Fixed words for the dashboard store (folders, boards, SQL sources).
pub(crate) const DASHBOARD_STORE: upstream_error::Context = upstream_error::Context::new(
    "dashboard store",
    "The dashboard store is unavailable.",
    "The dashboard store is unavailable.",
);

/// A `lakehouse_bi` error: its validation text is fixed and written for the
/// caller; a `ClickHouse` failure is classified.
pub(crate) fn classify_bi_error(err: &BiError) -> ApiError {
    match err {
        BiError::Validation(msg) => ApiError::BadRequest(msg.clone()),
        BiError::Clickhouse(ch) => classify_ch_error(ch),
    }
}

fn required_id(id: Option<&str>) -> Result<String, ApiError> {
    id.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::BadRequest("id is required.".to_owned()))
}

/// Refuse a `folder_id` that names no live ordinary folder. `""` (root) always
/// passes. Used by SQL sources before they are filed: a source is shared by
/// every dashboard, so it cannot be kept in somebody's personal space.
///
/// # Errors
///
/// 400 for an unknown folder or a personal space, 503 when the store is
/// unreachable.
pub(crate) async fn ensure_folder_exists(ch: &ChClient, folder_id: &str) -> Result<(), ApiError> {
    if folder_id.is_empty() {
        return Ok(());
    }
    let all = folders::list_folders(ch, &Viewer::system())
        .await
        .map_err(|err| classify_ch_error(&err))?;
    match all.iter().find(|f| f.id == folder_id) {
        None => Err(ApiError::BadRequest("folder not found.".to_owned())),
        Some(f) if !f.owner_id.is_empty() => Err(ApiError::BadRequest(
            "SQL sources cannot be kept in the personal space.".to_owned(),
        )),
        Some(_) => Ok(()),
    }
}

/// Refuse a `folder_id` a dashboard of `viewer` may not be filed in: unknown,
/// or somebody else's personal space (an administrator sees every space but
/// files only into their own). `""` (root) always passes.
///
/// # Errors
///
/// 400 with a fixed sentence, 503 when the store is unreachable.
pub(crate) async fn ensure_folder_usable(
    ch: &ChClient,
    folder_id: &str,
    viewer: &Viewer,
) -> Result<(), ApiError> {
    if folder_id.is_empty() {
        return Ok(());
    }
    let all = folders::list_folders(ch, viewer)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    match all.iter().find(|f| f.id == folder_id) {
        None => Err(ApiError::BadRequest("folder not found.".to_owned())),
        Some(f) if !f.owner_id.is_empty() && f.owner_id != viewer.id() => {
            Err(ApiError::BadRequest(
                "Only the owner can file a dashboard in a personal space.".to_owned(),
            ))
        }
        Some(_) => Ok(()),
    }
}

/// The 400 for a change a personal space does not allow.
fn personal_space_refusal() -> ApiError {
    ApiError::BadRequest(
        "The personal space cannot be renamed, moved, deleted or hold folders.".to_owned(),
    )
}

/// `GET /api/dashboard/folders` — every live folder (a flat list; the
/// client builds the tree from `parentId`).
///
/// # Errors
///
/// 503 when the store is unreachable.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let all = folders::list_folders(&state.clickhouse, &viewer_of(&principal))
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "folders": all })))
}

/// `POST /api/dashboard/folders` — create a folder.
///
/// # Errors
///
/// 400 on a bad body or a broken tree rule, 503 when the store is
/// unreachable.
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: CreateBody = parse(&body)?;
    let ch = &state.clickhouse;
    let all = folders::list_folders(ch, &viewer_of(&principal))
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let parent = body.parent_id.unwrap_or_default().trim().to_owned();
    if all.iter().any(|f| f.id == parent && !f.owner_id.is_empty()) {
        return Err(personal_space_refusal().into());
    }
    let name = folders::check_placement(&all, None, &body.name, &parent)
        .map_err(|rule| ApiError::BadRequest(rule.message().to_owned()))?;
    let folder = Folder {
        id: folders::new_folder_id(),
        name,
        parent_id: parent,
        created_by: principal.id.uuid().to_string(),
        updated_at: None,
        owner_id: String::new(),
    };
    folders::save_folder(ch, &folder)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "ok": true, "folder": folder })))
}

/// `PUT /api/dashboard/folders` — rename and/or move a folder.
///
/// # Errors
///
/// 400 on a bad body or a broken tree rule, 404 for an unknown id, 503 when
/// the store is unreachable.
pub async fn update(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: UpdateBody = parse(&body)?;
    let id = required_id(body.id.as_deref())?;
    let ch = &state.clickhouse;
    let all = folders::list_folders(ch, &viewer_of(&principal))
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let current = all
        .iter()
        .find(|f| f.id == id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound("folder not found.".to_owned()))?;
    if !current.owner_id.is_empty() {
        return Err(personal_space_refusal().into());
    }
    let name = body.name.unwrap_or_else(|| current.name.clone());
    let parent = body
        .parent_id
        .map_or_else(|| current.parent_id.clone(), |p| p.trim().to_owned());
    let name = folders::check_placement(&all, Some(&id), &name, &parent)
        .map_err(|rule| ApiError::BadRequest(rule.message().to_owned()))?;
    if all.iter().any(|f| f.id == parent && !f.owner_id.is_empty()) {
        return Err(personal_space_refusal().into());
    }
    let folder = Folder {
        id,
        name,
        parent_id: parent,
        created_by: principal.id.uuid().to_string(),
        updated_at: None,
        owner_id: String::new(),
    };
    folders::save_folder(ch, &folder)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "ok": true, "folder": folder })))
}

/// `DELETE /api/dashboard/folders?id=` — delete an EMPTY folder.
///
/// # Errors
///
/// 400 without an id, 404 for an unknown one, 409 while it still holds
/// subfolders, dashboards or SQL sources, 503 when the store is
/// unreachable.
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let id = required_id(q.id.as_deref())?;
    let ch = &state.clickhouse;
    let all = folders::list_folders(ch, &viewer_of(&principal))
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let Some(folder) = all.iter().find(|f| f.id == id) else {
        return Err(ApiError::NotFound("folder not found.".to_owned()).into());
    };
    if !folder.owner_id.is_empty() {
        return Err(personal_space_refusal().into());
    }
    let subfolders = all.iter().filter(|f| f.parent_id == id).count();
    // An ordinary folder never holds a private dashboard, so counting what the
    // caller may see is counting all of it.
    let boards = store::list_boards(ch, &viewer_of(&principal))
        .await
        .map_err(|err| classify_ch_error(&err))?
        .iter()
        .filter(|b| b.folder_id.as_deref() == Some(id.as_str()))
        .count();
    let source_count = sources::list_sources(ch)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .iter()
        .filter(|s| s.folder_id == id)
        .count();
    if subfolders + boards + source_count > 0 {
        return Err(ApiError::Conflict(format!(
            "folder is not empty: {subfolders} subfolder(s), {boards} dashboard(s), \
             {source_count} SQL source(s). Move or delete them first."
        ))
        .into());
    }
    folders::delete_folder(ch, &id)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "ok": true })))
}

/// `POST /api/dashboard/personal` — the caller's personal space, created on
/// first use. Idempotent: the folder id is derived from the caller, so asking
/// twice (or twice at once) is one folder. Needs `dashboard:write`, like
/// creating any dashboard to put in it.
///
/// # Errors
///
/// 503 when the store is unreachable.
pub async fn personal(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let folder = folders::ensure_personal_folder(
        &state.clickhouse,
        &principal.id.uuid().to_string(),
        &principal.display_name,
    )
    .await
    .map_err(|err| classify_ch_error(&err))?;
    Ok(ApiJson(json!({ "folder": folder })))
}
