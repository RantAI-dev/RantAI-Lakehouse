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
//!
//! # The access lists
//!
//! The last surface here is the sharing one: who has what on a saved query
//! or on a folder (`QS-3`). A query's list is read with **manage**,
//! because managing a query includes sharing it: a person who may only see
//! a query has no business seeing who else sees it. A folder's list is
//! read with **edit**, so a person who may save or move a query into a
//! folder sees who the folder is shared with before they do. Changing
//! either list needs **manage**. A list answers with the
//! item's own rules and the rules it inherits from the folders above it,
//! and the grantee search answers with the people and the roles of the
//! caller's organisation — the two halves of one question, "who may I
//! share this with?". A grantee that is not of the caller's organisation
//! is refused before anything is written, so a refused change leaves the
//! old rules exactly as they were.

use std::collections::{HashMap, HashSet};

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::StoreError;
use lakehouse_store::queries::{
    self, FolderMove, SavedQuery, SavedQueryFolder, SavedQueryPatch, SavedQueryPlacement,
    SavedQueryTrashEntry, SavedQueryVersion,
};
use lakehouse_store::saved_query_access::{
    self, FolderError, FolderMap, FolderNode, GranteePerson, GranteeRole, ItemRule, Level, NewGrant,
};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::QUERY_STRING_UNREADABLE;
use crate::routes::quality::pool;
use crate::routes::query::{checked_saved_definitions, saved_query_scope, saved_query_viewer};
use crate::sql_guard::is_read_only;
use crate::state::AppState;
use crate::upstream_error;

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

/// The sentence a caller that may not read a folder's access list is told.
///
/// It is not the sentence of a refusal to change ([`FOLDER_FORBIDDEN`]):
/// this route changes nothing, and it says what the caller would need to
/// see the list. `#416`.
pub(crate) const FOLDER_ACCESS_FORBIDDEN: &str =
    "Only someone who can edit this folder can see who it is shared with.";

/// The sentence a move at a level below manage is refused with: the same
/// sentence for a query move and for a folder move, and the sentence the
/// store's own refusal carries (`StoreError::NotAllowed`, and
/// `FolderError::MoveNeedsManage` through [`folder_failure`]), so every
/// refused move reads alike (`QS-3` review of #382: a move needs manage).
pub(crate) const MOVE_NEEDS_MANAGE: &str = "Only someone who manages this can move it.";

/// The sentence a rule on a personal space itself is refused with: a
/// personal space is shared through a query or a folder inside it, and the
/// resolver ignores a rule stored on the space (`QS-3` review of #382).
pub(crate) const PERSONAL_SPACE_NOT_SHARED: &str =
    "A personal space is not shared: share a query or a folder inside it.";

/// The sentence a share of a folder is refused with when the caller does
/// not manage everything in it: a rule on a folder covers every sub-folder
/// and every query under it, so sharing it is held to the bar of a move
/// (`QS-3` review of #393).
pub(crate) const SHARE_NEEDS_MANAGE_OF_CONTENTS: &str =
    "Only someone who manages everything in this folder can share it.";

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

/// The sentence a body of another shape is refused with.
pub(crate) const ACCESS_BODY_SHAPE: &str =
    "Body must be JSON {rules: [{userId?, roleId?, level}]}.";

/// The sentence a rule that names two grantees or none, or a level outside
/// the set, is refused with.
pub(crate) const ACCESS_RULE_SHAPE: &str =
    "Each rule must name one person or one role, and one of the levels view, edit or manage.";

/// The sentence a grantee that is listed twice is refused with.
pub(crate) const ACCESS_GRANTEE_TWICE: &str = "The same person or role is listed twice.";

/// The sentence a grantee that does not exist, or a person of another
/// organisation, is refused with.
pub(crate) const ACCESS_GRANTEE_OUTSIDE: &str =
    "A saved query can only be shared with a person or a role of your organisation.";

/// How many people and how many roles the grantee search lists at most.
pub(crate) const GRANTEE_LIMIT: i64 = 20;

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

/// One rule of an item's own access list, as the console sees it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessRuleWire {
    /// The rule's own id.
    pub id: String,
    /// The person the rule names; `None` when the rule names a role.
    pub user_id: Option<String>,
    /// The role the rule names; `None` when the rule names a person.
    pub role_id: Option<String>,
    /// The name of the person or the role the rule names; `None` (sent as
    /// `null`) when the grantee is not found in the caller's organisation.
    pub grantee_name: Option<String>,
    /// The email of the person the rule names; `None` (sent as `null`) for
    /// a role, and for a person not found in the caller's organisation.
    pub grantee_email: Option<String>,
    /// What the rule gives: "view", "edit" or "manage".
    pub level: String,
    /// Who wrote the rule; `None` when nobody is recorded.
    pub granted_by: Option<String>,
    /// When the rule was written.
    pub granted_at: String,
}

impl AccessRuleWire {
    /// The wire shape of a stored rule, with its grantee named from `names`.
    fn of(rule: &ItemRule, names: &GranteeNames) -> Self {
        let (grantee_name, grantee_email) = names.of(rule);
        Self {
            id: rule.id.to_string(),
            user_id: rule.user_id.map(|id| id.to_string()),
            role_id: rule.role_id.map(|id| id.to_string()),
            grantee_name,
            grantee_email,
            level: rule.level.as_str().to_owned(),
            granted_by: rule.granted_by.map(|id| id.to_string()),
            granted_at: granted_at_of(rule.granted_at),
        }
    }
}

/// One rule an item inherits from a folder above it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InheritedRuleWire {
    /// The folder the rule sits on.
    pub folder_id: String,
    /// The folder's name.
    pub folder_name: String,
    /// The person the rule names; `None` when the rule names a role.
    pub user_id: Option<String>,
    /// The role the rule names; `None` when the rule names a person.
    pub role_id: Option<String>,
    /// The name of the person or the role the rule names; `None` (sent as
    /// `null`) when the grantee is not found in the caller's organisation.
    pub grantee_name: Option<String>,
    /// The email of the person the rule names; `None` (sent as `null`) for
    /// a role, and for a person not found in the caller's organisation.
    pub grantee_email: Option<String>,
    /// What the rule gives: "view", "edit" or "manage".
    pub level: String,
}

impl InheritedRuleWire {
    /// The wire shape of a rule inherited from the folder `folder_id`, with
    /// its grantee named from `names`.
    ///
    /// The folder's name is read from the same map the chain came from,
    /// so the list names the folder the way the tree does.
    fn of(rule: &ItemRule, folder_id: Uuid, map: &FolderMap, names: &GranteeNames) -> Self {
        let (grantee_name, grantee_email) = names.of(rule);
        Self {
            folder_id: folder_id.to_string(),
            folder_name: map
                .get(folder_id)
                .map_or_else(String::new, |folder| folder.name.clone()),
            user_id: rule.user_id.map(|id| id.to_string()),
            role_id: rule.role_id.map(|id| id.to_string()),
            grantee_name,
            grantee_email,
            level: rule.level.as_str().to_owned(),
        }
    }
}

/// The names of the people and roles an access list's rules name, read in
/// one lookup for the whole list ([`saved_query_access::grantee_labels`]).
///
/// A grantee the lookup did not find is not here, so its rule is sent with
/// `null` for the name and the email: the list never makes a name up.
struct GranteeNames {
    /// The people found in the caller's tenant, by id.
    people: HashMap<Uuid, GranteePerson>,
    /// The roles found, by id.
    roles: HashMap<Uuid, GranteeRole>,
}

impl GranteeNames {
    /// The name and the email of the grantee `rule` names: both for a person
    /// found, the name alone for a role found, neither otherwise.
    fn of(&self, rule: &ItemRule) -> (Option<String>, Option<String>) {
        if let Some(person) = rule.user_id.and_then(|id| self.people.get(&id)) {
            // ADM-11 D1: a person's email is itself optional now, so the
            // clone carries the option, not a nested one.
            return (Some(person.name.clone()), person.email.clone());
        }
        if let Some(role) = rule.role_id.and_then(|id| self.roles.get(&id)) {
            return (Some(role.name.clone()), None);
        }
        (None, None)
    }
}

/// The access list of one item: its own rules, the rules it inherits from
/// the folders above it, and the level the caller holds on it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessListWire {
    /// The kind of item: "query" or "folder".
    pub item_type: String,
    /// The item's id.
    pub item_id: String,
    /// The level the caller holds on the item: "view", "edit" or "manage".
    pub your_level: String,
    /// The item's own rules.
    pub rules: Vec<AccessRuleWire>,
    /// The rules of the folders above the item, the nearest folder first.
    pub inherited: Vec<InheritedRuleWire>,
}

/// One person a saved query may be shared with.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GranteePersonWire {
    /// The person's id.
    pub id: String,
    /// The person's display name.
    pub name: String,
    /// The person's email address; `None` when the account has none
    /// (ADM-11 D1), sent as `null`.
    pub email: Option<String>,
}

impl GranteePersonWire {
    /// The wire shape of a person the caller may share with.
    fn of(person: &GranteePerson) -> Self {
        Self {
            id: person.id.to_string(),
            name: person.name.clone(),
            email: person.email.clone(),
        }
    }
}

/// One role a saved query may be shared with.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GranteeRoleWire {
    /// The role's id.
    pub id: String,
    /// The role's name.
    pub name: String,
}

impl GranteeRoleWire {
    /// The wire shape of a role the caller may share with.
    fn of(role: &GranteeRole) -> Self {
        Self {
            id: role.id.to_string(),
            name: role.name.clone(),
        }
    }
}

/// The people and the roles a saved query may be shared with.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GranteeListWire {
    /// The people of the caller's organisation.
    pub people: Vec<GranteePersonWire>,
    /// The roles of the deployment.
    pub roles: Vec<GranteeRoleWire>,
}

/// One rule of a `PUT …/access` body.
///
/// The grantees are ids, and an id that is not a uuid is a body of
/// another shape rather than a rule of another shape: the sentence that
/// says so names the shape of the body, and the sentence for a rule
/// names the rule.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessRuleInput {
    /// The person the rule names.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// The role the rule names.
    #[serde(default)]
    pub role_id: Option<Uuid>,
    /// What the rule gives: "view", "edit" or "manage".
    #[serde(default)]
    pub level: String,
}

/// The `PUT …/access` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PutAccessBody {
    /// The rules the item shall carry.
    pub rules: Vec<AccessRuleInput>,
}

/// The `GET /api/query/grantees` query string.
///
/// The search text stays a raw string on purpose: the store matches it as
/// a literal substring, so an empty search box shows the first people
/// rather than nothing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GranteeQuery {
    /// The text typed in the search box: it matches a name or an email.
    #[serde(default)]
    pub q: Option<String>,
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

/// The folder and the level the caller holds on it, or a 404 when the
/// folder is not theirs to see.
///
/// [`folder_level`] answers with the level alone; this returns the folder
/// with it, for the handlers that need the folder itself — where it sits
/// now, and whether it is a personal space — so they ask the database
/// once, not twice.
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see; 500 on a database failure.
async fn folder_with_level(
    pool: &PgPool,
    viewer: &queries::SavedQueryViewer,
    id: Uuid,
) -> Result<(FolderNode, Level), ApiError> {
    let folders = saved_query_access::folders_with_levels(pool, viewer).await?;
    folders
        .into_iter()
        .find(|(folder, _level)| folder.id == id)
        .ok_or_else(|| ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()))
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
    folder_with_level(pool, viewer, id)
        .await
        .map(|(_folder, level)| level)
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
        FolderError::MoveNeedsManage => ApiError::PermissionDenied(MOVE_NEEDS_MANAGE.to_owned()),
        FolderError::Database(store_error) => {
            // A failure of the database is logged under a reference id and
            // answers with the fixed message and that id, at 503 when the
            // database cannot be reached (AGENTS.md principle 4; `QS-3`
            // review of #382, NOTE 7). The store's own fixed sentences
            // (a store with no pool, a row that is not there) keep their
            // own status through `ApiError::from`.
            match upstream_error::store(&upstream_error::STORE, &store_error) {
                upstream_error::Reported::Upstream(failure) => {
                    failure.into_api_error(upstream_error::FailedAs::Internal)
                }
                upstream_error::Reported::Own(_fixed) => ApiError::from(store_error),
            }
        }
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

/// The moment a rule was written, as the wire carries it: ISO 8601 in
/// UTC, to the second.
///
/// The second is the resolution the list shows: a rule is written once,
/// and the console shows how long ago that was.
fn granted_at_of(granted_at: OffsetDateTime) -> String {
    granted_at
        .replace_nanosecond(0)
        .unwrap_or(granted_at)
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// The item an access list belongs to: one saved query or one folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessItem {
    /// One saved query.
    Query {
        /// The query's id.
        id: Uuid,
        /// The folder the query sits in; `None` for the shared space.
        folder_id: Option<Uuid>,
    },
    /// One folder.
    Folder {
        /// The folder's id.
        id: Uuid,
    },
}

impl AccessItem {
    /// The item's id.
    fn id(self) -> Uuid {
        match self {
            Self::Query { id, .. } | Self::Folder { id } => id,
        }
    }

    /// The kind of item, as the wire names it.
    fn item_type(self) -> &'static str {
        match self {
            Self::Query { .. } => "query",
            Self::Folder { .. } => "folder",
        }
    }

    /// The query the rules sit on, when the item is a query.
    fn query_id(self) -> Option<Uuid> {
        match self {
            Self::Query { id, .. } => Some(id),
            Self::Folder { .. } => None,
        }
    }

    /// The folder the rules sit on, when the item is a folder.
    fn folder_id(self) -> Option<Uuid> {
        match self {
            Self::Query { .. } => None,
            Self::Folder { id } => Some(id),
        }
    }
}

/// The folders above the item, nearest first: the chain the item
/// inherits its rules from.
///
/// A query inherits from the folder it sits in and from that folder's
/// parents; a folder inherits from its parents only, because its own
/// rules are its own and sit in the list beside them.
///
/// A personal space is left out of the chain: the resolver ignores a rule
/// stored on one (`resolve_level`, `QS-3` review of #382), so listing it
/// would show access that grants nothing (`QS-3` review of #382, follow-up).
///
/// A chain that cannot be built is corrupt data, and corrupt data
/// inherits nothing.
fn chain_of(map: &FolderMap, item: AccessItem) -> Vec<Uuid> {
    let folder_id = match item {
        AccessItem::Query { folder_id, .. } => folder_id,
        AccessItem::Folder { id } => Some(id),
    };
    let Some(folder_id) = folder_id else {
        // A query in the shared space sits at the root: it inherits
        // nothing.
        return Vec::new();
    };
    let Some(chain) = map.chain(folder_id) else {
        return Vec::new();
    };
    // The folder's own rules are its own: only its parents are inherited.
    // The folder itself is skipped before the personal spaces are filtered,
    // so the chain's first folder is never mistaken for a parent.
    let skip = match item {
        AccessItem::Folder { .. } => 1,
        AccessItem::Query { .. } => 0,
    };
    chain
        .iter()
        .skip(skip)
        .filter(|folder| !folder.is_personal)
        .map(|folder| folder.id)
        .collect()
}

/// The access list of one item: its own rules, the rules it inherits
/// from the folders above it, and the level the caller holds on it.
///
/// The folders are read with the viewer's own tenant, so the chain is the
/// chain the resolver used to decide the level. The list names a folder
/// only when the resolver reads its rules: a rule stored on a personal
/// space grants nothing, so it is listed neither as inherited nor as the
/// space's own (`QS-3` review of #382, follow-up).
async fn access_list(
    pool: &PgPool,
    viewer: &queries::SavedQueryViewer,
    item: AccessItem,
    your_level: Level,
) -> Result<AccessListWire, ApiError> {
    let folders = saved_query_access::list_folders(pool, viewer.tenant_id).await?;
    let map = FolderMap::new(folders);
    let chain = chain_of(&map, item);
    // The resolver ignores a rule stored on a personal space, so the space
    // lists none of its own.
    let own = if item
        .folder_id()
        .and_then(|id| map.get(id))
        .is_some_and(|folder| folder.is_personal)
    {
        Vec::new()
    } else {
        saved_query_access::rules_for_item(pool, item.query_id(), item.folder_id()).await?
    };
    let inherited = saved_query_access::rules_for_folders(pool, &chain).await?;
    // One lookup names the grantees of the own and the inherited rules
    // together, whatever their number: a list is never a query per rule.
    let mut user_ids: Vec<Uuid> = own
        .iter()
        .chain(&inherited)
        .filter_map(|rule| rule.user_id)
        .collect();
    let mut role_ids: Vec<Uuid> = own
        .iter()
        .chain(&inherited)
        .filter_map(|rule| rule.role_id)
        .collect();
    user_ids.sort_unstable();
    user_ids.dedup();
    role_ids.sort_unstable();
    role_ids.dedup();
    let labels =
        saved_query_access::grantee_labels(pool, viewer.tenant_id, &user_ids, &role_ids).await?;
    let names = GranteeNames {
        people: labels
            .people
            .into_iter()
            .map(|person| (person.id, person))
            .collect(),
        roles: labels
            .roles
            .into_iter()
            .map(|role| (role.id, role))
            .collect(),
    };
    Ok(AccessListWire {
        item_type: item.item_type().to_owned(),
        item_id: item.id().to_string(),
        your_level: your_level.as_str().to_owned(),
        rules: own
            .iter()
            .map(|rule| AccessRuleWire::of(rule, &names))
            .collect(),
        inherited: inherited
            .iter()
            .filter_map(|rule| {
                rule.folder_id
                    .map(|folder_id| InheritedRuleWire::of(rule, folder_id, &map, &names))
            })
            .collect(),
    })
}

/// The rules as they will be stored: exactly one grantee each, a level
/// of the set, and nobody named twice.
///
/// The grantee is keyed by its kind as well as by its id: a person and a
/// role are two different grantees, even when they carry the same id.
fn checked_rules(input: &[AccessRuleInput]) -> Result<Vec<NewGrant>, ApiError> {
    let mut seen: HashSet<(bool, Uuid)> = HashSet::new();
    let mut rules = Vec::with_capacity(input.len());
    for rule in input {
        let level = Level::parse(rule.level.trim())
            .ok_or_else(|| ApiError::BadRequest(ACCESS_RULE_SHAPE.to_owned()))?;
        let key = match (rule.user_id, rule.role_id) {
            (Some(user_id), None) => (true, user_id),
            (None, Some(role_id)) => (false, role_id),
            _ => return Err(ApiError::BadRequest(ACCESS_RULE_SHAPE.to_owned())),
        };
        if !seen.insert(key) {
            return Err(ApiError::BadRequest(ACCESS_GRANTEE_TWICE.to_owned()));
        }
        rules.push(NewGrant {
            user_id: rule.user_id,
            role_id: rule.role_id,
            level,
        });
    }
    Ok(rules)
}

/// The grantees of the rules, checked against the caller's organisation.
///
/// The check happens before anything is written, so a rule that names
/// somebody outside the organisation leaves the old rules exactly as
/// they were.
async fn check_grantees(
    pool: &PgPool,
    viewer: &queries::SavedQueryViewer,
    rules: &[NewGrant],
) -> Result<(), ApiError> {
    let wanted_people: Vec<Uuid> = rules.iter().filter_map(|rule| rule.user_id).collect();
    let wanted_roles: Vec<Uuid> = rules.iter().filter_map(|rule| rule.role_id).collect();
    if wanted_people.is_empty() && wanted_roles.is_empty() {
        return Ok(());
    }
    let found_people =
        saved_query_access::tenant_user_ids(pool, viewer.tenant_id, &wanted_people).await?;
    let found_roles = saved_query_access::existing_role_ids(pool, &wanted_roles).await?;
    if wanted_people.iter().any(|id| !found_people.contains(id))
        || wanted_roles.iter().any(|id| !found_roles.contains(id))
    {
        return Err(ApiError::BadRequest(ACCESS_GRANTEE_OUTSIDE.to_owned()));
    }
    Ok(())
}

/// The route's own answer to a write of the rules the store refused.
///
/// The store says *what* was wrong; this says *what to tell the caller*.
/// The database's own text never reaches a response (`AGENTS.md`
/// principle 4).
fn access_failure(failure: StoreError) -> ApiError {
    match failure {
        // A rule that names a person or a role the database does not
        // hold is the organisation's own refusal.
        StoreError::ForeignKeyViolation => ApiError::BadRequest(ACCESS_GRANTEE_OUTSIDE.to_owned()),
        // A rule that names two grantees or none: the shape of a rule.
        StoreError::Validation(_message) => ApiError::BadRequest(ACCESS_RULE_SHAPE.to_owned()),
        other => ApiError::from(other),
    }
}

/// The folder a saved query sits in, when the row names one.
///
/// A row that names a folder the database cannot hold is corrupt data,
/// and corrupt data is not found: the same 404 as a query that does not
/// exist.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`].
fn saved_folder_id(saved: &SavedQuery) -> Result<Option<Uuid>, ApiError> {
    match saved.folder_id.as_deref() {
        Some(raw) => Uuid::parse_str(raw)
            .map(Some)
            .map_err(|_err| ApiError::NotFound(QUERY_NOT_FOUND.to_owned())),
        None => Ok(None),
    }
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
    /// New parameter definitions (`QS-5`), the whole list; absent leaves
    /// them as they are, `[]` clears them. Raw JSON so a list of the wrong
    /// shape is refused with a fixed sentence.
    #[serde(default)]
    pub parameters: Option<Value>,
}

/// `PATCH /api/query/saved/{id}` — change a saved query.
///
/// The caller must hold **edit** on the query: `edit` changes the SQL, the
/// title and the tags, `view` may change nothing. A move to another folder,
/// or to the shared root, also needs **manage** on the query and edit on
/// the target folder: a move changes who sees the query, so edit alone does
/// not move it. The store asks the manage level before any write, so a
/// refused PATCH writes nothing, not the title or the SQL in the same body
/// (`QS-3` review of #382), and the route holds no copy of the rule. A
/// statement that is not read-only is refused the same way the save route
/// refuses it, so nothing that can never run is ever stored.
///
/// A change of the SQL or of the parameters is checked as a pair
/// (`QS-5`): the definitions of the body, or the stored ones, against the
/// SQL of the body, or the stored SQL. A patch that touches neither does not
/// read the SQL again, so a query saved before `QS-5` still renames. A
/// change of the parameters alone writes a version, like a change of the
/// SQL.
///
/// # Errors
///
/// 400 for a malformed body, a blank title or parameters that do not fit
/// the SQL (the sentence of [`crate::query_params::ParamError`], or the
/// fixed shape sentence for a list of the wrong shape); 404 with the fixed sentence
/// of [`QUERY_NOT_FOUND`] for a query the caller may not see; 403 with the
/// fixed sentence of [`QUERY_FORBIDDEN`] for a query the caller may see
/// but not change, and for a folder the caller may see but not fill; 403
/// with the fixed sentence of [`MOVE_NEEDS_MANAGE`] for a move the caller
/// does not manage the query for; 422 with the fixed sentence of
/// [`NOT_READ_ONLY`] for a statement that is not read-only; 500 on a
/// database failure.
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
        ApiError::BadRequest(
            "Body must be JSON {title?, sql?, tags?, folderId?, parameters?}.".to_owned(),
        )
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
    // The SQL and the definitions travel as a pair: when either changes,
    // the pair the query will hold is checked.
    let parameters = if sql.is_some() || body.parameters.is_some() {
        let final_sql = sql.as_deref().unwrap_or(&saved.sql);
        let final_definitions = body.parameters.unwrap_or_else(|| saved.parameters.clone());
        Some(checked_saved_definitions(
            final_sql,
            Some(final_definitions),
        )?)
    } else {
        None
    };
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
    // A move needs manage on the query, and the store asks that before it
    // writes anything (`QS-3` review of #382): the route holds no second
    // copy of the rule, so a refused PATCH writes nothing and the rule has
    // one owner. The level checks above and on the target folder stay here.
    let patch = SavedQueryPatch {
        title,
        sql,
        tags: body.tags,
        folder,
        parameters,
    };
    let saved = queries::update_saved_query(pool(&state)?, &viewer, &id, &patch)
        .await
        .map_err(query_failure)?;
    Ok(ApiJson(saved))
}

/// `DELETE /api/query/saved/{id}` — move a saved query to the trash.
///
/// The caller must hold **edit** on the query: decision 6 (plan 55 D5)
/// puts deleting a query beside changing it, because a person who may
/// rewrite its SQL may take it to the trash too. The query waits in the
/// trash until it is restored or swept away; it is not gone yet.
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
    // Edit may delete a query (decision 6, plan 55 D5); the store
    // re-checks the same level on the write.
    if level_of(&saved.level) < Level::Edit {
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
    if scope.effective_viewer().is_none() {
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    }
    let source = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    // The copy stores the same statement, so it meets the same guard.
    if !is_read_only(&source.sql) {
        return Err(ApiError::Unprocessable(NOT_READ_ONLY.to_owned()).into());
    }
    // The copy is a new row, so it belongs to the tenant the request acts
    // as — not to the reach the read above used. The reach of a
    // deployment with a single tenant names no tenant at all, and a row
    // that carries no tenant has no personal space to land in: it lands
    // in the shared space, where the caller's own work is nobody's
    // private work. The tenant the request acts as is the one rule that
    // says which tenant a request is for (`crate::tenant_scope::resolve`),
    // the same rule the save of a query and the assistant's save use.
    let viewer = saved_query_viewer(
        &principal,
        crate::tenant_scope::resolve(&principal, &headers)?,
    );
    let owner = principal.display_name.clone();
    let copy = queries::create_saved_query_with_parameters(
        pool(&state)?,
        &viewer,
        &source.title,
        &source.sql,
        &source.parameters,
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
/// **edit** on it: whoever may delete a query may bring it back
/// (decision 6, plan 55 D5).
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
    // A trashed query is found with the same scope and the same resolver
    // the trash list uses: what the list shows is what restore acts on.
    let saved = queries::get_saved_including_trash(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    // Edit may restore a query (decision 6, plan 55 D5); the store
    // re-checks the same level on the write.
    if level_of(&saved.level) < Level::Edit {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    let restored = queries::restore_saved_query(pool(&state)?, &viewer, &id)
        .await
        .map_err(query_failure)?;
    Ok(ApiJson(restored))
}

/// `GET /api/query/saved-trash` — the queries waiting in the trash.
///
/// The trash holds what the caller may **edit** (decision 6, plan 55
/// D5): a query they may delete and restore is theirs to see waiting,
/// and a caller that may only view a query has no business seeing where
/// it waits.
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
/// The caller must hold **edit** on the folder to rename it or to move it,
/// and edit on the folder it moves under. A move also needs **manage** on
/// the folder and on every folder and query inside it, the ones in the
/// trash included: it changes which rules cover all of them, so the store
/// asks it before it writes (`QS-3` review of #382) and the route holds no
/// copy of that rule. A **personal** space may be neither renamed nor
/// moved — it is one person's own space, and moving it would move their
/// private queries with it.
///
/// A body that carries both a name and a parent moves the folder first and
/// renames it second, after the route has checked the name with the
/// store's placement rule at the folder's new place. Everything that can
/// refuse is settled before the first write, so a refused PATCH changes
/// nothing (`QS-3` review of #382, SHOULD-FIX 2). There is no transaction
/// across the two writes, so two things between them leave the folder moved
/// and not renamed: a database failure, and a concurrent request that
/// creates or renames a sibling to the same name, which makes the rename
/// answer 409 (`QS-3` review of #382, follow-up). The store judges the move by the
/// folder's current name, so a body that moves a folder under a sibling of
/// its current name and renames it to a free one at the same time is
/// refused as a duplicate; two requests do it. The answer is the folder as
/// it is after both writes.
///
/// # Errors
///
/// 400 for a malformed body, for a bad name, for a tree that would grow
/// too deep and for a move that would make the folder its own ancestor;
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see and for a parent that does not exist; 403 with the
/// fixed sentence of [`FOLDER_FORBIDDEN`] for a folder the caller may see
/// but not change; 403 with the fixed sentence of [`MOVE_NEEDS_MANAGE`]
/// for a move of something the caller does not manage, and of anything
/// inside it; 409 with the fixed sentence of [`FOLDER_DUPLICATE_NAME`] for
/// a name a sibling already carries; 500, or 503 when the database cannot
/// be reached, with a reference id on a database failure.
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
    // The node is kept, not only the level: the name check reads where the
    // folder sits now, and this is the same read the level check needs
    // anyway.
    let (current, level) = folder_with_level(pool(&state)?, &viewer, folder_id).await?;
    if level < Level::Edit {
        return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
    }
    // The body is read only now: a folder the caller may not see is a
    // 404 and a folder it may not change is a 403, whatever the body
    // holds.
    let body: PatchFolderBody = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest("Body must be JSON {name?, parentId?}.".to_owned()))?;
    // Everything that can refuse is settled before the first write, so a
    // refused PATCH changes nothing (`QS-3` review of #382, SHOULD-FIX 2).
    // The parent comes first: it needs edit on the target folder.
    let moves = !matches!(body.parent_id, FolderField::Absent);
    let new_parent = if moves {
        match parse_folder_reference(body.parent_id)? {
            FolderReference::Absent | FolderReference::SharedSpace => None,
            FolderReference::Folder(parent_id) => {
                let level = folder_level(pool(&state)?, &viewer, parent_id).await?;
                if level < Level::Edit {
                    return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
                }
                Some(parent_id)
            }
        }
    } else {
        None
    };
    if body.name.is_none() && !moves {
        return Err(ApiError::BadRequest("nothing to change.".to_owned()).into());
    }
    // The name is checked with the store's own placement rule, at the place
    // the folder will be in: the move may be allowed while the name is
    // refused there, and the folder must not be left moved by then. A
    // personal space is skipped here: the store refuses it with its own
    // answer on either write.
    if let Some(name) = body.name.as_deref()
        && !current.is_personal
    {
        let folders =
            saved_query_access::list_folders(pool(&state)?, Some(current.tenant_id)).await?;
        let place = if moves { new_parent } else { current.parent_id };
        saved_query_access::check_placement(&folders, Some(folder_id), name, place)
            .map_err(|rule| folder_failure(FolderError::from(rule)))?;
    }
    // The move goes first: the store asks manage on the folder and on
    // everything in it before it writes, and a refusal there leaves the
    // name as it was. The rename after it was checked above.
    let mut folder = None;
    if moves {
        folder = Some(
            saved_query_access::move_folder_for_viewer(
                pool(&state)?,
                &viewer,
                folder_id,
                new_parent,
            )
            .await
            .map_err(folder_failure)?,
        );
    }
    if let Some(name) = body.name.as_deref() {
        folder = Some(
            saved_query_access::rename_folder(pool(&state)?, folder_id, name)
                .await
                .map_err(folder_failure)?,
        );
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
/// Manage, not edit: decision 6 lifted only a query's delete and
/// restore to edit, and a folder is not a query (plan 55 D5).
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

// ── The access routes ────────────────────────────────────────────────────
//
// The sharing surface: who has what on a saved query or on a folder, and
// who may be given a rule at all. Every route here needs **manage** on
// the item, and every route reads the item first — a 404 for what the
// caller may not see, a 403 for what it may see but not manage, and only
// then the body. Manage stays the bar here even though decision 6
// lifted a query's delete and restore to edit (plan 55 D5): sharing is
// not changing, and edit must not reach it.

/// `GET /api/query/saved/{id}/access` — who has what on one saved query.
///
/// The list needs **manage**: managing a query includes sharing it, and a
/// person who may only see a query has no business seeing who else sees
/// it.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see; 403 with the fixed sentence of [`QUERY_FORBIDDEN`]
/// for a query the caller may see but not manage; 500 on a database
/// failure.
pub async fn get_query_access(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<AccessListWire>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        // A caller that acts for no organisation owns no saved query.
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    let saved = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    let level = level_of(&saved.level);
    if level < Level::Manage {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    let item = AccessItem::Query {
        id: query_id_of(&saved.id)?,
        folder_id: saved_folder_id(&saved)?,
    };
    Ok(ApiJson(
        access_list(pool(&state)?, &viewer, item, level).await?,
    ))
}

/// `PUT /api/query/saved/{id}/access` — replace the rules on one saved
/// query.
///
/// The answer is the list as it is after the write, so the console can
/// draw it again without a second call. The level it carries is the
/// level the change was authorised with — the one the caller holds
/// before the write — because a rule the caller writes for somebody
/// else cannot change the caller's own level.
///
/// # Errors
///
/// 404 with the fixed sentence of [`QUERY_NOT_FOUND`] for a query the
/// caller may not see; 403 with the fixed sentence of [`QUERY_FORBIDDEN`]
/// for a query the caller may see but not manage; 400 for a body of
/// another shape, for a rule that names two grantees or none, for a
/// grantee that is listed twice and for a grantee that is not of the
/// caller's organisation; 500 on a database failure.
pub async fn put_query_access(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<AccessListWire>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(QUERY_NOT_FOUND.to_owned()).into());
    };
    let saved = queries::get_saved(pool(&state)?, &id, &scope)
        .await?
        .ok_or_else(|| ApiError::NotFound(QUERY_NOT_FOUND.to_owned()))?;
    let level = level_of(&saved.level);
    if level < Level::Manage {
        return Err(ApiError::PermissionDenied(QUERY_FORBIDDEN.to_owned()).into());
    }
    // The body is read only now: a query the caller may not see is a 404
    // and a query it may not manage is a 403, whatever the body holds.
    let body: PutAccessBody = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest(ACCESS_BODY_SHAPE.to_owned()))?;
    let rules = checked_rules(&body.rules)?;
    let item = AccessItem::Query {
        id: query_id_of(&saved.id)?,
        folder_id: saved_folder_id(&saved)?,
    };
    // Every grantee is checked before anything is written, so a refused
    // change leaves the old rules exactly as they were.
    check_grantees(pool(&state)?, &viewer, &rules).await?;
    saved_query_access::replace_grants(
        pool(&state)?,
        item.query_id(),
        item.folder_id(),
        &rules,
        viewer.id,
    )
    .await
    .map_err(access_failure)?;
    Ok(ApiJson(
        access_list(pool(&state)?, &viewer, item, level).await?,
    ))
}

/// `GET /api/query/folders/{id}/access` — who has what on one folder.
///
/// A rule on a folder covers everything inside it, so the list of a
/// folder is the list of the space it holds.
///
/// The list is read from edit up and changed with manage
/// ([`put_folder_access`]): a person who may save or move a query into a
/// folder is told who that folder is shared with before they do (`QS-3`,
/// folder sharing warning).
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see, for an id that is not a UUID and for a caller that
/// acts for no organisation; 404 for an `X-Tenant` header that names a
/// tenant the caller does not belong to; 403 with the fixed sentence of
/// [`FOLDER_ACCESS_FORBIDDEN`] — "Only someone who can edit this folder can
/// see who it is shared with." — for a folder the caller may see but only
/// view; 503 when no database pool is configured, which `pool` raises
/// before any store call, or when the tenant scope cannot verify the
/// deployment's tenant count; 500 with the fixed sentence `database error`
/// on a database failure inside a store call, through
/// `From<StoreError> for ApiError`. This route does not use
/// `folder_failure`, which only the writes use.
pub async fn get_folder_access(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<AccessListWire>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        // A caller that acts for no organisation owns no folder.
        return Err(ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()).into());
    };
    let folder_id = folder_id_of(&id)?;
    let level = folder_level(pool(&state)?, &viewer, folder_id).await?;
    // Edit reads the list, manage changes it (`QS-3`, folder sharing
    // warning): a person who may save or move a query into a folder is
    // told who that folder is shared with before they do. The refusal
    // names the level the list is read at, not a change the caller never
    // asked for (`#416`).
    if level < Level::Edit {
        return Err(ApiError::PermissionDenied(FOLDER_ACCESS_FORBIDDEN.to_owned()).into());
    }
    let item = AccessItem::Folder { id: folder_id };
    Ok(ApiJson(
        access_list(pool(&state)?, &viewer, item, level).await?,
    ))
}

/// `PUT /api/query/folders/{id}/access` — replace the rules on one
/// folder.
///
/// The answer is the list as it is after the write, so the console can
/// draw it again without a second call. The level it carries is the
/// level the change was authorised with — the one the caller holds
/// before the write.
///
/// # Errors
///
/// 404 with the fixed sentence of [`FOLDER_NOT_FOUND`] for a folder the
/// caller may not see; 403 with the fixed sentence of [`FOLDER_FORBIDDEN`]
/// for a folder the caller may see but not manage; 403 with the fixed
/// sentence of [`PERSONAL_SPACE_NOT_SHARED`] for a personal space; 403 with
/// the fixed sentence of [`SHARE_NEEDS_MANAGE_OF_CONTENTS`] for a folder
/// whose sub-folders or queries the caller does not all manage; 400 for
/// a body of another shape, for a rule that names two grantees or none,
/// for a grantee that is listed twice and for a grantee that is not of the
/// caller's organisation; 503 with a reference id when the database cannot
/// be reached, which `folder_failure` answers for the check of what the
/// caller manages in the folder; 500 on any other database failure.
pub async fn put_folder_access(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<AccessListWire>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        return Err(ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()).into());
    };
    let folder_id = folder_id_of(&id)?;
    let (folder, level) = folder_with_level(pool(&state)?, &viewer, folder_id).await?;
    if level < Level::Manage {
        return Err(ApiError::PermissionDenied(FOLDER_FORBIDDEN.to_owned()).into());
    }
    // A personal space is shared through a query or a folder inside it,
    // never as a whole (`QS-3` review of #382). Asked after the 404 and the
    // manage check, so a person who may not see the space learns nothing.
    if folder.is_personal {
        return Err(ApiError::PermissionDenied(PERSONAL_SPACE_NOT_SHARED.to_owned()).into());
    }
    // A rule on a folder covers what is inside it, so the caller manages
    // the folder and everything under it, the trash included, or shares
    // nothing: a PUT that only removes rules is a share too (`QS-3` review
    // of #393). Asked before the body is read, so a refused PUT writes
    // nothing.
    if !saved_query_access::manages_folder_and_contents(pool(&state)?, &viewer, folder_id)
        .await
        .map_err(folder_failure)?
    {
        return Err(ApiError::PermissionDenied(SHARE_NEEDS_MANAGE_OF_CONTENTS.to_owned()).into());
    }
    // The body is read only now: a folder the caller may not see is a
    // 404 and a folder it may not manage is a 403, whatever the body
    // holds.
    let body: PutAccessBody = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest(ACCESS_BODY_SHAPE.to_owned()))?;
    let rules = checked_rules(&body.rules)?;
    let item = AccessItem::Folder { id: folder_id };
    // Every grantee is checked before anything is written, so a refused
    // change leaves the old rules exactly as they were.
    check_grantees(pool(&state)?, &viewer, &rules).await?;
    saved_query_access::replace_grants(
        pool(&state)?,
        item.query_id(),
        item.folder_id(),
        &rules,
        viewer.id,
    )
    .await
    .map_err(access_failure)?;
    Ok(ApiJson(
        access_list(pool(&state)?, &viewer, item, level).await?,
    ))
}

/// `GET /api/query/grantees?q=` — the people and the roles the caller
/// may share with.
///
/// The people are the members of the caller's organisation, and the roles
/// are the roles of the deployment: a role belongs to the deployment
/// rather than to an organisation, so every role of it is a possible
/// grantee. A caller that acts for no organisation has nobody to share
/// with, and the answer is empty rather than everybody's.
///
/// # Errors
///
/// 400 for a query string the route cannot read; 500 on a database
/// failure.
pub async fn search_grantees(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    query: Result<Query<GranteeQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<ApiJson<GranteeListWire>> {
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    let Some(viewer) = scope.effective_viewer() else {
        // A caller that acts for no organisation has nobody to share
        // with.
        return Ok(ApiJson(GranteeListWire {
            people: Vec::new(),
            roles: Vec::new(),
        }));
    };
    // A query string the extractor cannot read — a parameter sent twice,
    // for instance — is refused with this surface's own sentence. The
    // rejection's own text is serde's, and it stays in the log: a
    // response never carries a library's rendering of the caller's
    // request.
    let Query(raw) = query.map_err(|rejection| {
        tracing::warn!(%rejection, "a grantee query string could not be read");
        ApiError::BadRequest(QUERY_STRING_UNREADABLE.to_owned())
    })?;
    let needle = raw.q.as_deref().unwrap_or_default();
    let found =
        saved_query_access::search_grantees(pool(&state)?, viewer.tenant_id, needle, GRANTEE_LIMIT)
            .await?;
    Ok(ApiJson(GranteeListWire {
        people: found.people.iter().map(GranteePersonWire::of).collect(),
        roles: found.roles.iter().map(GranteeRoleWire::of).collect(),
    }))
}

// ── The catalog's "used by" ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! The catalog's "used by" list is a reader of the saved queries like
    //! any other, so the personal space has to be a wall for it too: a
    //! personal query of one person is never in the answer about another.
    //!
    //! The test lives HERE, and not in `routes/catalog_governance.rs`,
    //! because that file is held by an open pull request and this step
    //! does not touch it. `dependents` is `pub(crate)`, so a unit test in
    //! this crate can call it directly — and the catalog's own detail
    //! route needs `ClickHouse`, which this test does not.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
    use lakehouse_store::PgPool;
    use lakehouse_store::queries::{
        SavedQueryPlacement, SavedQueryViewer, create_saved_query, create_saved_query_at,
    };
    use uuid::Uuid;

    use crate::config::Config;
    use crate::state::AppState;

    // Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
    // testcontainer bootstrap runs for this `--lib` test binary, the same
    // reason `routes/ai/tools/queries`'s test module does it.
    use lakehouse_test_support as _;

    /// The table the two queries of the fixture read.
    const TABLE: &str = "silver.orders";

    /// A `DATABASE_URL` string dialing the SAME per-test Postgres database
    /// `#[sqlx::test]` handed us: `AppState::new` takes
    /// `Config::database_url`, not a pre-built pool.
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

    /// The author of the two queries: the seeded Analyst of Meridian
    /// Group.
    fn rina() -> Uuid {
        Uuid::parse_str("33333333-3333-4333-8333-000000000001").expect("the seeded user id")
    }

    /// The other person of the same organisation: the seeded Data
    /// Scientist, who acts for the same tenant.
    fn reza() -> Uuid {
        Uuid::parse_str("33333333-3333-4333-8333-000000000008").expect("the seeded user id")
    }

    /// Meridian Group, the tenant both of them act for.
    fn group_tenant() -> Uuid {
        Uuid::parse_str("11111111-1111-4111-8111-000000000001").expect("the group tenant id")
    }

    /// One query in the author's personal space and one in the shared
    /// space, both reading the same table: the pair every control needs.
    async fn two_queries_reading_one_table(pool: &PgPool) -> (String, String) {
        let viewer = SavedQueryViewer {
            id: Some(rina()),
            role_names: Vec::new(),
            tenant_id: Some(group_tenant()),
            all: false,
        };
        let personal = create_saved_query(
            pool,
            &viewer,
            "Rina's own",
            "SELECT * FROM silver.orders",
            "Rina Wijaya",
            &[],
            None,
        )
        .await
        .expect("the personal query is written");
        let shared = create_saved_query_at(
            pool,
            &viewer,
            "Everybody's",
            "SELECT * FROM silver.orders",
            "Rina Wijaya",
            &[],
            SavedQueryPlacement::SharedSpace,
        )
        .await
        .expect("the shared query is written");
        (personal.id, shared.id)
    }

    /// The catalog's "used by" list does not name another person's
    /// personal query.
    ///
    /// CONTROL: the shared space IS in the answer, so the test cannot pass
    /// by an empty list.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_catalogs_used_by_does_not_name_another_persons_personal_query(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let (personal_id, shared_id) = two_queries_reading_one_table(&pool).await;

        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        // The catalog's dashboards are not read here: the principal holds
        // no `dashboard:read`, so the answer is the saved queries only.
        env.insert("CH_URL".to_owned(), "http://127.0.0.1:1".to_owned());
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));

        let principal = Principal {
            id: PrincipalId::User(reza()),
            tenant_ids: vec![group_tenant()],
            display_name: "Reza Hakim".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };

        let listed =
            crate::routes::catalog_governance::dependents(&state, &principal, &[TABLE.to_owned()])
                .await;

        assert!(
            !listed.iter().any(|entry| entry["id"] == personal_id),
            "another person's personal query is not a dependent: {listed:?}"
        );
        assert!(
            listed.iter().any(|entry| entry["id"] == shared_id),
            "CONTROL: the shared space is a dependent: {listed:?}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod folder_failure_tests {
    //! How a refused folder write is answered: the fixed sentence of the
    //! move rule, and a database failure that carries a reference id and
    //! never the failure's own text.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_core::ApiError;
    use lakehouse_store::StoreError;
    use lakehouse_store::saved_query_access::FolderError;

    use super::{MOVE_NEEDS_MANAGE, folder_failure};

    /// The one sentence a refused move carries is written twice, here and
    /// in the store's `NotAllowed`; the two must read the same, so a query
    /// move and a folder move answer identically.
    #[test]
    fn the_move_sentence_is_the_same_for_a_query_and_for_a_folder() {
        assert_eq!(StoreError::NotAllowed.to_string(), MOVE_NEEDS_MANAGE);
        let folder = folder_failure(FolderError::MoveNeedsManage);
        assert_eq!(folder.status(), 403);
        assert_eq!(
            folder.to_string(),
            ApiError::from(StoreError::NotAllowed).to_string()
        );
        assert_eq!(
            folder.to_string(),
            format!("permission_denied: {MOVE_NEEDS_MANAGE}")
        );
    }

    /// A database failure answers with the fixed message and the reference
    /// the cause was logged under, and nothing of the cause itself.
    #[test]
    fn a_database_failure_answers_with_a_reference_and_not_its_text() {
        let cause = sqlx::Error::Protocol("relation saved_query_folder is broken".to_owned());
        let answer = folder_failure(FolderError::Database(StoreError::Database(cause)));
        assert_eq!(answer.status(), 500);
        let text = answer.to_string();
        assert!(text.contains("Reference: "), "{text}");
        assert!(!text.contains("saved_query_folder"), "{text}");
    }

    /// A pool that cannot be reached answers 503, like every other store
    /// route does, with a reference to find the log line by.
    #[test]
    fn an_unreachable_database_answers_503_with_a_reference() {
        let answer = folder_failure(FolderError::Database(StoreError::Database(
            sqlx::Error::PoolTimedOut,
        )));
        assert_eq!(answer.status(), 503);
        assert!(answer.to_string().contains("Reference: "), "{answer}");
    }

    /// No pool at all is the store's own 503, not a 500.
    #[test]
    fn a_store_without_a_pool_answers_503() {
        let answer = folder_failure(FolderError::Database(StoreError::Unavailable));
        assert_eq!(answer.status(), 503);
    }
}
