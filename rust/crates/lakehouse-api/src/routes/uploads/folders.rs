//! `/api/uploads/folders*` and `POST /api/uploads/move`: the folders that group
//! the files on the Uploaded files tab (`DATA-35`, plan
//! `docs/superpowers/plans/2026-10-26-data-35-upload-folders.md`, AD-2 and AD-3;
//! feature page `docs/core/features/upload-folders.md`).
//!
//! # Posture
//!
//! * **Who.** `connector:manage`, the permission of the whole Sources page
//!   (`DATA-35` decision 4, `policy.rs`).
//! * **Whose.** The tenant is `tenant_scope::resolve`, never a value from the body
//!   (decision 5, `SEC-16`). A folder of another tenant answers the same 404 as an
//!   id that does not exist, and a move of files is all or nothing: one id that is
//!   not the caller's tenant's moves none.
//! * **The rules are the store's** (`lakehouse_store::upload_folders`): depth,
//!   cycle, name, empty-to-delete, checked against the database. Here each
//!   refusal becomes one fixed sentence (`SEC-11`); a database failure reaches the
//!   response only as the store's own fixed `database error`.
//! * **Static paths.** `folders` and `move` are mounted beside `/api/uploads/{id}`
//!   (`routes::uploads_router`); the router prefers a static segment, and an
//!   upload id (`up-<uuid>`) can never be either word. They belong to no upload,
//!   so they sit outside the per-upload tenant layer.
//! * **Body parsing is by hand**, so a bad body gets one of these sentences and not
//!   the extractor's text.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::audit::NewAuditEvent;
use lakehouse_store::upload_folders::{self, FolderError, FolderPatch, ParentChange, UploadFolder};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{BODY_NOT_AN_OBJECT, FOLDER_ID_RULE, NOT_FOUND, pool, record_audit};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

// ── Messages. Every one is fixed text; none carries upstream text. ───────

const NO_TENANT: &str = "Your account belongs to no tenant, so folders cannot be used.";
pub(super) const FOLDER_NOT_FOUND: &str = "Folder not found.";
const NAME_RULE: &str = "A folder name has 1 to 100 characters.";
const NAME_TAKEN: &str = "A folder with that name already exists in the same place.";
const TOO_DEEP: &str = "Folders can be nested at most 3 levels deep.";
const CYCLE: &str = "A folder cannot be moved into itself or into one of its own folders.";
const NOT_EMPTY: &str =
    "This folder still holds files or folders. Move them out and delete its folders first.";
const NAME_TEXT: &str = "name must be text.";
const PARENT_TEXT: &str = "parentId must be text or null.";
const NOTHING_TO_CHANGE: &str = "Send a name, a parentId, or both.";
const UPLOAD_IDS_RULE: &str = "uploadIds must be a list of 1 to 500 upload ids.";

/// The most files one move names. The tab lists at most 100 at a time, so this is
/// a bound on the request, not a product number; it matches the store's listing
/// clamp.
const MAX_MOVE_IDS: usize = 500;

/// The caller and their tenant, or the 401 / 400 / 404 of a request that has
/// none. `GET` callers in no tenant see an empty list instead; they do not call
/// this.
fn caller_and_tenant(
    principal: Option<Extension<Principal>>,
    headers: &HeaderMap,
) -> Result<(Principal, Uuid), ApiError> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized());
    };
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, headers)? else {
        return Err(ApiError::BadRequest(NO_TENANT.to_owned()));
    };
    Ok((principal, tenant_id))
}

/// A store refusal as the fixed sentence the caller sees. A database failure is
/// the store's own classified error (`database error`), never its text.
fn refusal(err: FolderError) -> ApiError {
    match err {
        FolderError::NotFound => ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()),
        FolderError::NameInvalid => ApiError::BadRequest(NAME_RULE.to_owned()),
        FolderError::NameTaken => ApiError::Conflict(NAME_TAKEN.to_owned()),
        FolderError::TooDeep => ApiError::BadRequest(TOO_DEEP.to_owned()),
        FolderError::Cycle => ApiError::BadRequest(CYCLE.to_owned()),
        FolderError::NotEmpty => ApiError::Conflict(NOT_EMPTY.to_owned()),
        FolderError::Store(err) => err.into(),
    }
}

fn json_object(body: &[u8]) -> Result<Map<String, Value>, ApiError> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(fields)) => Ok(fields),
        _ => Err(ApiError::BadRequest(BODY_NOT_AN_OBJECT.to_owned())),
    }
}

/// A request field that names a folder, with its three JSON states told apart:
/// absent, `null` and text. An enum, not an `Option<Option<String>>` (clippy
/// `option_option`, PR #259 CI), so the states carry names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FolderRef {
    /// The field is not in the body.
    Absent,
    /// The field is `null`: the top level, no folder.
    Null,
    /// The field is the id of a folder.
    Id(String),
}

impl FolderRef {
    /// The folder id, or `None` for both "absent" and `null`.
    pub(super) fn into_id(self) -> Option<String> {
        match self {
            Self::Id(id) => Some(id),
            Self::Absent | Self::Null => None,
        }
    }

    /// The same three states as a patch's parent change.
    fn as_change(&self) -> ParentChange<'_> {
        match self {
            Self::Absent => ParentChange::Keep,
            Self::Null => ParentChange::Root,
            Self::Id(id) => ParentChange::Under(id),
        }
    }
}

/// `text` or `null` for a field that names a folder; [`FolderRef::Absent`] when
/// it is absent.
fn optional_folder_ref(
    fields: &Map<String, Value>,
    name: &str,
    sentence: &str,
) -> Result<FolderRef, ApiError> {
    match fields.get(name) {
        None => Ok(FolderRef::Absent),
        Some(Value::Null) => Ok(FolderRef::Null),
        Some(Value::String(id)) => Ok(FolderRef::Id(id.clone())),
        Some(_) => Err(ApiError::BadRequest(sentence.to_owned())),
    }
}

fn folder_audit_event(
    principal: &Principal,
    action: &str,
    kind: &str,
    resource_id: Option<&str>,
    args: Value,
) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: action.to_owned(),
        resource_kind: Some(kind.to_owned()),
        resource_id: resource_id.map(str::to_owned),
        args: Some(args),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// `GET /api/uploads/folders`: the caller's tenant's folders, each with the
/// number of files and of folders directly in it, counted on the server.
///
/// # Errors
///
/// 401 without a session; 404 if `X-Tenant` names a tenant the caller does not
/// belong to; 503 if no pool is configured; 500 on a database failure.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<UploadFolder>>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Ok(ApiJson(Vec::new()));
    };
    Ok(ApiJson(
        upload_folders::list(pool(&state)?, tenant_id).await?,
    ))
}

/// `POST /api/uploads/folders` `{name, parentId?}`: make a folder.
///
/// # Errors
///
/// 400 for a body that is not an object, a name that is not text or not 1 to 100
/// characters, a parent that is not text or null, a caller in no tenant, or a
/// folder that would sit deeper than 3 levels (`DATA-35` decision 3); 404 if the
/// parent is not a folder of the caller's tenant; 409 if a folder of that name
/// (case-insensitively) is already in the same place (decision 6); 503 if no pool
/// is configured; 500 on a database failure.
pub async fn create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<UploadFolder>)> {
    let (principal, tenant_id) = caller_and_tenant(principal, &headers)?;
    let fields = json_object(&body)?;
    let name = match fields.get("name") {
        Some(Value::String(name)) => name.as_str(),
        _ => return Err(ApiError::BadRequest(NAME_TEXT.to_owned()).into()),
    };
    let parent = optional_folder_ref(&fields, "parentId", PARENT_TEXT)?.into_id();
    let id = format!("uf-{}", Uuid::new_v4());
    let made = upload_folders::create(
        pool(&state)?,
        tenant_id,
        &id,
        parent.as_deref(),
        name,
        &principal.display_name,
    )
    .await
    .map_err(refusal)?;
    let args = json!({ "name": made.name, "parentId": made.parent_id });
    let event = folder_audit_event(
        &principal,
        "upload.folder.create",
        "upload_folder",
        Some(&id),
        args,
    );
    record_audit(pool(&state)?, event).await;
    Ok((StatusCode::CREATED, ApiJson(made)))
}

/// `PATCH /api/uploads/folders/{id}` `{name?, parentId?}`: rename a folder, move
/// it (`parentId: null` is the top level), or both.
///
/// # Errors
///
/// As [`create`], plus 400 when the move would put the folder into itself or its
/// own folders, and 400 for a body that changes nothing; 404 if the folder is not
/// the caller's tenant's.
pub async fn update(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<UploadFolder>> {
    let (principal, tenant_id) = caller_and_tenant(principal, &headers)?;
    let fields = json_object(&body)?;
    let name = match fields.get("name") {
        None => None,
        Some(Value::String(name)) => Some(name.as_str()),
        Some(_) => return Err(ApiError::BadRequest(NAME_TEXT.to_owned()).into()),
    };
    let parent = optional_folder_ref(&fields, "parentId", PARENT_TEXT)?;
    if name.is_none() && parent == FolderRef::Absent {
        return Err(ApiError::BadRequest(NOTHING_TO_CHANGE.to_owned()).into());
    }
    let pool = pool(&state)?;
    let changed = upload_folders::update(
        pool,
        tenant_id,
        &id,
        FolderPatch {
            name,
            parent: parent.as_change(),
        },
    )
    .await
    .map_err(refusal)?;
    // One event per kind of change, so the audit log reads "renamed" and "moved".
    if name.is_some() {
        // The name the folder ended up with, not the one that was typed: the
        // store trims it (`PR #259 review SHOULD-FIX 5`).
        let args = json!({ "name": changed.name });
        record_audit(
            pool,
            folder_audit_event(
                &principal,
                "upload.folder.rename",
                "upload_folder",
                Some(&id),
                args,
            ),
        )
        .await;
    }
    if parent != FolderRef::Absent {
        let args = json!({ "parentId": changed.parent_id });
        record_audit(
            pool,
            folder_audit_event(
                &principal,
                "upload.folder.move",
                "upload_folder",
                Some(&id),
                args,
            ),
        )
        .await;
    }
    Ok(ApiJson(changed))
}

/// `DELETE /api/uploads/folders/{id}`: delete a folder that holds no file and no
/// folder (`DATA-35` decision 8).
///
/// # Errors
///
/// 404 if the folder is not the caller's tenant's; 409 while it holds a file or a
/// folder; 400 for a caller in no tenant; 503 if no pool is configured; 500 on a
/// database failure.
pub async fn delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let (principal, tenant_id) = caller_and_tenant(principal, &headers)?;
    let pool = pool(&state)?;
    upload_folders::delete(pool, tenant_id, &id)
        .await
        .map_err(refusal)?;
    let event = folder_audit_event(
        &principal,
        "upload.folder.delete",
        "upload_folder",
        Some(&id),
        json!({}),
    );
    record_audit(pool, event).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/uploads/move` `{uploadIds: [..], folderId: string|null}`: put the
/// files in a folder, or out of any (`null`). All the files are the caller's
/// tenant's or none moves. Answers `{moved: n}`.
///
/// # Errors
///
/// 400 for a body that is not an object, `uploadIds` that is not 1 to 500 text
/// ids, a `folderId` that is not text or null, or a caller in no tenant; 404 if
/// the folder is not a folder of the caller's tenant, or if any one upload is not
/// the caller's tenant's (the upload sentence, as every upload route says it; no
/// file moves); 503 if no pool is configured; 500 on a database failure.
pub async fn move_uploads(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let (principal, tenant_id) = caller_and_tenant(principal, &headers)?;
    let fields = json_object(&body)?;
    let upload_ids = upload_ids_of(&fields)?;
    // `folderId` is required: `null` means "out of any folder" and must be said.
    let folder = match optional_folder_ref(&fields, "folderId", FOLDER_ID_RULE)? {
        FolderRef::Absent => return Err(ApiError::BadRequest(FOLDER_ID_RULE.to_owned()).into()),
        given => given.into_id(),
    };
    let pool = pool(&state)?;
    // The folder is checked on its own so that "no such folder" and "no such
    // upload" keep their own sentences; the store checks both again inside the
    // transaction that moves.
    if let Some(folder) = &folder
        && !upload_folders::folder_in_tenant(pool, tenant_id, folder).await?
    {
        return Err(ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()).into());
    }
    let moved = upload_folders::move_uploads(pool, tenant_id, &upload_ids, folder.as_deref())
        .await
        .map_err(|err| match err {
            FolderError::NotFound => ApiError::NotFound(NOT_FOUND.to_owned()),
            other => refusal(other),
        })?;
    let args = json!({ "uploadIds": upload_ids, "folderId": folder });
    record_audit(
        pool,
        folder_audit_event(&principal, "upload.move", "upload", None, args),
    )
    .await;
    Ok(ApiJson(json!({ "moved": moved })))
}

fn upload_ids_of(fields: &Map<String, Value>) -> Result<Vec<String>, ApiError> {
    let rule = || ApiError::BadRequest(UPLOAD_IDS_RULE.to_owned());
    let Some(Value::Array(ids)) = fields.get("uploadIds") else {
        return Err(rule());
    };
    if ids.is_empty() || ids.len() > MAX_MOVE_IDS {
        return Err(rule());
    }
    ids.iter()
        .map(|id| id.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(rule)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// Every refusal the store can make is a sentence written in this file, and a
    /// database failure is not one of them.
    #[test]
    fn each_store_refusal_has_its_own_fixed_sentence() {
        let cases = [
            (FolderError::NotFound, 404, FOLDER_NOT_FOUND),
            (FolderError::NameInvalid, 400, NAME_RULE),
            (FolderError::NameTaken, 409, NAME_TAKEN),
            (FolderError::TooDeep, 400, TOO_DEEP),
            (FolderError::Cycle, 400, CYCLE),
            (FolderError::NotEmpty, 409, NOT_EMPTY),
        ];
        for (err, status, sentence) in cases {
            let api = refusal(err);
            assert_eq!(api.status(), status, "{sentence}");
            assert_eq!(api.to_string(), sentence);
        }
    }

    #[test]
    fn the_move_list_must_be_one_to_five_hundred_text_ids() {
        let fields = |value: Value| json_object(value.to_string().as_bytes()).unwrap();
        assert!(upload_ids_of(&fields(json!({ "uploadIds": ["up-1"] }))).is_ok());
        for bad in [
            json!({}),
            json!({ "uploadIds": [] }),
            json!({ "uploadIds": "up-1" }),
            json!({ "uploadIds": [1] }),
            json!({ "uploadIds": vec!["up"; MAX_MOVE_IDS + 1] }),
        ] {
            assert!(upload_ids_of(&fields(bad)).is_err());
        }
    }

    #[test]
    fn a_folder_reference_may_be_absent_null_or_text_and_nothing_else() {
        let obj = |value: Value| json_object(value.to_string().as_bytes()).unwrap();
        let read = |value: Value| optional_folder_ref(&obj(value), "p", PARENT_TEXT);
        assert_eq!(read(json!({})).unwrap(), FolderRef::Absent);
        assert_eq!(read(json!({ "p": null })).unwrap(), FolderRef::Null);
        assert_eq!(
            read(json!({ "p": "uf-1" })).unwrap(),
            FolderRef::Id("uf-1".to_owned())
        );
        assert!(read(json!({ "p": 3 })).is_err());
    }
}
