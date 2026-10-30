//! The saved-query routes and the folder routes: what a person does with
//! the queries they saved (`QS-3`).
//!
//! # Two spaces, one resolver
//!
//! Every organisation has a **shared space** — the root, where a query with
//! no folder sits — that every member of the organisation sees at `view`,
//! and a **personal space** — one folder per person — that only its owner
//! sees. The level the caller holds on an item is decided by the one
//! resolver in `lakehouse_store::saved_query_access`, so this module never
//! decides visibility on its own: it asks the store, which answers with the
//! level, and it turns that level into a 404, a 403 or the write.
//!
//! # 404 before 403
//!
//! An item the caller may not see is a **404** with a fixed sentence — the
//! same sentence as for one that does not exist, so the answer never says
//! which it was. An item the caller may see but may not change is a
//! **403** with its own fixed sentence. The order is fixed in every
//! handler here: the route reads the item first, and only then asks what
//! the caller may do with it.
//!
//! # Fixed sentences
//!
//! Every refusal carries fixed text. The store says *what* was wrong; this
//! module says *what to tell the caller*. No upstream text ever reaches a
//! response (`AGENTS.md` principle 4), and no sentence here names a row
//! that the caller may not see.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::StoreError;
use lakehouse_store::queries::{
    self, FolderMove, SavedQuery, SavedQueryFolder, SavedQueryPatch, SavedQueryPlacement,
    SavedQueryTrashEntry, SavedQueryVersion,
};
use lakehouse_store::saved_query_access::{self, FolderError, FolderNode, Level};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::quality::pool;
use crate::routes::query::{saved_query_scope, saved_query_viewer};
use crate::sql_guard::is_read_only;
use crate::state::AppState;

// ── The fixed sentences ──────────────────────────────────────────────────
//
// Every one of them is fixed text, and every one of them is a sentence
// this repository writes. They are collected here so that the same
// refusal always reads the same way, whatever route answers it.

/// The sentence a caller that may not see a saved query is told.
///
/// The same sentence as for a query that does not exist, so the answer
/// never says which it was.
pub(crate) const QUERY_NOT_FOUND: &str = "That saved query could not be found.";

/// The sentence a caller that may not see a folder is told.
///
/// The same sentence as for a folder that does not exist.
pub(crate) const FOLDER_NOT_FOUND: &str = "That folder could not be found.";

/// The sentence a caller that may not change a saved query is told.
pub(crate) const QUERY_FORBIDDEN: &str = "You do not have permission to change this saved query.";

/// The sentence a caller that may not change a folder is told.
pub(crate) const FOLDER_FORBIDDEN: &str = "You do not have permission to change this folder.";

/// The sentence a folder that still holds queries is refused with.
pub(crate) const FOLDER_HOLDS_QUERIES: &str =
    "This folder still holds queries, some of them in the trash. Move or restore them first.";

/// The sentence a folder that still holds folders is refused with.
pub(crate) const FOLDER_HOLDS_FOLDERS: &str =
    "This folder still holds other folders. Move or delete them first.";

/// The sentence a taken folder name is refused with.
pub(crate) const FOLDER_DUPLICATE_NAME: &str = "A folder with that name already exists here.";

/// The sentence a bad folder name is refused with.
pub(crate) const FOLDER_INVALID_NAME: &str = "A folder name must be between 1 and 100 characters.";

/// The sentence a too-deep tree is refused with.
pub(crate) const FOLDER_TOO_DEEP: &str = "A folder tree cannot be deeper than four levels.";

/// The sentence a cycle is refused with.
pub(crate) const FOLDER_CYCLE: &str = "A folder cannot be moved into itself.";

/// The sentence a caller that acts for no organisation is refused with
/// when it asks for a folder.
pub(crate) const FOLDER_NEEDS_TENANT: &str = "A folder belongs to an organisation.";

/// The sentence a query string the route cannot read is told.
pub(crate) const QUERY_STRING_UNREADABLE: &str = "The query string could not be read.";

/// The sentence a folder reference that is neither an id nor the root is
/// refused with.
pub(crate) const FOLDER_ID_INVALID: &str = "A folder must be an id or \"root\".";

/// The sentence a statement that is not read-only is refused with.
///
/// The same sentence the save route already used, so a statement that
/// cannot be saved cannot be changed into one that can.
pub(crate) const NOT_READ_ONLY: &str =
    "only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) can be saved";

/// The sentence a saved query without a title or without a statement is
/// refused with.
pub(crate) const TITLE_AND_SQL_REQUIRED: &str = "title and sql are required";

// ── The shared shapes ────────────────────────────────────────────────────

/// The `GET /api/query/saved` query string.
///
/// Every field stays a raw string on purpose: this struct parses whatever
/// the caller sends, so a bad *value* becomes a 400 with this surface's
/// own fixed message — never `serde_urlencoded`'s rendering of a serde
/// error. A query string the extractor itself cannot read (a parameter
/// sent twice) never reaches this struct: it is refused with the fixed
/// sentence of [`QUERY_STRING_UNREADABLE`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryListQuery {
    /// The text typed in the search box: it matches the title or the SQL.
    #[serde(default)]
    pub q: Option<String>,
    /// The folder asked for: an id, or `"root"` for the shared space.
    #[serde(default)]
    pub folder: Option<String>,
}

/// A body field that may be absent, `null` or a value.
///
/// `serde`'s own `Option` cannot tell an absent field from a `null` one:
/// both are `None`. The console sends `folderId: null` for the shared
/// space and leaves the field out for "leave it where it is", so the two
/// have to be different answers. An enum with a named state per answer,
/// not an `Option<Option<Value>>` (clippy `option_option`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum FolderField {
    /// The field is not in the body.
    #[default]
    Absent,
    /// The field is `null`: the shared space, the root of the tree.
    Null,
    /// The field carries a value.
    Value(Value),
}

impl<'de> Deserialize<'de> for FolderField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<Value>::deserialize(deserializer)? {
            None => Self::Null,
            Some(value) => Self::Value(value),
        })
    }
}

/// The folder a request names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderReference {
    /// The request named no folder.
    Absent,
    /// The shared space: the root of the tree.
    SharedSpace,
    /// One folder, by id.
    Folder(Uuid),
}

/// The folder a body names, and the refusals a name that is neither an id
/// nor the root earns.
///
/// # Errors
///
/// 400 with the fixed sentence of [`FOLDER_ID_INVALID`] for a name that is
/// neither an id nor `"root"`.
pub(crate) fn parse_folder_reference(raw: FolderField) -> Result<FolderReference, ApiError> {
    match raw {
        FolderField::Absent => Ok(FolderReference::Absent),
        FolderField::Null => Ok(FolderReference::SharedSpace),
        FolderField::Value(Value::String(value)) => {
            let value = value.trim();
            if value.eq_ignore_ascii_case("root") {
                return Ok(FolderReference::SharedSpace);
            }
            Uuid::parse_str(value)
                .map(FolderReference::Folder)
                .map_err(|_err| ApiError::BadRequest(FOLDER_ID_INVALID.to_owned()))
        }
        FolderField::Value(_other) => Err(ApiError::BadRequest(FOLDER_ID_INVALID.to_owned())),
    }
}

/// The folder the list is filtered by.
///
/// An empty value filters nothing: a client that builds a link from an
/// empty input shows the whole list instead of a list it cannot read.
///
/// # Errors
///
/// 400 with the fixed sentence of [`FOLDER_ID_INVALID`] for a value that is
/// neither an id nor `"root"`.
pub(crate) fn parse_folder_filter(raw: Option<&str>) -> Result<Option<SavedQueryFolder>, ApiError> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.eq_ignore_ascii_case("root") {
        return Ok(Some(SavedQueryFolder::Root));
    }
    Uuid::parse_str(value)
        .map(|id| Some(SavedQueryFolder::Folder(id)))
        .map_err(|_err| ApiError::BadRequest(FOLDER_ID_INVALID.to_owned()))
}

/// One folder, as the console sees it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderNodeWire {
    /// The folder's id.
    pub id: String,
    /// The folder this one sits in; `None` at the root.
    pub parent_id: Option<String>,
    /// The folder's name.
    pub name: String,
    /// Whether this is a personal space: one person's own space, which
    /// only that person sees.
    pub is_personal: bool,
    /// The person the folder belongs to: the owner of a personal space,
    /// the creator of an ordinary folder.
    pub owner_id: Option<String>,
    /// The level the caller holds on it: "view", "edit" or "manage".
    pub level: String,
}

impl FolderNodeWire {
    /// The wire shape of a folder, at the level the caller holds on it.
    fn new(folder: &FolderNode, level: Level) -> Self {
        Self {
            id: folder.id.to_string(),
            parent_id: folder.parent_id.map(|id| id.to_string()),
            name: folder.name.clone(),
            is_personal: folder.is_personal,
            owner_id: folder.owner_id.map(|id| id.to_string()),
            level: level.as_str().to_owned(),
        }
    }
}

// ── The shared rules ─────────────────────────────────────────────────────

/// The level the caller holds on a saved query.
///
/// The store writes the level as a word, so it always parses; a word
/// outside the set is read as `view` — the weakest level — so a level
/// this module cannot read never widens what the caller may do.
fn level_of(level: &str) -> Level {
    Level::parse(level).unwrap_or(Level::View)
}

/// The level the caller holds on a folder, or a 404 when the folder is
/// not theirs to see.
///
/// The folders are read with the one resolver
/// ([`saved_query_access::folders_with_levels`]), so the tree and the
/// resolver cannot disagree about who may do what, and a folder the
/// caller may not see is not in the answer at all.
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see; 500 on a database failure.
async fn folder_level(
    pool: &PgPool,
    viewer: &queries::SavedQueryViewer,
    id: Uuid,
) -> Result<Level, ApiError> {
    let folders = saved_query_access::folders_with_levels(pool, viewer).await?;
    folders
        .into_iter()
        .find(|(folder, _level)| folder.id == id)
        .map(|(_folder, level)| level)
        .ok_or_else(|| ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()))
}

/// The route's own answer to a folder write the store refused.
///
/// The store says *what* was wrong; this says *what to tell the caller*,
/// with the fixed sentence each case earns. The database's own text never
/// reaches a response (`AGENTS.md` principle 4).
fn folder_failure(failure: FolderError) -> ApiError {
    match failure {
        FolderError::NotFound => ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()),
        FolderError::HoldsQueries | FolderError::HoldsTrashedQueries => {
            ApiError::Conflict(FOLDER_HOLDS_QUERIES.to_owned())
        }
        FolderError::HoldsFolders => ApiError::Conflict(FOLDER_HOLDS_FOLDERS.to_owned()),
        FolderError::DuplicateName => ApiError::Conflict(FOLDER_DUPLICATE_NAME.to_owned()),
        FolderError::InvalidName => ApiError::BadRequest(FOLDER_INVALID_NAME.to_owned()),
        FolderError::TooDeep => ApiError::BadRequest(FOLDER_TOO_DEEP.to_owned()),
        FolderError::Cycle => ApiError::BadRequest(FOLDER_CYCLE.to_owned()),
        FolderError::PersonalSpace => ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()),
        FolderError::Database(_database) => ApiError::Internal("database error".to_owned()),
    }
}

/// The route's own answer to a saved-query write the store refused.
///
/// The store answers `NotFound` for a row the caller may not change — it
/// does not say whether the row exists — so the route maps it to the
/// fixed 404 sentence.
///
/// # Errors
///
/// The `ApiError` the refusal becomes: 404 with the fixed sentence of
/// [`QUERY_NOT_FOUND`], or whatever the store's own classification says
/// for anything else.
fn query_failure(failure: StoreError) -> ApiError {
    match failure {
        StoreError::NotFound => ApiError::NotFound(QUERY_NOT_FOUND.to_owned()),
        other => ApiError::from(other),
    }
}

/// Where a new saved query lands, checked against what the caller may do.
///
/// A query may be placed in the shared space (every `query:read` holder
/// of the organisation may place one there), in the caller's own personal
/// space (the resolver gives its owner `manage`), or in a folder the
/// caller holds **edit** on. A folder the caller may only see is not
/// theirs to fill, so it is refused the same way a change of a query it
/// may only see is refused.
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see; 403 with the fixed sentence of [`QUERY_FORBIDDEN`]
/// for a folder the caller may see but not fill.
pub(crate) async fn require_query_placement(
    pool: &PgPool,
    viewer: &queries::SavedQueryViewer,
    reference: FolderReference,
) -> Result<SavedQueryPlacement, ApiError> {
    match reference {
        FolderReference::Absent => Ok(SavedQueryPlacement::PersonalSpace),
        FolderReference::SharedSpace => Ok(SavedQueryPlacement::SharedSpace),
        FolderReference::Folder(id) => {
            let level = folder_level(pool, viewer, id).await?;
            if level < Level::Edit {
                return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()));
            }
            Ok(SavedQueryPlacement::Folder(id))
        }
    }
}

/// The id in a path, or a 404 when it is not one.
///
/// The id is parsed here rather than by the extractor, so a malformed one
/// is a 404 with the fixed sentence — the same answer as for an id that
/// does not exist, and never a 400 that would say the shape of the answer
/// differs from the shape of a missing row's.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`].
fn query_id_of(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_err| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))
}

/// The id in a path, or a 404 when it is not one.
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`].
fn folder_id_of(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_err| ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()))
}

// ── The saved-query routes ───────────────────────────────────────────────

/// `GET /api/query/saved/{id}` — one saved query the caller may see.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see (and for one that does not exist); 500 on a
/// database failure.
pub async fn get_saved(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<SavedQuery>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let saved = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    Ok(ApiJson(saved))
}

/// The `PATCH /api/query/saved/{id}` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchSavedQueryBody {
    /// A new title.
    #[serde(default)]
    pub title: Option<String>,
    /// New SQL.
    #[serde(default)]
    pub sql: Option<String>,
    /// New tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// The folder the query moves to: an id, `null` for the shared space,
    /// or absent to leave it where it is.
    #[serde(default)]
    pub folder_id: FolderField,
}

/// `PATCH /api/query/saved/{id}` — change a saved query.
///
/// The caller must hold **edit** on the query: `edit` changes the SQL,
/// the title, the tags and the folder, `view` may do neither. A statement
/// that is not read-only is refused the same way the save route refuses
/// it, so nothing that can never run is ever stored.
///
/// # Errors
///
/// 400 for a malformed body or a blank title; 404 with the fixed sentence
/// of [`QUERY_NOT_FOUND`] for a query the caller may not see; 403 with the
/// fixed sentence of [`QUERY_FORBIDDEN`] for a query the caller may see
/// but not change, and for a folder the caller may see but not fill; 422
/// with the fixed sentence of [`NOT_READ_ONLY`] for a statement that is
/// not read-only; 500 on a database failure.
pub async fn patch_saved(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<SavedQuery>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        // A caller that acts for no organisation owns no saved query.
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    let saved = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    if level_of(&saved.level) < Level::Edit {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    // The body is read only now: a query the caller may not see is a 404
    // and a query it may not change is a 403, whatever the body holds.
    let body: PatchSavedQueryBody = serde_json::from_slice(&body).map_err(|_err| {
        ApiError::BadRequest("Body must be JSON {title?, sql?, tags?, folderId?}.".to_owned())
    })?;
    let title = body.title.map(|title| title.trim().to_owned());
    if title.as_deref().is_some_and(str::is_empty) {
        return Err(ApiError::BadRequest(TITLE_AND_SQL_REQUIRED.to_owned()).into());
    }
    let sql = body.sql.map(|sql| sql.trim().to_owned());
    if sql.as_deref().is_some_and(str::is_empty) {
        return Err(ApiError::BadRequest(TITLE_AND_SQL_REQUIRED.to_owned()).into());
    }
    // Every path that stores a statement goes through the same read-only
    // guard the save route uses.
    if let Some(statement) = sql.as_deref()
        && !is_read_only(statement)
    {
        return Err(ApiError::Unprocessable(NOT_READ_ONLY.to_owned()).into());
    }
    let folder = match parse_folder_reference(body.folder_id)? {
        FolderReference::Absent => None,
        FolderReference::SharedSpace => Some(FolderMove::Root),
        FolderReference::Folder(folder_id) => {
            let level = folder_level(pool(&state)?, &viewer, folder_id).await?;
            if level < Level::Edit {
                return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
            }
            Some(FolderMove::Into(folder_id))
        }
    };
    let patch = SavedQueryPatch {
        title,
        sql,
        tags: body.tags,
        folder,
    };
    let saved = queries::update_saved_query(pool(&state)?, &viewer, &id, &patch)
        .await
        .map_err(query_failure)?;
    Ok(ApiJson(saved))
}

/// `DELETE /api/query/saved/{id}` — move a saved query to the trash.
///
/// The caller must hold **manage** on the query: only then may it be
/// deleted. The query waits in the trash until it is restored or swept
/// away; it is not gone yet.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see; 403 with the fixed sentence of [`QUERY_FORBIDDEN`]
/// for a query the caller may see but not delete; 500 on a database
/// failure.
pub async fn delete_saved(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    let saved = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    if level_of(&saved.level) < Level::Manage {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    queries::trash_saved_query(pool(&state)?, &viewer, &id)
        .await
        .map_err(query_failure)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/query/saved/{id}/duplicate` — copy a saved query.
///
/// The copy lands in the caller's own personal space, whatever space the
/// original sits in: a copy is the caller's own work, and the shared
/// space is not where one's own work belongs. Seeing the original is
/// enough — the copy changes nothing — so `view` may duplicate.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see; 422 with the fixed sentence of [`NOT_READ_ONLY`]
/// for a query that holds a statement which can never run; 500 on a
/// database failure.
pub async fn duplicate_saved(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, ApiJson<SavedQuery>)> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    let source = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    // The copy stores the same statement, so it meets the same guard.
    if !is_read_only(&source.sql) {
        return Err(ApiError::Unprocessable(NOT_READ_ONLY.to_owned()).into());
    }
    let owner = principal.display_name.clone();
    let copy = queries::create_saved_query_at(
        pool(&state)?,
        &viewer,
        &source.title,
        &source.sql,
        &owner,
        &source.tags,
        SavedQueryPlacement::PersonalSpace,
    )
    .await?;
    Ok((StatusCode::CREATED, ApiJson(copy)))
}

/// `POST /api/query/saved/{id}/restore` — bring a query back out of the
/// trash.
///
/// The query comes back in the folder it was in. The caller must hold
/// **manage** on it: the trash entry is part of managing a query.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see; 403 with the fixed sentence of [`QUERY_FORBIDDEN`]
/// for a query the caller may see but not restore; 500 on a database
/// failure.
pub async fn restore_saved(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<SavedQuery>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    // The trash entry is part of managing a query, so it is found with the
    // same scope and the same resolver the trash list uses.
    let saved = queries::get_saved_including_trash(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    if level_of(&saved.level) < Level::Manage {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    let restored = queries::restore_saved_query(pool(&state)?, &viewer, &id)
        .await
        .map_err(query_failure)?;
    Ok(ApiJson(restored))
}

/// `GET /api/query/saved-trash` — the queries waiting in the trash.
///
/// The trash holds what the caller may **manage**: the entry in the trash
/// is part of managing a query, and a caller that may only see a query
/// has no business seeing where it waits.
///
/// # Errors
///
/// 500 on a database failure.
pub async fn list_trash(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<SavedQueryTrashEntry>>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    Ok(ApiJson(
        queries::list_saved_trash_entries(pool(&state)?, &scope).await?,
    ))
}

/// `GET /api/query/saved/{id}/versions` — the history of a saved query,
/// newest first.
///
/// Seeing the history is part of seeing the query: `view` may read it.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see; 500 on a database failure.
pub async fn list_versions(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<SavedQueryVersion>>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let saved = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    let query_id = query_id_of(&saved.id)?;
    Ok(ApiJson(
        queries::list_versions(pool(&state)?, query_id).await?,
    ))
}

/// The path of the revert route: the query and the version to go back to.
#[derive(Debug, Deserialize)]
pub struct RevertPath {
    /// The saved query's id.
    pub id: String,
    /// The version's number.
    pub version: String,
}

/// `POST /api/query/saved/{id}/versions/{n}/revert` — go back to an
/// earlier version of a saved query.
///
/// The revert writes a **new** version, holding the content of the
/// version it went back to, so going back is itself undoable. The
/// caller must hold **edit** on the query, and the statement the version
/// holds goes through the same read-only guard as any other statement
/// that is about to be stored.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see and for a version that does not exist; 403 with the
/// fixed sentence of [`QUERY_FORBIDDEN`] for a query the caller may see
/// but not change; 422 with the fixed sentence of [`NOT_READ_ONLY`] for a
/// version that holds a statement which can never run; 500 on a database
/// failure.
pub async fn revert_version(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(path): Path<RevertPath>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<SavedQuery>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    let saved = queries::get_saved(pool(&state)?, &path.id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    if level_of(&saved.level) < Level::Edit {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    let query_id = query_id_of(&saved.id)?;
    let version: i32 = path
        .version
        .trim()
        .parse()
        .map_err(|_err| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    // The statement the version holds is about to be stored again, so it
    // meets the same guard as any other statement that is stored.
    let versions = queries::list_versions(pool(&state)?, query_id).await?;
    let wanted = versions
        .iter()
        .find(|candidate| candidate.version == version)
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    if !is_read_only(&wanted.sql) {
        return Err(ApiError::Unprocessable(NOT_READ_ONLY.to_owned()).into());
    }
    let reverted = queries::revert_to_version(pool(&state)?, &viewer, query_id, version)
        .await
        .map_err(query_failure)?;
    Ok(ApiJson(reverted))
}

// ── The folder routes ────────────────────────────────────────────────────

/// `GET /api/query/folders` — the tree the caller may see.
///
/// The tree holds only the folders the caller may see, each with the
/// level the caller holds on it. The shared space is the root of the tree
/// and has no node of its own, and a personal space of another person is
/// not in the tree at all.
///
/// # Errors
///
/// 500 on a database failure.
pub async fn list_folders(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<FolderNodeWire>>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        // A caller that acts for no organisation owns no folder and sees
        // none.
        return Ok(ApiJson(Vec::new()));
    };
    let folders = saved_query_access::folders_with_levels(pool(&state)?, &viewer).await?;
    Ok(ApiJson(
        folders
            .iter()
            .map(|(folder, level)| FolderNodeWire::new(folder, *level))
            .collect(),
    ))
}

/// The `POST /api/query/folders` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderBody {
    /// The folder's name.
    pub name: String,
    /// The folder this one sits in: an id, `null` for the root, or absent
    /// for the root as well.
    #[serde(default)]
    pub parent_id: FolderField,
}

/// `POST /api/query/folders` — make a folder.
///
/// A folder belongs to an organisation, so the caller must act for one:
/// a caller that acts for none has nowhere to put a folder, and the
/// answer is the fixed sentence of [`FOLDER_NEEDS_TENANT`].
///
/// The parent must be one the caller may fill: a folder they may only
/// see is not theirs to put things in, and a personal space of another
/// person is not theirs at all.
///
/// # Errors
///
/// 400 for a malformed body, for a bad name, for a tree that would grow
/// too deep, for a move that would make the folder its own ancestor, for
/// a parent the caller may not fill and for a caller that acts for no
/// organisation; 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a
/// parent that does not exist; 409 with the fixed sentence of
/// [`FOLDER_DUPLICATE_NAME`] for a name a sibling already carries; 500 on
/// a database failure.
pub async fn create_folder(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<FolderNodeWire>)> {
    let body: CreateFolderBody = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest("Body must be JSON {name, parentId?}.".to_owned()))?;
    // The folder belongs to the tenant the request acts as.
    let tenant_id = crate::tenant_scope::resolve(&principal, &headers)?
        .ok_or_else(|| ApiError::BadRequest(FOLDER_NEEDS_TENANT.to_owned()))?;
    let viewer = saved_query_viewer(&principal, Some(tenant_id));
    let parent_id = match parse_folder_reference(body.parent_id)? {
        FolderReference::Absent | FolderReference::SharedSpace => None,
        FolderReference::Folder(parent_id) => {
            let level = folder_level(pool(&state)?, &viewer, parent_id).await?;
            if level < Level::Edit {
                return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
            }
            Some(parent_id)
        }
    };
    let folder = saved_query_access::create_folder(
        pool(&state)?,
        tenant_id,
        viewer.id,
        &body.name,
        parent_id,
    )
    .await
    .map_err(folder_failure)?;
    // The creator of a folder manages it, so the answer carries that.
    Ok((
        StatusCode::CREATED,
        ApiJson(FolderNodeWire::new(&folder, Level::Manage)),
    ))
}

/// The `PATCH /api/query/folders/{id}` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchFolderBody {
    /// A new name.
    #[serde(default)]
    pub name: Option<String>,
    /// The folder this one sits in: an id, `null` for the root, or absent
    /// to leave it where it is.
    #[serde(default)]
    pub parent_id: FolderField,
}

/// `PATCH /api/query/folders/{id}` — rename a folder or move it.
///
/// The caller must hold **edit** on the folder: renaming it and moving it
/// are changes to the tree the folder sits in. A **personal** space may be
/// neither renamed nor moved — it is one person's own space, and moving
/// it would move their private queries with it.
///
/// A body that carries both a name and a parent renames the folder first
/// and moves it second. The two are two writes and there is no
/// transaction across them, so a rename that is refused leaves the folder
/// where it was; a move that is refused leaves the renamed folder where it
/// was. The answer is the folder as it is after both writes.
///
/// # Errors
///
/// 400 for a malformed body, for a bad name, for a tree that would grow
/// too deep and for a move that would make the folder its own ancestor;
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see and for a parent that does not exist; 403 with the
/// fixed sentence of [`FOLDER_FORBIDDEN`] for a folder the caller may see
/// but not change; 409 with the fixed sentence of
/// [`FOLDER_DUPLICATE_NAME`] for a name a sibling already carries; 500 on
/// a database failure.
pub async fn patch_folder(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<FolderNodeWire>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()).into());
    };
    let folder_id = folder_id_of(&id)?;
    let level = folder_level(pool(&state)?, &viewer, folder_id).await?;
    if level < Level::Edit {
        return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
    }
    // The body is read only now: a folder the caller may not see is a
    // 404 and a folder it may not change is a 403, whatever the body
    // holds.
    let body: PatchFolderBody = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest("Body must be JSON {name?, parentId?}.".to_owned()))?;
    let mut folder = None;
    if let Some(name) = body.name.as_deref() {
        folder = Some(
            saved_query_access::rename_folder(pool(&state)?, folder_id, name)
                .await
                .map_err(folder_failure)?,
        );
    }
    match body.parent_id {
        FolderField::Absent => {
            // The body does not name a parent: the folder stays where it is.
        }
        reference => {
            let parent_id = match parse_folder_reference(reference)? {
                FolderReference::Absent | FolderReference::SharedSpace => None,
                FolderReference::Folder(parent_id) => {
                    let level = folder_level(pool(&state)?, &viewer, parent_id).await?;
                    if level < Level::Edit {
                        return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
                    }
                    Some(parent_id)
                }
            };
            folder = Some(
                saved_query_access::move_folder(pool(&state)?, folder_id, parent_id)
                    .await
                    .map_err(folder_failure)?,
            );
        }
    }
    let folder = folder.ok_or_else(|| ApiError::BadRequest("nothing to change.".to_owned()))?;
    let level = folder_level(pool(&state)?, &viewer, folder.id).await?;
    Ok(ApiJson(FolderNodeWire::new(&folder, level)))
}

/// `DELETE /api/query/folders/{id}` — delete a folder.
///
/// A folder is deleted only when it holds nothing: no live query, no
/// query in the trash and no folder of its own. The caller must hold
/// **manage** on it, and a **personal** space may not be deleted at all.
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see; 403 with the fixed sentence of
/// [`FOLDER_FORBIDDEN`] for a folder the caller may see but not delete;
/// 409 with the fixed sentence of [`FOLDER_HOLDS_QUERIES`] for a folder
/// that still holds queries and with the fixed sentence of
/// [`FOLDER_HOLDS_FOLDERS`] for one that still holds folders; 500 on a
/// database failure.
pub async fn delete_folder(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()).into());
    };
    let folder_id = folder_id_of(&id)?;
    let level = folder_level(pool(&state)?, &viewer, folder_id).await?;
    if level < Level::Manage {
        return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
    }
    saved_query_access::delete_folder(pool(&state)?, folder_id)
        .await
        .map_err(folder_failure)?;
    Ok(StatusCode::NO_CONTENT)
}
