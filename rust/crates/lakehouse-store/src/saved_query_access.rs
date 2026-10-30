//! Saved-query access: the folders a saved query sits in, the rules that
//! say who may do what with it, and the one pure resolver that turns
//! "who is asking" into "what may they do".
//!
//! # Two spaces per organisation
//!
//! Every organisation has a **shared space** — the root, where a query
//! with no folder sits — that every member of the organisation sees at
//! `view`, and a **personal space** — one folder per person — that only
//! its owner sees, at `manage`. A rule is the only way anybody else sees
//! a query in a personal space.
//!
//! # One resolver
//!
//! [`resolve_level`] is the whole decision. It is pure: it takes the
//! item, the chain of folders above it, the rules that name the viewer
//! and the viewer, and it answers the level the viewer holds — or
//! `None`, which means "you may not see this at all". Every reader and
//! every writer goes through it, so a new read path cannot answer
//! "visible" without also answering "at what level".
//!
//! # The tenant is the outer boundary
//!
//! A rule names a person or a role, never a tenant: the `role` table
//! has no tenant column, and a person's tenants live in
//! `app_user_tenant`. A rule therefore only ever decides a level
//! *inside* the tenant whose row is being read. The tenant itself is
//! the scope every read is built with, and the resolver checks it again
//! so a row of another tenant is never answered at any level.
//!
//! # The tree rules live here too
//!
//! [`check_placement`] is the pure rule for a folder's name and its
//! place in the tree, so it can be unit-tested without a database: the
//! name is required and at most [`MAX_FOLDER_NAME_CHARS`] characters,
//! the parent must exist, a folder may not move into itself or into one
//! of its own descendants, the tree is at most [`MAX_FOLDER_DEPTH`]
//! levels deep, and a sibling's name is unique (compared
//! case-insensitively).
//!
//! # The writes
//!
//! [`create_folder`], [`rename_folder`], [`move_folder`] and
//! [`delete_folder`] are the four writes the tree needs. They all check
//! the placement with [`check_placement`] and answer with
//! [`FolderError`]: a typed answer, so the route layer can tell "the
//! folder still holds queries" from "the name is taken" and say two
//! different fixed sentences. The store says *what* was wrong; the route
//! says *what to tell the caller*, and the database's own text never
//! reaches a response.
//!
//! # The grantee search
//!
//! [`search_grantees`] answers the two lists the sharing route shows:
//! the people of the tenant and the roles of the deployment. A role
//! belongs to the deployment rather than to a tenant — the `role` table
//! has no tenant column — so every role is a possible grantee.

use std::collections::HashSet;

use sqlx::FromRow;
use thiserror::Error;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// The deepest a folder may sit: a folder at the root is depth 1.
pub const MAX_FOLDER_DEPTH: usize = 4;

/// The longest folder name accepted.
pub const MAX_FOLDER_NAME_CHARS: usize = 100;

/// How much of an item a person may do.
///
/// The order is the strength: `Manage` includes `Edit`, which includes
/// `View`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// See it and run it.
    View,
    /// Change its SQL, its title, its tags and the folder it sits in.
    Edit,
    /// Share it, delete it, restore it and see its entry in the trash.
    Manage,
}

impl Level {
    /// The word the database and the wire use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Edit => "edit",
            Self::Manage => "manage",
        }
    }

    /// The level a stored word names, `None` for a word outside the set:
    /// a rule that says anything else grants nothing at all.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            "manage" => Some(Self::Manage),
            _ => None,
        }
    }
}

/// Who is asking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedQueryViewer {
    /// The caller's user id. `None` for a service identity: it has no
    /// `app_user` row, so it is nobody's personal space's owner and
    /// nobody's query's author.
    pub id: Option<Uuid>,
    /// The roles the caller holds, by name, as the principal carries
    /// them (a rule may name a role).
    pub role_names: Vec<String>,
    /// The tenant whose rows are read. `None` when the read covers every
    /// tenant's rows (a holder of `"*:*"`, or a deployment that holds a
    /// single tenant).
    pub tenant_id: Option<Uuid>,
    /// The caller holds `"*:*"`: it manages everything of the
    /// organisation, personal spaces included.
    pub all: bool,
}

/// One stored rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGrant {
    /// The query the rule is on, when it is on a query.
    pub query_id: Option<Uuid>,
    /// The folder the rule is on, when it is on a folder. A rule on a
    /// folder covers everything inside it.
    pub folder_id: Option<Uuid>,
    /// The person the rule names.
    pub user_id: Option<Uuid>,
    /// The role the rule names.
    pub role_id: Option<Uuid>,
    /// What the rule gives.
    pub level: Level,
}

/// The item the resolver is asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessTarget {
    /// The item's id: a saved query's id, or a folder's id.
    pub id: Uuid,
    /// The item's author: the query's `owner_id`, or the folder's
    /// `owner_id`. `None` for a query nobody owns.
    pub owner_id: Option<Uuid>,
    /// The folder the item sits in. `None` for a query in the shared
    /// space.
    pub folder_id: Option<Uuid>,
    /// The tenant the item belongs to. `None` for a row that belongs to
    /// no tenant: those rows are the shared catalog's own, and which
    /// tenant reads them is the scope's business, not the resolver's.
    pub tenant_id: Option<Uuid>,
}

/// A folder in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderNode {
    /// The folder's id.
    pub id: Uuid,
    /// The folder this one sits in; `None` at the root.
    pub parent_id: Option<Uuid>,
    /// Whether this is a personal space: one person's own space, which
    /// only that person sees.
    pub is_personal: bool,
    /// The person the folder belongs to: the owner of a personal space,
    /// the creator of an ordinary folder.
    pub owner_id: Option<Uuid>,
    /// The tenant the folder belongs to.
    pub tenant_id: Uuid,
    /// The folder's name.
    pub name: String,
}

/// The folders of one tenant, and the chains they make.
#[derive(Debug, Clone, Default)]
pub struct FolderMap {
    folders: Vec<FolderNode>,
}

impl FolderMap {
    /// The map of these folders.
    #[must_use]
    pub fn new(folders: Vec<FolderNode>) -> Self {
        Self { folders }
    }

    /// The folder with this id, when the map holds it.
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&FolderNode> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// The chain of a folder, nearest first: the folder itself, then its
    /// parent, and so on to the root.
    ///
    /// `None` when the chain cannot be built: a parent that is not in
    /// the map, a chain that repeats a folder (corrupt data), or a
    /// chain deeper than [`MAX_FOLDER_DEPTH`]. The caller then knows no
    /// level at all, which is the fail-closed answer.
    #[must_use]
    pub fn chain(&self, id: Uuid) -> Option<Vec<&FolderNode>> {
        let mut chain: Vec<&FolderNode> = Vec::new();
        let mut current = self.get(id)?;
        loop {
            if chain.len() >= MAX_FOLDER_DEPTH {
                return None;
            }
            chain.push(current);
            let Some(parent_id) = current.parent_id else {
                return Some(chain);
            };
            let parent = self.get(parent_id)?;
            if chain.iter().any(|folder| folder.id == parent.id) {
                return None;
            }
            current = parent;
        }
    }
}

/// The level the viewer holds on the item, or `None` when the viewer may
/// not see it at all.
///
/// The rules, in the order they are applied:
///
/// 1. **The tenant.** A viewer scoped to one tenant sees that tenant's
///    items and nothing else.
/// 2. **The depth.** A chain deeper than [`MAX_FOLDER_DEPTH`] is corrupt
///    data: no level at all.
/// 3. **`"*:*"`.** A holder of every permission manages everything of
///    the organisation, personal spaces included.
/// 4. **The author.** The author of an item manages it: the query's
///    `owner_id`, or the folder's `owner_id`.
/// 5. **The rules.** The highest level of the rules that name the item
///    itself or a folder in its chain.
/// 6. **The personal space.** A chain that holds a personal folder is a
///    wall: its owner manages what is inside it, and anybody else sees
///    it only through a rule.
/// 7. **The shared space.** A chain with no personal folder is the
///    shared space: every member of the organisation sees the item at
///    `view`, and a rule raises the level.
#[must_use]
pub fn resolve_level(
    target: &AccessTarget,
    chain: &[FolderNode],
    grants: &[AccessGrant],
    viewer: &SavedQueryViewer,
) -> Option<Level> {
    // The tenant boundary: a viewer scoped to one tenant sees that
    // tenant's items. A row that belongs to no tenant is the shared
    // catalog's own, and which tenant reads it is the scope's decision.
    if let Some(tenant_id) = viewer.tenant_id
        && let Some(item_tenant) = target.tenant_id
        && item_tenant != tenant_id
    {
        return None;
    }
    // A chain deeper than the tree allows is corrupt data, and corrupt
    // data answers nothing.
    if chain.len() > MAX_FOLDER_DEPTH {
        return None;
    }
    // `"*:*"` manages everything of the organisation.
    if viewer.all {
        return Some(Level::Manage);
    }
    // The author manages their own item.
    if viewer.id.is_some() && viewer.id == target.owner_id {
        return Some(Level::Manage);
    }
    // The rules that name the item itself, or a folder in its chain.
    let granted = grants
        .iter()
        .filter(|grant| {
            grant.query_id == Some(target.id)
                || grant
                    .folder_id
                    .is_some_and(|folder_id| chain.iter().any(|folder| folder.id == folder_id))
        })
        .map(|grant| grant.level)
        .max();
    // A personal space is a wall.
    if let Some(personal) = chain.iter().find(|folder| folder.is_personal) {
        if personal.owner_id.is_some() && personal.owner_id == viewer.id {
            return Some(Level::Manage);
        }
        return granted;
    }
    // The shared space: every member of the organisation sees the item.
    Some(granted.unwrap_or(Level::View).max(Level::View))
}

/// Why a folder cannot be created, renamed or moved as asked. Every
/// message is fixed text, safe to return to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderRule {
    /// Blank name.
    NameRequired,
    /// Over [`MAX_FOLDER_NAME_CHARS`].
    NameTooLong,
    /// A sibling already has this name.
    DuplicateName,
    /// The parent folder does not exist.
    ParentNotFound,
    /// Moving a folder under itself or one of its descendants.
    Cycle,
    /// The result would be deeper than [`MAX_FOLDER_DEPTH`].
    TooDeep,
}

/// Why a folder write was refused.
///
/// The store says *what* was wrong; the route layer says *what to tell
/// the caller*. Every variant therefore names exactly one thing, so the
/// route can answer with its own fixed sentence — and the text of this
/// error never reaches a response, the same rule the rest of the crate
/// follows for the database's own errors (principle 4 of `AGENTS.md`).
#[derive(Debug, Error)]
pub enum FolderError {
    /// The folder does not exist, or it belongs to another tenant.
    #[error("the folder does not exist")]
    NotFound,
    /// The folder still holds queries that are not in the trash.
    #[error("the folder still holds queries")]
    HoldsQueries,
    /// The folder still holds queries that wait in the trash.
    #[error("the folder still holds queries in the trash")]
    HoldsTrashedQueries,
    /// The folder still holds sub-folders.
    #[error("the folder still holds sub-folders")]
    HoldsFolders,
    /// A sibling already carries that name.
    #[error("a sibling folder already carries that name")]
    DuplicateName,
    /// The name is empty, or longer than [`MAX_FOLDER_NAME_CHARS`].
    #[error("the name is empty or too long")]
    InvalidName,
    /// The tree would be deeper than [`MAX_FOLDER_DEPTH`].
    #[error("the tree would be deeper than {MAX_FOLDER_DEPTH} levels")]
    TooDeep,
    /// The move would make the folder its own ancestor.
    #[error("the folder would become its own ancestor")]
    Cycle,
    /// A `Personal space` may not be renamed, moved or deleted.
    #[error("a personal space is not renamed, moved or deleted")]
    PersonalSpace,
    /// The database refused or failed the write.
    #[error("database error")]
    Database(#[source] StoreError),
}

impl From<FolderRule> for FolderError {
    /// The pure rule and the store's answer say the same thing: the
    /// rule that was broken is the answer the caller gets.
    fn from(rule: FolderRule) -> Self {
        match rule {
            FolderRule::NameRequired | FolderRule::NameTooLong => Self::InvalidName,
            FolderRule::DuplicateName => Self::DuplicateName,
            FolderRule::ParentNotFound => Self::NotFound,
            FolderRule::Cycle => Self::Cycle,
            FolderRule::TooDeep => Self::TooDeep,
        }
    }
}

impl From<StoreError> for FolderError {
    /// The database's own failure keeps its text behind the error, so it
    /// can be logged; the answer the caller sees stays the generic
    /// "database error".
    fn from(err: StoreError) -> Self {
        Self::Database(err)
    }
}

impl From<sqlx::Error> for FolderError {
    /// A failed query is the database's own failure: it keeps its
    /// classification inside the [`StoreError`] and the caller still
    /// sees nothing but "database error".
    fn from(err: sqlx::Error) -> Self {
        Self::Database(StoreError::from(err))
    }
}

/// The depth of a folder: 1 at the root, `None` for an unknown id. A
/// corrupt cycle already in storage stops the walk instead of looping.
#[must_use]
pub fn depth(folders: &[FolderNode], id: Uuid) -> Option<usize> {
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut current = Some(folders.iter().find(|folder| folder.id == id)?);
    let mut depth = 0usize;
    while let Some(folder) = current {
        if !seen.insert(folder.id) {
            return None;
        }
        depth += 1;
        current = folder
            .parent_id
            .and_then(|parent| folders.iter().find(|candidate| candidate.id == parent));
    }
    Some(depth)
}

/// The levels in the subtree rooted at `id`, counting `id` itself (a leaf
/// is 1).
#[must_use]
pub fn subtree_height(folders: &[FolderNode], id: Uuid) -> usize {
    fn walk(folders: &[FolderNode], id: Uuid, seen: &mut HashSet<Uuid>) -> usize {
        if !seen.insert(id) {
            return 0;
        }
        1 + folders
            .iter()
            .filter(|folder| folder.parent_id == Some(id))
            .map(|folder| walk(folders, folder.id, seen))
            .max()
            .unwrap_or(0)
    }
    walk(folders, id, &mut HashSet::new())
}

/// Whether `candidate` is `ancestor` or lies anywhere below it.
#[must_use]
pub fn is_self_or_descendant(folders: &[FolderNode], ancestor: Uuid, candidate: Uuid) -> bool {
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut current = Some(candidate);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        current = folders
            .iter()
            .find(|folder| folder.id == id)
            .and_then(|folder| folder.parent_id);
    }
    false
}

/// Whether the folder is a personal space: one person's own space.
///
/// A personal space cannot be renamed, moved or deleted — the tree it
/// holds is that person's, and moving it would move their private
/// queries — so every write path asks this before it writes.
#[must_use]
pub fn is_personal_space(folders: &[FolderNode], id: Uuid) -> bool {
    folders
        .iter()
        .any(|folder| folder.id == id && folder.is_personal)
}

/// Check that the folder `moving` (`None` for a new folder) may be named
/// `name` and sit under `parent` (`None` for the root), returning the
/// trimmed name.
///
/// The first broken rule is the one returned, in this order: the name is
/// required, the name is too long, the parent exists, the move makes no
/// cycle, the result is not too deep, and no sibling carries the same
/// name (compared case-insensitively). A `Personal space` does not count
/// as a sibling for the name: its name was written by the system, and
/// nobody is blocked by it.
///
/// # Errors
///
/// The first [`FolderRule`] the placement breaks.
pub fn check_placement(
    folders: &[FolderNode],
    moving: Option<Uuid>,
    name: &str,
    parent: Option<Uuid>,
) -> Result<String, FolderRule> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FolderRule::NameRequired);
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(FolderRule::NameTooLong);
    }
    let parent_depth = match parent {
        None => 0,
        Some(parent_id) => match depth(folders, parent_id) {
            Some(parent_depth) => parent_depth,
            None => return Err(FolderRule::ParentNotFound),
        },
    };
    if let Some(moving) = moving
        && let Some(parent) = parent
        && is_self_or_descendant(folders, moving, parent)
    {
        return Err(FolderRule::Cycle);
    }
    let height = moving.map_or(1, |id| subtree_height(folders, id).max(1));
    if parent_depth + height > MAX_FOLDER_DEPTH {
        return Err(FolderRule::TooDeep);
    }
    let lower = name.to_lowercase();
    let clash = folders.iter().any(|folder| {
        folder.parent_id == parent
            && Some(folder.id) != moving
            // A `Personal space` carries a name the system wrote ("Personal
            // space of Rina Wijaya") and it belongs to one person, so
            // nobody may be blocked from an ordinary folder by it.
            && !folder.is_personal
            && folder.name.to_lowercase() == lower
    });
    if clash {
        return Err(FolderRule::DuplicateName);
    }
    Ok(name.to_owned())
}

/// One folder, as the database holds it.
#[derive(Debug, FromRow)]
struct FolderRow {
    id: Uuid,
    parent_id: Option<Uuid>,
    is_personal: bool,
    owner_id: Option<Uuid>,
    tenant_id: Uuid,
    name: String,
}

impl From<FolderRow> for FolderNode {
    fn from(row: FolderRow) -> Self {
        Self {
            id: row.id,
            parent_id: row.parent_id,
            is_personal: row.is_personal,
            owner_id: row.owner_id,
            tenant_id: row.tenant_id,
            name: row.name,
        }
    }
}

/// One rule, as the database holds it.
#[derive(Debug, FromRow)]
struct GrantRow {
    query_id: Option<Uuid>,
    folder_id: Option<Uuid>,
    user_id: Option<Uuid>,
    role_id: Option<Uuid>,
    level: String,
}

impl GrantRow {
    /// The rule, or `None` when its level is a word outside the set: a
    /// rule that says something else grants nothing.
    fn into_grant(self) -> Option<AccessGrant> {
        Some(AccessGrant {
            query_id: self.query_id,
            folder_id: self.folder_id,
            user_id: self.user_id,
            role_id: self.role_id,
            level: Level::parse(&self.level)?,
        })
    }
}

/// A rule to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGrant {
    /// The person the rule names.
    pub user_id: Option<Uuid>,
    /// The role the rule names.
    pub role_id: Option<Uuid>,
    /// What the rule gives.
    pub level: Level,
}

/// The folders of one tenant, or every folder when `tenant_id` is
/// `None`.
///
/// The map is built with the scope the read uses, so a query that points
/// at a folder of another tenant finds no folder and therefore no level:
/// it is invisible, not an error.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_folders(
    pool: &PgPool,
    tenant_id: Option<Uuid>,
) -> Result<Vec<FolderNode>, StoreError> {
    let rows: Vec<FolderRow> = sqlx::query_as(
        "SELECT id, parent_id, is_personal, owner_id, tenant_id, name FROM saved_query_folder \
         WHERE $1::uuid IS NULL OR tenant_id = $1 \
         ORDER BY name, id",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(FolderNode::from).collect())
}

/// The name of a person's personal space.
///
/// The same shape the dashboards' personal spaces carry
/// (`lakehouse-bi::folders`): "Personal space of {name}", trimmed and cut
/// to [`MAX_FOLDER_NAME_CHARS`] characters, so a long display name never
/// makes the database refuse the folder.
#[must_use]
pub fn personal_space_name(display_name: &str) -> String {
    format!("Personal space of {}", display_name.trim())
        .chars()
        .take(MAX_FOLDER_NAME_CHARS)
        .collect()
}

/// The folders of the viewer's tenant, each with the level the viewer
/// holds on it.
///
/// A folder the viewer may not see at all is not in the answer: the
/// tree a caller sees holds only what they may see, and a personal space
/// of somebody else is not in it.
///
/// The level is resolved by the one resolver ([`resolve_level`]), with
/// the same chain and the same rules every other reader uses — so the
/// tree and a query inside it can never disagree about who may do what.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn folders_with_levels(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
) -> Result<Vec<(FolderNode, Level)>, StoreError> {
    let folders = list_folders(pool, viewer.tenant_id).await?;
    let grants = grants_for_viewer(pool, viewer).await?;
    let map = FolderMap::new(folders.clone());
    let mut answer = Vec::new();
    for folder in &folders {
        let Some(chain) = map.chain(folder.id) else {
            // A folder whose chain cannot be built is corrupt data, and
            // corrupt data answers nothing.
            continue;
        };
        let chain: Vec<FolderNode> = chain.into_iter().cloned().collect();
        let target = AccessTarget {
            id: folder.id,
            owner_id: folder.owner_id,
            folder_id: folder.parent_id,
            tenant_id: Some(folder.tenant_id),
        };
        if let Some(level) = resolve_level(&target, &chain, &grants, viewer) {
            answer.push((folder.clone(), level));
        }
    }
    Ok(answer)
}

/// The personal space of one person of one tenant, created on first use.
///
/// One folder per tenant and person: the partial unique index on
/// `(tenant_id, owner_id)` decides, so two people writing at the same
/// moment end up in the same space rather than in two of them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn ensure_personal_space(
    pool: &PgPool,
    tenant_id: Uuid,
    owner_id: Uuid,
    display_name: &str,
) -> Result<FolderNode, StoreError> {
    sqlx::query(
        "INSERT INTO saved_query_folder (tenant_id, name, is_personal, owner_id) \
         VALUES ($1, $2, true, $3) \
         ON CONFLICT (tenant_id, owner_id) WHERE is_personal DO NOTHING",
    )
    .bind(tenant_id)
    .bind(personal_space_name(display_name))
    .bind(owner_id)
    .execute(pool)
    .await?;
    let row: FolderRow = sqlx::query_as(
        "SELECT id, parent_id, is_personal, owner_id, tenant_id, name \
         FROM saved_query_folder \
         WHERE tenant_id = $1 AND owner_id = $2 AND is_personal",
    )
    .bind(tenant_id)
    .bind(owner_id)
    .fetch_one(pool)
    .await?;
    Ok(FolderNode::from(row))
}

/// The rules that name the viewer: their own rule, and the rules of the
/// roles they hold.
///
/// The role is matched by name, because that is what the principal
/// carries. The tenant is not matched here: a rule only ever decides a
/// level inside the tenant whose row is being read.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn grants_for_viewer(
    pool: &PgPool,
    viewer: &SavedQueryViewer,
) -> Result<Vec<AccessGrant>, StoreError> {
    let rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT query_id, folder_id, user_id, role_id, level FROM saved_query_access_rule \
         WHERE user_id = $1 OR role_id IN (SELECT id FROM role WHERE name = ANY($2))",
    )
    .bind(viewer.id)
    .bind(&viewer.role_names)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(GrantRow::into_grant).collect())
}

/// The rules on one item: the query's own, or the folder's.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn grants_for_item(
    pool: &PgPool,
    query_id: Option<Uuid>,
    folder_id: Option<Uuid>,
) -> Result<Vec<AccessGrant>, StoreError> {
    let rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT query_id, folder_id, user_id, role_id, level FROM saved_query_access_rule \
         WHERE ($1::uuid IS NOT NULL AND query_id = $1) \
            OR ($2::uuid IS NOT NULL AND folder_id = $2) \
         ORDER BY granted_at, id",
    )
    .bind(query_id)
    .bind(folder_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(GrantRow::into_grant).collect())
}

/// Replace the rules on one item with `rules`.
///
/// The old rules go and the new ones are written in one transaction, so
/// an item never carries half of one set and half of another.
///
/// A rule names one item (a query or a folder) and one grantee (a person
/// or a role), exactly as the table's own checks say; a rule that names
/// two of either, or none, is refused here rather than written.
///
/// # Errors
///
/// Returns [`StoreError::Validation`] when a rule names both a person and
/// a role, or neither, or both a query and a folder, and
/// [`StoreError::Database`] if a query fails.
pub async fn replace_grants(
    pool: &PgPool,
    query_id: Option<Uuid>,
    folder_id: Option<Uuid>,
    rules: &[NewGrant],
    granted_by: Option<Uuid>,
) -> Result<(), StoreError> {
    if query_id.is_some() == folder_id.is_some() {
        return Err(StoreError::Validation(
            "a rule is on one item: a saved query or a folder, never both and never neither"
                .to_owned(),
        ));
    }
    for rule in rules {
        if rule.user_id.is_some() == rule.role_id.is_some() {
            return Err(StoreError::Validation(
                "a rule names one grantee: a person or a role, never both and never neither"
                    .to_owned(),
            ));
        }
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM saved_query_access_rule \
         WHERE ($1::uuid IS NOT NULL AND query_id = $1) \
            OR ($2::uuid IS NOT NULL AND folder_id = $2)",
    )
    .bind(query_id)
    .bind(folder_id)
    .execute(&mut *tx)
    .await?;
    for rule in rules {
        sqlx::query(
            "INSERT INTO saved_query_access_rule \
             (query_id, folder_id, user_id, role_id, level, granted_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(query_id)
        .bind(folder_id)
        .bind(rule.user_id)
        .bind(rule.role_id)
        .bind(rule.level.as_str())
        .bind(granted_by)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ── The writes the routes need ──────────────────────────────────────────

/// One folder, by id, in whichever tenant it sits.
///
/// The tenant is read from the row itself, so a write never has to be
/// told which tenant it acts for — and the folder's own tenant is the
/// one the rest of the write is checked against.
async fn load_folder(pool: &PgPool, id: Uuid) -> Result<Option<FolderNode>, FolderError> {
    let row: Option<FolderRow> = sqlx::query_as(
        "SELECT id, parent_id, is_personal, owner_id, tenant_id, name \
         FROM saved_query_folder WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(FolderNode::from))
}

/// Make a folder under `parent_id` (`None` for the root) of `tenant_id`.
///
/// The name is trimmed, and it must be 1 to [`MAX_FOLDER_NAME_CHARS`]
/// characters. The parent must be a folder of the same tenant: a folder
/// of another tenant is not found at all, so a caller cannot reach
/// across the tenant boundary by naming one. The name must be free among
/// the siblings (compared case-insensitively), and the folder may not
/// make the tree deeper than [`MAX_FOLDER_DEPTH`] levels.
///
/// `owner_id` is the creator of the folder; it is `None` for a folder
/// nobody owns beyond the tenant.
///
/// # Errors
///
/// Returns [`FolderError::InvalidName`] for a blank name or one that is
/// too long, [`FolderError::NotFound`] when the parent does not exist,
/// [`FolderError::DuplicateName`] when a sibling carries the name,
/// [`FolderError::TooDeep`] when the tree would grow past the depth
/// limit, and [`FolderError::Database`] if a query fails.
pub async fn create_folder(
    pool: &PgPool,
    tenant_id: Uuid,
    owner_id: Option<Uuid>,
    name: &str,
    parent_id: Option<Uuid>,
) -> Result<FolderNode, FolderError> {
    // Only the tenant's own folders are in the tree, so a parent of
    // another tenant is not found by the placement rule.
    let folders = list_folders(pool, Some(tenant_id)).await?;
    let name = check_placement(&folders, None, name, parent_id).map_err(FolderError::from)?;
    let row: FolderRow = sqlx::query_as(
        "INSERT INTO saved_query_folder (tenant_id, parent_id, name, owner_id) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, parent_id, is_personal, owner_id, tenant_id, name",
    )
    .bind(tenant_id)
    .bind(parent_id)
    .bind(&name)
    .bind(owner_id)
    .fetch_one(pool)
    .await?;
    Ok(FolderNode::from(row))
}

/// Give the folder another name.
///
/// The name is trimmed, and it must be 1 to [`MAX_FOLDER_NAME_CHARS`]
/// characters and free among the siblings (compared case-insensitively).
/// The folder keeps its place in the tree.
///
/// # Errors
///
/// Returns [`FolderError::NotFound`] when the folder does not exist,
/// [`FolderError::PersonalSpace`] when it is a `Personal space`,
/// [`FolderError::InvalidName`] for a blank name or one that is too
/// long, [`FolderError::DuplicateName`] when a sibling carries the name,
/// and [`FolderError::Database`] if a query fails.
pub async fn rename_folder(pool: &PgPool, id: Uuid, name: &str) -> Result<FolderNode, FolderError> {
    let Some(folder) = load_folder(pool, id).await? else {
        return Err(FolderError::NotFound);
    };
    if folder.is_personal {
        return Err(FolderError::PersonalSpace);
    }
    let folders = list_folders(pool, Some(folder.tenant_id)).await?;
    let name =
        check_placement(&folders, Some(id), name, folder.parent_id).map_err(FolderError::from)?;
    let row: FolderRow = sqlx::query_as(
        "UPDATE saved_query_folder SET name = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, parent_id, is_personal, owner_id, tenant_id, name",
    )
    .bind(id)
    .bind(&name)
    .fetch_one(pool)
    .await?;
    Ok(FolderNode::from(row))
}

/// Move the folder under `new_parent` (`None` for the root).
///
/// The whole subtree that moves has to fit in [`MAX_FOLDER_DEPTH`]
/// levels: the depth of the new parent plus the height of the subtree
/// may not pass the limit. The folder may not become its own ancestor,
/// and the new parent must be a folder of the same tenant — a folder of
/// another tenant is not found at all. The name stays, and it stays
/// free among the new siblings.
///
/// # Errors
///
/// Returns [`FolderError::NotFound`] when the folder or the new parent
/// does not exist, [`FolderError::PersonalSpace`] when the folder is a
/// `Personal space`, [`FolderError::Cycle`] when the move would make
/// the folder its own ancestor, [`FolderError::TooDeep`] when the
/// subtree would grow past the depth limit,
/// [`FolderError::DuplicateName`] when a new sibling carries the name,
/// and [`FolderError::Database`] if a query fails.
pub async fn move_folder(
    pool: &PgPool,
    id: Uuid,
    new_parent: Option<Uuid>,
) -> Result<FolderNode, FolderError> {
    let Some(folder) = load_folder(pool, id).await? else {
        return Err(FolderError::NotFound);
    };
    if folder.is_personal {
        return Err(FolderError::PersonalSpace);
    }
    let folders = list_folders(pool, Some(folder.tenant_id)).await?;
    // The name does not change, but the sibling rule still holds: the
    // folder may not land beside a sibling that carries the same name.
    check_placement(&folders, Some(id), &folder.name, new_parent).map_err(FolderError::from)?;
    let row: FolderRow = sqlx::query_as(
        "UPDATE saved_query_folder SET parent_id = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, parent_id, is_personal, owner_id, tenant_id, name",
    )
    .bind(id)
    .bind(new_parent)
    .fetch_one(pool)
    .await?;
    Ok(FolderNode::from(row))
}

/// Delete the folder.
///
/// A folder is deleted only when it holds nothing: no live query, no
/// query in the trash and no sub-folder. The `saved_query` rows point at
/// their folder under `ON DELETE RESTRICT`, so the database refuses the
/// delete as well — the explicit check is here so the answer is the
/// route's fixed sentence and not a database error.
///
/// The route layer answers [`FolderError::HoldsQueries`] and
/// [`FolderError::HoldsTrashedQueries`] with the one sentence the plan
/// names ("This folder still holds queries, some of them in the trash.
/// Move or restore them first."), and
/// [`FolderError::HoldsFolders`] with a second sentence of its own.
///
/// # Errors
///
/// Returns [`FolderError::NotFound`] when the folder does not exist,
/// [`FolderError::PersonalSpace`] when it is a `Personal space`,
/// [`FolderError::HoldsQueries`] or [`FolderError::HoldsTrashedQueries`]
/// when it still holds queries, [`FolderError::HoldsFolders`] when it
/// still holds sub-folders, and [`FolderError::Database`] if a query
/// fails.
pub async fn delete_folder(pool: &PgPool, id: Uuid) -> Result<(), FolderError> {
    let Some(folder) = load_folder(pool, id).await? else {
        return Err(FolderError::NotFound);
    };
    if folder.is_personal {
        return Err(FolderError::PersonalSpace);
    }
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM saved_query WHERE folder_id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    if live > 0 {
        return Err(FolderError::HoldsQueries);
    }
    let trashed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM saved_query WHERE folder_id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    if trashed > 0 {
        return Err(FolderError::HoldsTrashedQueries);
    }
    let children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saved_query_folder WHERE parent_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
    if children > 0 {
        return Err(FolderError::HoldsFolders);
    }
    sqlx::query("DELETE FROM saved_query_folder WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── The grantee search ───────────────────────────────────────────────────

/// One person a saved query may be shared with.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GranteePerson {
    /// The person's id.
    pub id: Uuid,
    /// The person's display name.
    pub name: String,
    /// The person's email address.
    pub email: String,
}

/// One role a saved query may be shared with.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GranteeRole {
    /// The role's id.
    pub id: Uuid,
    /// The role's name.
    pub name: String,
}

/// The people and the roles a saved query may be shared with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GranteeMatches {
    /// The people of the tenant, or of the deployment when the search
    /// names no tenant.
    pub people: Vec<GranteePerson>,
    /// The roles of the deployment.
    pub roles: Vec<GranteeRole>,
}

/// The `ILIKE` pattern that matches `needle` as a literal substring.
///
/// `%`, `_` and `\` are the wildcards of `ILIKE`; the escape character
/// comes first, so a literal `\` survives the pass that escapes the two
/// wildcards and never turns into one. An empty needle gives the
/// pattern `%`, which matches every row: an empty search box shows the
/// first people rather than nothing.
pub(crate) fn escape_like_pattern(needle: &str) -> String {
    let escaped = needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// The people and the roles a saved query may be shared with.
///
/// The people are the members of `tenant_id` (through `app_user_tenant`)
/// whose name or email holds `needle`, and only the active ones: an
/// inactive person is nobody to share with. With no tenant the search
/// covers every person of the deployment.
///
/// The roles are the roles whose name holds `needle`. The `role` table
/// has no tenant column — a person's tenants live in `app_user_tenant` —
/// so a role is a property of the deployment and every role of it is a
/// possible grantee.
///
/// `%`, `_` and `\` in `needle` are literals, not wildcards.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn search_grantees(
    pool: &PgPool,
    tenant_id: Option<Uuid>,
    needle: &str,
    limit: i64,
) -> Result<GranteeMatches, StoreError> {
    // A limit of zero asks for no rows, and Postgres refuses a negative
    // one, so the caller's number is clamped here.
    let limit = limit.max(0);
    let pattern = escape_like_pattern(needle.trim());
    let people: Vec<GranteePerson> = sqlx::query_as(
        "SELECT u.id, u.name, u.email FROM app_user u \
         WHERE u.status = 'active' \
           AND ($1::uuid IS NULL OR EXISTS ( \
                SELECT 1 FROM app_user_tenant t \
                 WHERE t.user_id = u.id AND t.tenant_id = $1)) \
           AND (u.name ILIKE $2 OR u.email ILIKE $2) \
         ORDER BY u.name, u.email LIMIT $3",
    )
    .bind(tenant_id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let roles: Vec<GranteeRole> =
        sqlx::query_as("SELECT id, name FROM role WHERE name ILIKE $1 ORDER BY name LIMIT $2")
            .bind(&pattern)
            .bind(limit)
            .fetch_all(pool)
            .await?;
    Ok(GranteeMatches { people, roles })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const TENANT: Uuid = Uuid::from_u128(1);
    const OTHER_TENANT: Uuid = Uuid::from_u128(2);
    const PERSON: Uuid = Uuid::from_u128(10);
    const OTHER_PERSON: Uuid = Uuid::from_u128(11);
    const ROLE: Uuid = Uuid::from_u128(20);

    fn viewer(id: Option<Uuid>, tenant_id: Option<Uuid>) -> SavedQueryViewer {
        SavedQueryViewer {
            id,
            role_names: Vec::new(),
            tenant_id,
            all: false,
        }
    }

    fn target(id: Uuid, owner_id: Option<Uuid>, folder_id: Option<Uuid>) -> AccessTarget {
        AccessTarget {
            id,
            owner_id,
            folder_id,
            tenant_id: Some(TENANT),
        }
    }

    fn folder(id: Uuid, parent: Option<Uuid>, personal: bool, owner: Option<Uuid>) -> FolderNode {
        FolderNode {
            id,
            parent_id: parent,
            is_personal: personal,
            owner_id: owner,
            tenant_id: TENANT,
            name: format!("folder {id}"),
        }
    }

    fn grant(query_id: Option<Uuid>, folder_id: Option<Uuid>, level: Level) -> AccessGrant {
        AccessGrant {
            query_id,
            folder_id,
            user_id: Some(PERSON),
            role_id: None,
            level,
        }
    }

    /// The shared space is the root: a query with no folder sits there,
    /// and every member of the organisation sees it at `view`.
    #[test]
    fn the_shared_space_gives_view_to_the_organisation() {
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, None),
            &[],
            &[],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::View));
    }

    /// A rule on a query in the shared space raises the level above
    /// `view`.
    #[test]
    fn a_rule_on_a_shared_query_raises_its_level() {
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, None),
            &[],
            &[grant(Some(Uuid::from_u128(100)), None, Level::Edit)],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Edit));
    }

    /// The author of a query manages it, whatever space it sits in.
    #[test]
    fn the_author_manages_their_own_query() {
        let level = resolve_level(
            &target(Uuid::from_u128(100), Some(PERSON), None),
            &[],
            &[],
            &viewer(Some(PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Manage));
    }

    /// A personal space is a wall: its owner manages what is inside it.
    #[test]
    fn the_owner_of_a_personal_space_manages_what_is_inside_it() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let level = resolve_level(
            &target(Uuid::from_u128(100), Some(PERSON), Some(personal.id)),
            &[personal],
            &[],
            &viewer(Some(PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Manage));
    }

    /// A personal space gives nothing to anybody else: no rule, no
    /// level, and the item does not exist for that person.
    #[test]
    fn a_personal_space_gives_nothing_to_another_person() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(personal.id)),
            &[personal],
            &[],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, None);
    }

    /// A rule is the only way somebody else sees a query in a personal
    /// space, and it gives exactly what it says.
    #[test]
    fn a_rule_opens_a_personal_space_at_the_level_it_names() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(personal.id)),
            &[personal],
            &[grant(Some(Uuid::from_u128(100)), None, Level::View)],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::View));
    }

    /// A rule on a folder covers what is inside it.
    #[test]
    fn a_rule_on_a_folder_covers_the_item_inside_it() {
        let ordinary = folder(Uuid::from_u128(200), None, false, Some(PERSON));
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(ordinary.id)),
            std::slice::from_ref(&ordinary),
            &[grant(None, Some(ordinary.id), Level::Edit)],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Edit));
    }

    /// A rule on a folder covers what is inside it, at any depth the tree
    /// allows: the rule names the folder, and the chain carries it down.
    #[test]
    fn a_rule_on_a_folder_covers_what_is_deeper_inside_it() {
        let root = folder(Uuid::from_u128(200), None, false, Some(PERSON));
        let child = folder(Uuid::from_u128(201), Some(root.id), false, None);
        let grandchild = folder(Uuid::from_u128(202), Some(child.id), false, None);
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(grandchild.id)),
            &[grandchild, child, root.clone()],
            &[grant(None, Some(root.id), Level::Edit)],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Edit));
    }

    /// The author of a query manages it, whatever space it sits in — a
    /// personal space of somebody else included, because the author is the
    /// one who wrote it.
    #[test]
    fn the_author_of_a_query_in_another_persons_personal_space_manages_it() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let level = resolve_level(
            &target(Uuid::from_u128(100), Some(OTHER_PERSON), Some(personal.id)),
            &[personal],
            &[],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Manage));
    }

    /// A chain of exactly four folders is as deep as the tree allows, and
    /// it answers.
    #[test]
    fn a_chain_of_exactly_four_folders_is_allowed() {
        let chain = vec![
            folder(Uuid::from_u128(201), None, false, None),
            folder(
                Uuid::from_u128(202),
                Some(Uuid::from_u128(201)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(203),
                Some(Uuid::from_u128(202)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(204),
                Some(Uuid::from_u128(203)),
                false,
                None,
            ),
        ];
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(Uuid::from_u128(204))),
            &chain,
            &[],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::View));
    }

    /// A rule on the item and a rule on its folder give the higher of the
    /// two.
    #[test]
    fn a_rule_on_the_item_and_a_rule_on_its_folder_give_the_higher_one() {
        let ordinary = folder(Uuid::from_u128(200), None, false, Some(PERSON));
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(ordinary.id)),
            std::slice::from_ref(&ordinary),
            &[
                grant(Some(Uuid::from_u128(100)), None, Level::Edit),
                grant(None, Some(ordinary.id), Level::Manage),
            ],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Manage));
    }

    /// A person's rule and a role's rule on the same item give the
    /// higher of the two.
    #[test]
    fn a_person_rule_and_a_role_rule_give_the_higher_one() {
        let mut person_rule = grant(Some(Uuid::from_u128(100)), None, Level::Edit);
        person_rule.user_id = Some(PERSON);
        let mut role_rule = grant(Some(Uuid::from_u128(100)), None, Level::Manage);
        role_rule.user_id = None;
        role_rule.role_id = Some(ROLE);
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, None),
            &[],
            &[person_rule, role_rule],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, Some(Level::Manage));
    }

    /// A holder of every permission manages everything of the
    /// organisation, personal spaces included.
    #[test]
    fn the_unrestricted_holder_manages_everything() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let mut all = viewer(Some(OTHER_PERSON), Some(TENANT));
        all.all = true;
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(personal.id)),
            &[personal],
            &[],
            &all,
        );
        assert_eq!(level, Some(Level::Manage));
    }

    /// A chain deeper than the tree allows is corrupt data: no level at
    /// all.
    #[test]
    fn a_chain_deeper_than_the_limit_gives_nothing() {
        let chain = vec![
            folder(Uuid::from_u128(201), None, false, None),
            folder(
                Uuid::from_u128(202),
                Some(Uuid::from_u128(201)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(203),
                Some(Uuid::from_u128(202)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(204),
                Some(Uuid::from_u128(203)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(205),
                Some(Uuid::from_u128(204)),
                false,
                None,
            ),
        ];
        let level = resolve_level(
            &target(Uuid::from_u128(100), None, Some(Uuid::from_u128(205))),
            &chain,
            &[],
            &viewer(Some(OTHER_PERSON), Some(TENANT)),
        );
        assert_eq!(level, None);
    }

    /// Another tenant's item gives nothing at all.
    #[test]
    fn another_tenants_item_gives_nothing() {
        let mut item = target(Uuid::from_u128(100), None, None);
        item.tenant_id = Some(OTHER_TENANT);
        let level = resolve_level(&item, &[], &[], &viewer(Some(OTHER_PERSON), Some(TENANT)));
        assert_eq!(level, None);
    }

    /// A word outside `view`, `edit` and `manage` names no level, so it
    /// grants nothing.
    #[test]
    fn a_word_outside_the_set_grants_nothing() {
        assert_eq!(Level::parse("view"), Some(Level::View));
        assert_eq!(Level::parse("edit"), Some(Level::Edit));
        assert_eq!(Level::parse("manage"), Some(Level::Manage));
        assert_eq!(Level::parse("owner"), None);
        assert_eq!(Level::parse(""), None);
    }

    /// The chain of a folder is the folder itself and its parents, and
    /// a chain that cannot be built is `None`.
    #[test]
    fn a_chain_is_the_folder_and_its_parents() {
        let root = folder(Uuid::from_u128(200), None, false, None);
        let child = folder(Uuid::from_u128(201), Some(root.id), false, None);
        let map = FolderMap::new(vec![root.clone(), child.clone()]);
        let chain = map.chain(child.id).expect("the chain is built");
        assert_eq!(
            chain.iter().map(|folder| folder.id).collect::<Vec<_>>(),
            vec![child.id, root.id]
        );
        assert!(map.chain(Uuid::from_u128(999)).is_none());
    }

    /// A chain that repeats a folder is corrupt data and answers
    /// nothing.
    #[test]
    fn a_broken_chain_answers_nothing() {
        let first = folder(
            Uuid::from_u128(200),
            Some(Uuid::from_u128(201)),
            false,
            None,
        );
        let second = folder(
            Uuid::from_u128(201),
            Some(Uuid::from_u128(200)),
            false,
            None,
        );
        let map = FolderMap::new(vec![first, second]);
        assert!(map.chain(Uuid::from_u128(200)).is_none());
    }

    /// The placement rules, one at a time: the name is required, the
    /// name is at most a hundred characters, the parent exists, the move
    /// makes no cycle, the tree is at most four levels deep and a
    /// sibling's name is unique.
    #[test]
    fn the_placement_rules_refuse_what_they_name() {
        let tree = vec![
            folder(Uuid::from_u128(200), None, false, None),
            folder(
                Uuid::from_u128(201),
                Some(Uuid::from_u128(200)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(202),
                Some(Uuid::from_u128(201)),
                false,
                None,
            ),
            folder(
                Uuid::from_u128(203),
                Some(Uuid::from_u128(202)),
                false,
                None,
            ),
        ];
        assert_eq!(
            check_placement(&tree, None, "   ", None),
            Err(FolderRule::NameRequired)
        );
        assert_eq!(
            check_placement(&tree, None, &"x".repeat(101), None),
            Err(FolderRule::NameTooLong)
        );
        assert_eq!(
            check_placement(&tree, None, "Week", Some(Uuid::from_u128(999))),
            Err(FolderRule::ParentNotFound)
        );
        assert_eq!(
            check_placement(
                &tree,
                Some(Uuid::from_u128(200)),
                "Sales",
                Some(Uuid::from_u128(201))
            ),
            Err(FolderRule::Cycle)
        );
        assert_eq!(
            check_placement(&tree, None, "Day", Some(Uuid::from_u128(203))),
            Err(FolderRule::TooDeep)
        );
        // The sibling's name, shouted: the comparison ignores the case.
        let sibling_name = tree[0].name.to_uppercase();
        assert_eq!(
            check_placement(&tree, None, &sibling_name, None),
            Err(FolderRule::DuplicateName)
        );
    }

    /// The first broken rule is the one reported.
    #[test]
    fn the_first_broken_rule_is_the_one_reported() {
        let tree = vec![folder(Uuid::from_u128(200), None, false, None)];
        assert_eq!(
            check_placement(&tree, None, "", Some(Uuid::from_u128(999))),
            Err(FolderRule::NameRequired),
            "the blank name is reported before the missing parent"
        );
        assert_eq!(
            check_placement(&tree, None, &"x".repeat(101), Some(Uuid::from_u128(999))),
            Err(FolderRule::NameTooLong),
            "the long name is reported before the missing parent"
        );
    }

    /// A placement that breaks no rule comes back with its name
    /// trimmed.
    #[test]
    fn a_placement_that_breaks_no_rule_comes_back_trimmed() {
        let tree = vec![folder(Uuid::from_u128(200), None, false, None)];
        assert_eq!(
            check_placement(&tree, None, "  Feb  ", Some(Uuid::from_u128(200))),
            Ok("Feb".to_owned())
        );
    }

    /// A personal space is one person's own space: it cannot be renamed,
    /// moved or deleted.
    #[test]
    fn a_personal_space_is_recognised_as_one() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let ordinary = folder(Uuid::from_u128(201), None, false, Some(PERSON));
        let tree = vec![personal.clone(), ordinary.clone()];
        assert!(is_personal_space(&tree, personal.id));
        assert!(!is_personal_space(&tree, ordinary.id));
        assert!(!is_personal_space(&tree, Uuid::from_u128(999)));
    }

    /// A personal space carries a name the system wrote, and it blocks
    /// nobody: an ordinary folder may carry the same name.
    #[test]
    fn a_personal_space_blocks_no_name() {
        let personal = folder(Uuid::from_u128(200), None, true, Some(PERSON));
        let tree = vec![personal.clone()];
        assert_eq!(
            check_placement(&tree, None, &personal.name, None),
            Ok(personal.name.clone()),
            "the personal space's name is free for an ordinary folder"
        );
    }

    /// Every rule the placement breaks has its own answer, so the route
    /// layer can tell "the name is taken" from "the name is invalid".
    #[test]
    fn every_broken_rule_has_its_own_answer() {
        assert!(matches!(
            FolderError::from(FolderRule::NameRequired),
            FolderError::InvalidName
        ));
        assert!(matches!(
            FolderError::from(FolderRule::NameTooLong),
            FolderError::InvalidName
        ));
        assert!(matches!(
            FolderError::from(FolderRule::DuplicateName),
            FolderError::DuplicateName
        ));
        assert!(matches!(
            FolderError::from(FolderRule::ParentNotFound),
            FolderError::NotFound
        ));
        assert!(matches!(
            FolderError::from(FolderRule::Cycle),
            FolderError::Cycle
        ));
        assert!(matches!(
            FolderError::from(FolderRule::TooDeep),
            FolderError::TooDeep
        ));
    }

    /// The wildcards of the match are literals: the escape character is
    /// escaped first, so a literal `\` is not turned into a wildcard by
    /// the pass that escapes the two others.
    #[test]
    fn the_wildcards_of_the_match_are_escaped() {
        assert_eq!(escape_like_pattern("plain"), "%plain%");
        assert_eq!(escape_like_pattern("50%"), "%50\\%%");
        assert_eq!(escape_like_pattern("a_b"), "%a\\_b%");
        assert_eq!(escape_like_pattern("a\\b"), "%a\\\\b%");
    }
}
