//! Who may see which dashboard: the per-request resolution of a caller's
//! rules into the [`Viewer`] every dashboard read takes (`BI-19` part B1; plan
//! `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`, decisions D7 and D8;
//! feature page `docs/core/features/sharing-and-delivery.md`, decisions B1 to
//! B9).
//!
//! # One entry point
//!
//! [`viewer_for`] replaces `viewer_of` on the read routes whose floor moved
//! from `dashboard:read` to "signed in" (D8). The router no longer refuses a
//! person who holds no `dashboard:read`; this module does, after looking at
//! whether a rule names them or one of their roles, so a person with neither
//! is refused exactly as before (403 `permission_denied: dashboard:read`) and
//! one with a rule is let in and shown only what the rule gives.
//!
//! The pure decisions (levels, inheritance, restrictions) live in
//! `lakehouse_bi::access::AccessMap`; this module only reads its inputs and
//! turns its refusals into responses. Rules are read from Postgres on every
//! request, so a rule takes effect on the next request and there is no cache
//! to go stale.
//!
//! # The starting point
//!
//! `dashboard:read` is `view` and `dashboard:write` with it is `manage` on
//! every dashboard that is not restricted (feature B2). `dashboard:write`
//! alone is not a starting point: the routes it unlocked always also needed
//! `dashboard:read` for reading, and holding it must not newly show anything.
//! When nothing is restricted anywhere, a person with a starting point sees
//! what they always saw, so nothing but the rules and marks themselves is
//! read (no folder tree, no dashboard list): the common deployment pays two
//! small Postgres reads.
//!
//! # Failing closed
//!
//! A Postgres failure is a 500 (never "no rules, so everything"); a rule with
//! a kind or level this build does not know grants nothing; more ids than
//! `MAX_SCOPE_IDS` is a 503 with a plain sentence, never a shortened list.
//! SEC-11: no store text reaches a response.

use std::collections::HashSet;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_bi::access::{AccessMap, BoardPlacement, Grant, ItemKind, Level, TooManyIds, Viewer};
use lakehouse_bi::folders::Folder;
use lakehouse_bi::{folders, store};
use lakehouse_core::ApiError;
use lakehouse_store::dashboard_access::{self as rules, NewRule};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard_folders::classify_ch_error;
use crate::routes::dashboard_organise::{BOARD_NOT_FOUND, viewer_of};
use crate::state::AppState;
use crate::upstream_error;

/// The permission whose absence (with no rule) refuses a caller, as the router
/// refused them before the floor moved.
const ENTRY_PERMISSION: &str = "dashboard:read";

/// The starting point a caller's permissions give (feature B2): none without
/// `dashboard:read`.
#[must_use]
pub(crate) fn baseline_of(principal: &Principal) -> Option<Level> {
    if !principal.has(ENTRY_PERMISSION) {
        return None;
    }
    Some(if principal.has("dashboard:write") {
        Level::Manage
    } else {
        Level::View
    })
}

/// The rules that name the caller and every mark, from Postgres. Without a
/// Postgres pool there is nothing to read and nothing could have been stored.
async fn read_rules(
    state: &AppState,
    principal: &Principal,
) -> Result<(Vec<Grant>, Vec<(ItemKind, String)>), ApiError> {
    let Some(pg) = state.pg.as_deref() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let stored = rules::rules_for_grantee(pg, principal.id.uuid(), &principal.role_names).await?;
    let marks = rules::all_restrictions(pg).await?;
    // A kind or a level this build does not know grants nothing, and marks
    // nothing: it is dropped rather than guessed at.
    let grants = stored
        .iter()
        .filter_map(|r| {
            Some(Grant {
                kind: ItemKind::parse(&r.item_kind)?,
                item_id: r.item_id.clone(),
                level: Level::parse(&r.level)?,
            })
        })
        .collect();
    let marks = marks
        .iter()
        .filter_map(|m| Some((ItemKind::parse(&m.item_kind)?, m.item_id.clone())))
        .collect();
    Ok((grants, marks))
}

fn refused() -> ApiError {
    ApiError::PermissionDenied(ENTRY_PERMISSION.to_owned())
}

fn too_many(_: TooManyIds) -> ApiError {
    ApiError::Unavailable(TooManyIds::MESSAGE.to_owned())
}

/// Refuse a caller who holds neither `dashboard:read` nor any rule naming them
/// or one of their roles: the guard of the catalog reads (D8) that need no
/// dashboard, only the right to be in the area.
///
/// # Errors
///
/// 403 `permission_denied: dashboard:read` for such a caller; the database
/// error classified (500) when the rules cannot be read.
pub(crate) async fn require_entitled(
    state: &AppState,
    principal: &Principal,
) -> Result<(), ApiError> {
    if principal.has(ENTRY_PERMISSION) {
        return Ok(());
    }
    let (grants, _) = read_rules(state, principal).await?;
    if grants.is_empty() {
        return Err(refused());
    }
    Ok(())
}

/// The [`Viewer`] the caller reads as: [`require_entitled`], then their
/// personal space and the dashboards and folders their rules, the marks and
/// their starting point leave them.
///
/// # Errors
///
/// 403 for a caller who is not entitled; 503 with a plain sentence when too
/// many dashboards or folders carry their own permissions to work it out;
/// otherwise the store error classified (no upstream text).
pub(crate) async fn viewer_for(
    state: &AppState,
    principal: &Principal,
) -> Result<Viewer, ApiError> {
    resolve(state, principal, baseline_of(principal)).await
}

/// [`viewer_for`] for the write handlers' visibility check (`BI-19` B1 review
/// R5, option a): the same resolution, so a dashboard the caller's rules and
/// the restrictions leave out is a 404 for a writer too (it was reachable
/// through duplicate, the public link, move and revert, several of which are
/// reads in effect). The one difference is the starting point: a writer who
/// holds `dashboard:write` without `dashboard:read` keeps the reach those
/// routes always gave them (every unrestricted dashboard), not the 403 a read
/// gives, so no existing writer is newly refused; a restriction still takes it
/// away. The levels (view, edit, manage) are checked in T8.
///
/// # Errors
///
/// As [`viewer_for`].
pub(crate) async fn viewer_for_write(
    state: &AppState,
    principal: &Principal,
) -> Result<Viewer, ApiError> {
    let baseline = baseline_of(principal)
        .or_else(|| principal.has("dashboard:write").then_some(Level::Manage));
    resolve(state, principal, baseline).await
}

/// Refuse early a caller who could not reach `needed` on *any* item: no
/// starting point, no rule gives it. The answer does not depend on the item
/// (so it is no oracle for ids), is a `403` the way the router's floor was,
/// and costs nothing for the common holder of `dashboard:write` (the starting
/// point settles it with no read). Write handlers call it before they parse
/// the body or read a dashboard, so a viewer is refused the same with or
/// without a valid request (`BI-19` part B2).
///
/// # Errors
///
/// 403 `dashboard:read` for a caller with neither a starting point nor a rule
/// (as before the floor moved); 403 naming the level for one who holds less
/// everywhere; the classified store error when the rules cannot be read.
pub(crate) async fn require_can_ever(
    state: &AppState,
    principal: &Principal,
    needed: Level,
) -> Result<(), ApiError> {
    if principal.has("identity:write") {
        return Ok(());
    }
    let baseline = baseline_of(principal)
        .or_else(|| principal.has("dashboard:write").then_some(Level::Manage));
    if baseline.is_some_and(|b| b >= needed) {
        return Ok(());
    }
    let (grants, _) = read_rules(state, principal).await?;
    if grants.iter().any(|g| g.level >= needed) {
        return Ok(());
    }
    if baseline.is_none() && grants.is_empty() {
        return Err(refused());
    }
    Err(lacks(needed))
}

/// What a write handler knows about the caller, worked out once per request
/// (`BI-19` part B2, plan D8): the [`Viewer`] that decides what exists for them
/// and the [`AccessMap`] that decides what they may do with it.
///
/// A write is judged by the caller's resolved *level* on the dashboard (or
/// folder) it touches: `edit` for changing it, `manage` for sharing, the
/// public link, embedding, moving and deleting it (feature B1). The answer for
/// something the caller cannot see is the same `404` a read gives (no id
/// guessing); for something they see and may not do, `403`.
pub(crate) struct Scope {
    pub(crate) viewer: Viewer,
    map: AccessMap,
}

/// Why the caller is refused a level, in the sentence the `403` carries.
fn lacks(needed: Level) -> ApiError {
    ApiError::PermissionDenied(
        match needed {
            Level::View => "view on this dashboard",
            Level::Edit => "edit on this dashboard",
            Level::Manage => "manage on this dashboard",
        }
        .to_owned(),
    )
}

/// [`Scope`] for a write handler. The starting point is
/// [`viewer_for_write`]'s (a `dashboard:write` holder with no
/// `dashboard:read` keeps the reach those routes always gave them), so no
/// existing writer is newly refused; rules only add to it, and a restriction
/// takes it away, as for reads.
///
/// The folder tree is read only when something can change a level (a rule
/// names the caller or a mark exists), so a deployment that uses no
/// permissions pays the two small Postgres reads and nothing more.
///
/// # Errors
///
/// 403 for a caller with neither a starting point nor a rule; 503 when too
/// many dashboards carry their own permissions; otherwise the classified
/// store error.
pub(crate) async fn scope_for_write(
    state: &AppState,
    principal: &Principal,
) -> Result<Scope, ApiError> {
    let baseline = baseline_of(principal)
        .or_else(|| principal.has("dashboard:write").then_some(Level::Manage));
    let (grants, marks) = read_rules(state, principal).await?;
    if baseline.is_none() && grants.is_empty() {
        return Err(refused());
    }
    let all = principal.has("identity:write");
    let tree = if !all && (!grants.is_empty() || !marks.is_empty()) {
        folders::list_folders(&state.clickhouse, &Viewer::system())
            .await
            .map_err(|e| classify_ch_error(&e))?
    } else {
        Vec::new()
    };
    let map = AccessMap::new(
        principal.id.uuid().to_string(),
        all,
        baseline,
        &tree,
        &grants,
        &marks,
    );
    let viewer = if all || (baseline.is_some() && marks.is_empty()) {
        viewer_of(principal)
    } else {
        let placements = store::board_placements(&state.clickhouse)
            .await
            .map_err(|e| classify_ch_error(&e))?;
        map.viewer(&placements).map_err(too_many)?
    };
    Ok(Scope { viewer, map })
}

impl Scope {
    /// The level at the root of the tree, where the built-in dashboard lives
    /// and a new dashboard or folder is filed when it names no parent.
    fn root_level(&self) -> Option<Level> {
        self.map.level_at_root()
    }

    /// Require `needed` at the root.
    ///
    /// # Errors
    ///
    /// 404 for somebody with no starting point, 403 below `needed`.
    pub(crate) fn require_root(&self, needed: Level) -> Result<(), ApiError> {
        match self.root_level() {
            None => Err(ApiError::NotFound(BOARD_NOT_FOUND.to_owned())),
            Some(level) if level < needed => Err(lacks(needed)),
            Some(_) => Ok(()),
        }
    }

    /// Require `needed` on a dashboard already fetched: its row, or `None` for
    /// the built-in dashboard (judged at the root, and a 404 for somebody who
    /// does not see it). Lets a caller that checks many people against one
    /// dashboard read the row once (`BI-19` E review E-3).
    ///
    /// # Errors
    ///
    /// As [`require_on_board`](Self::require_on_board) and
    /// [`require_root`](Self::require_root).
    pub(crate) fn require_known(
        &self,
        board: Option<&lakehouse_bi::store::Board>,
        needed: Level,
    ) -> Result<(), ApiError> {
        if let Some(board) = board {
            return self.require_on_board(board, needed);
        }
        if !self.viewer.sees_builtin() {
            return Err(ApiError::NotFound(BOARD_NOT_FOUND.to_owned()));
        }
        self.require_root(needed)
    }

    /// Require `needed` on the dashboard whose row is `board`.
    ///
    /// # Errors
    ///
    /// 404 when the caller has no level on it, 403 when it is below `needed`.
    pub(crate) fn require_on_board(
        &self,
        board: &lakehouse_bi::store::Board,
        needed: Level,
    ) -> Result<(), ApiError> {
        let folder_id = board.folder_id.clone().unwrap_or_default();
        let owner = board.owner_id.clone().unwrap_or_default();
        Self::check(
            self.map.level_for_board(&board.id, &folder_id, &owner),
            needed,
            BOARD_NOT_FOUND,
        )
    }

    /// Require `needed` on the folder `id` (`""` = the root).
    ///
    /// # Errors
    ///
    /// 404 when the caller has no level on it, 403 when it is below `needed`.
    pub(crate) fn require_on_folder(&self, id: &str, needed: Level) -> Result<(), ApiError> {
        if id.is_empty() {
            return self.require_root(needed);
        }
        Self::check(self.map.level_for_folder(id), needed, FOLDER_NOT_FOUND)
    }

    fn check(level: Option<Level>, needed: Level, missing: &str) -> Result<(), ApiError> {
        match level {
            None => Err(ApiError::NotFound(missing.to_owned())),
            Some(level) if level < needed => Err(lacks(needed)),
            Some(_) => Ok(()),
        }
    }

    /// For a dashboard that may be in the trash (so [`require_board_level`]'s
    /// live lookup cannot find it): `403` when the caller can see where it
    /// sits and holds less than `needed` there. An unknown dashboard or one
    /// they have no level on passes, so the caller's own lookup gives the
    /// `404` it always gave (nothing new is confirmed).
    ///
    /// # Errors
    ///
    /// 403 for a dashboard the caller sees and may not do this to.
    pub(crate) fn refuse_below(
        &self,
        placements: &[BoardPlacement],
        id: &str,
        needed: Level,
    ) -> Result<(), ApiError> {
        let level = placements
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| self.map.level_for_board(&p.id, &p.folder_id, &p.owner_id));
        match level {
            Some(level) if level < needed => Err(lacks(needed)),
            _ => Ok(()),
        }
    }
}

/// Require `needed` on dashboard `id` for a write, and return its row. The
/// built-in dashboard (`default`, or blank) is judged at the root and returns
/// `None`.
///
/// # Errors
///
/// 404 `dashboard not found.` for an unknown, trashed or invisible dashboard
/// (one answer, so an id guess learns nothing); 403 for one the caller sees
/// and may not do this to; the classified store error otherwise.
pub(crate) async fn require_board_level(
    state: &AppState,
    scope: &Scope,
    id: &str,
    needed: Level,
) -> Result<Option<lakehouse_bi::store::Board>, ApiError> {
    let id = id.trim();
    if id.is_empty() || id == store::DEFAULT_BOARD_ID {
        // The built-in dashboard belongs to the starting point, like a chart
        // filed without a board (`require_chart_board`).
        scope.require_known(None, needed)?;
        return Ok(None);
    }
    let board = store::get_board(&state.clickhouse, id, &Viewer::system())
        .await
        .map_err(|e| classify_ch_error(&e))?
        .ok_or_else(|| ApiError::NotFound(BOARD_NOT_FOUND.to_owned()))?;
    scope.require_on_board(&board, needed)?;
    Ok(Some(board))
}

/// The [`Scope`] the dashboard payload is built from: the viewer it reads as and
/// the levels its `access.level` is worked out from, resolved once per request
/// (`BI-19` B2 review note: the level used to cost a second resolution).
///
/// A caller who holds `dashboard:read` has the same starting point for a read as
/// for a write, so one resolution serves both. Somebody who holds only rules
/// reads as [`viewer_for`] says (no starting point), and the level comes from
/// the write resolution.
///
/// # Errors
///
/// As [`viewer_for`].
pub(crate) async fn payload_scope(
    state: &AppState,
    principal: &Principal,
) -> Result<Scope, ApiError> {
    if principal.has(ENTRY_PERMISSION) {
        return scope_for_write(state, principal).await;
    }
    let viewer = viewer_for(state, principal).await?;
    let mut scope = scope_for_write(state, principal).await?;
    scope.viewer = viewer;
    Ok(scope)
}

/// The caller's level on dashboard `id` for the payload's `access.level`, so the
/// console shows only what the level allows (`BI-19` part B2, T9). The server
/// enforces it either way; this only decides what the page offers. The reader
/// already passed the read check, so anything that cannot be worked out reads as
/// `view` (never a wider offer). `all`, `default` and blank are judged at the
/// root.
pub(crate) async fn payload_level(state: &AppState, scope: &Scope, id: &str) -> Level {
    let id = id.trim();
    let level = if id.is_empty() || id == store::DEFAULT_BOARD_ID || id == "all" {
        scope.root_level()
    } else {
        match store::get_board(&state.clickhouse, id, &Viewer::system()).await {
            Ok(Some(board)) => scope.map.level_for_board(
                &board.id,
                &board.folder_id.clone().unwrap_or_default(),
                &board.owner_id.clone().unwrap_or_default(),
            ),
            _ => None,
        }
    };
    level.unwrap_or(Level::View)
}

/// Whether the caller manages anything at all: the starting point is manage, or
/// a rule names them with `manage`. The People panel's recipient picker asks it
/// (an item-less question, as the picker is asked before an item is chosen);
/// the item itself is checked on save.
///
/// # Errors
///
/// The classified store error.
pub(crate) async fn manages_something(
    state: &AppState,
    principal: &Principal,
) -> Result<bool, ApiError> {
    if principal.has("identity:write") || principal.has("dashboard:write") {
        return Ok(true);
    }
    let (grants, _) = read_rules(state, principal).await?;
    Ok(grants.iter().any(|g| g.level == Level::Manage))
}

async fn resolve(
    state: &AppState,
    principal: &Principal,
    baseline: Option<Level>,
) -> Result<Viewer, ApiError> {
    let (grants, marks) = read_rules(state, principal).await?;
    if baseline.is_none() && grants.is_empty() {
        return Err(refused());
    }
    // An administrator sees everything today, and keeps doing so; a person
    // with a starting point and no marks anywhere has nothing taken away.
    if principal.has("identity:write") || (baseline.is_some() && marks.is_empty()) {
        return Ok(viewer_of(principal));
    }
    let ch = &state.clickhouse;
    let tree = folders::list_folders(ch, &Viewer::system())
        .await
        .map_err(|e| classify_ch_error(&e))?;
    let placements = store::board_placements(ch)
        .await
        .map_err(|e| classify_ch_error(&e))?;
    AccessMap::new(
        principal.id.uuid().to_string(),
        false,
        baseline,
        &tree,
        &grants,
        &marks,
    )
    .viewer(&placements)
    .map_err(too_many)
}

/// Whether dashboard `board_id` is restricted ("only the people listed"):
/// by its own mark or a folder above it. What leaves the console to people the
/// rules do not name (a digest, a report to an outside address) is refused for
/// such a dashboard (`BI-19` B1 review R6/R12, owner decision Q-R12). Without
/// Postgres nothing could have been marked; an unknown dashboard is not
/// restricted (the caller's own checks answer it).
///
/// # Errors
///
/// The classified store error; the caller treats it as "restricted".
pub(crate) async fn board_is_restricted(
    pg: Option<&lakehouse_store::PgPool>,
    ch: &lakehouse_clickhouse::ChClient,
    board_id: &str,
) -> Result<bool, ApiError> {
    let Some(pg) = pg else {
        return Ok(false);
    };
    let marks: Vec<(ItemKind, String)> = rules::all_restrictions(pg)
        .await
        .map_err(|e| store_failed(&e))?
        .iter()
        .filter_map(|m| Some((ItemKind::parse(&m.item_kind)?, m.item_id.clone())))
        .collect();
    if marks.is_empty() {
        return Ok(false);
    }
    let Some(board) = store::get_board(ch, board_id, &Viewer::system())
        .await
        .map_err(|e| classify_ch_error(&e))?
    else {
        return Ok(false);
    };
    let tree = folders::list_folders(ch, &Viewer::system())
        .await
        .map_err(|e| classify_ch_error(&e))?;
    let folder_id = board.folder_id.unwrap_or_default();
    Ok(
        AccessMap::new(String::new(), false, None, &tree, &[], &marks)
            .board_is_restricted(board_id, &folder_id),
    )
}

/// The viewer for the places that only *count or name* what uses something
/// (metric, field, parameter and source usage, delete conflicts), `BI-19` B1
/// review R3. A caller who holds neither `dashboard:read` nor a rule (a
/// `metric:write` holder, say) is not refused, as it is not a dashboard read,
/// but sees no dashboard: every chart of a dashboard they cannot see turns
/// into "a dashboard you cannot see", never its title (never the baseline).
/// With nobody attached the answer is the same.
///
/// # Errors
///
/// 503/500 as [`viewer_for`] when the rules cannot be read or are too many;
/// the caller then gets no answer rather than a wider one.
pub(crate) async fn usage_viewer(
    state: &AppState,
    principal: Option<&Extension<Principal>>,
) -> Result<Viewer, ApiError> {
    match principal {
        Some(Extension(principal)) => usage_viewer_of(state, principal).await,
        None => Ok(Viewer::nothing()),
    }
}

/// [`usage_viewer`] for a handler that always has a principal.
///
/// # Errors
///
/// As [`usage_viewer`].
pub(crate) async fn usage_viewer_of(
    state: &AppState,
    principal: &Principal,
) -> Result<Viewer, ApiError> {
    match viewer_for(state, principal).await {
        Err(ApiError::PermissionDenied(_)) => Ok(Viewer::nothing()),
        other => other,
    }
}

// ── Who may see and change a dashboard's permissions (T7, plan D10) ────────

const NEEDS_DATABASE: &str = "Permissions need the database.";
const BODY_SHAPE: &str = "body must be {\"kind\": \"board\"|\"folder\", \"id\": string, \"rules\": [{\"userId\"|\"roleId\": string, \"level\": \"view\"|\"edit\"|\"manage\"}], \"restricted\": boolean}";
const NOT_MANAGER: &str = "manage on this item";
const FOLDER_NOT_FOUND: &str = "folder not found.";
const UNKNOWN_GRANTEE: &str = "One of the people or roles you chose does not exist.";
const LISTED_TWICE: &str = "A person or role is listed twice.";
const TOO_MANY_RULES: &str = "An item can list at most 200 people and roles.";
const LEVEL_TOO_HIGH: &str = "You cannot give a level higher than your own.";
const PERSONAL_SPACE: &str =
    "The personal space cannot be shared this way. Move the dashboard to a folder first.";
const BUILT_IN: &str = "The built-in dashboard has no permissions of its own.";
const AUDIT_DOWN: &str = "the audit store is unavailable";
const MAX_RULES: usize = 200;

/// What the caller can learn and do on one item, worked out once.
struct Subject {
    kind: ItemKind,
    id: String,
    /// Folders from the item's own folder up to the root, nearest first.
    ancestors: Vec<Folder>,
    level: Level,
}

fn pool_of(state: &AppState) -> Result<&lakehouse_store::PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable(NEEDS_DATABASE.to_owned()))
}

fn store_failed(err: &lakehouse_store::StoreError) -> ApiError {
    // `StoreError` reads "database error"; the source goes to the log (SEC-11).
    upstream_error::internal_error(&upstream_error::STORE, err)
}

/// The levels `caller` may give: none above their own (feature B6). Pure, so
/// the rule is tested without a database.
fn check_levels(caller: Level, requested: &[Level]) -> Result<(), ApiError> {
    if requested.iter().any(|l| *l > caller) {
        return Err(ApiError::PermissionDenied(LEVEL_TOO_HIGH.to_owned()));
    }
    Ok(())
}

/// The folders above `folder_id`, nearest first, stopping at the root, at an
/// unknown folder, or after as many steps as a tree may have.
fn ancestors_of(tree: &[Folder], folder_id: &str) -> Vec<Folder> {
    let mut out: Vec<Folder> = Vec::new();
    let mut next = folder_id.to_owned();
    while !next.is_empty() && out.len() <= folders::MAX_FOLDER_DEPTH {
        let Some(found) = tree.iter().find(|f| f.id == next) else {
            break;
        };
        out.push(found.clone());
        next.clone_from(&found.parent_id);
    }
    out
}

/// The caller's own map: the rules that name them, every mark, the folder
/// tree, their starting point.
async fn access_map(
    state: &AppState,
    principal: &Principal,
) -> Result<(AccessMap, Vec<Folder>), ApiError> {
    let (grants, marks) = read_rules(state, principal).await?;
    let tree = folders::list_folders(&state.clickhouse, &Viewer::system())
        .await
        .map_err(|e| classify_ch_error(&e))?;
    let map = AccessMap::new(
        principal.id.uuid().to_string(),
        principal.has("identity:write"),
        baseline_of(principal),
        &tree,
        &grants,
        &marks,
    );
    Ok((map, tree))
}

/// The item a call is about, when the caller manages it. The same `404` for an
/// item that does not exist, is in the trash, or the caller cannot see (an id
/// guess learns nothing); `403` for one they can see and do not manage; `400`
/// for the personal space and the built-in dashboard.
async fn managed_subject(
    state: &AppState,
    principal: &Principal,
    kind: ItemKind,
    id: &str,
) -> Result<Subject, ApiError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(ApiError::BadRequest("id is required.".to_owned()));
    }
    // BI-19 B2 (PR #250 CI): a caller who holds no rule and no starting point
    // for managing anything is refused before `ClickHouse` is asked, so the
    // answer does not depend on the analytics store being up (`route_auth`).
    require_can_ever(state, principal, Level::Manage).await?;
    let (map, tree) = access_map(state, principal).await?;
    if !map.is_entitled() {
        return Err(refused());
    }
    let missing = || {
        ApiError::NotFound(
            if kind == ItemKind::Board {
                BOARD_NOT_FOUND
            } else {
                FOLDER_NOT_FOUND
            }
            .to_owned(),
        )
    };
    let (level, folder_id, owner) = match kind {
        ItemKind::Board => {
            if id == store::DEFAULT_BOARD_ID {
                return Err(ApiError::BadRequest(BUILT_IN.to_owned()));
            }
            let board = store::get_board(&state.clickhouse, id, &Viewer::system())
                .await
                .map_err(|e| classify_ch_error(&e))?
                .ok_or_else(missing)?;
            let folder_id = board.folder_id.clone().unwrap_or_default();
            let owner = board.owner_id.unwrap_or_default();
            (
                map.level_for_board(id, &folder_id, &owner),
                folder_id,
                owner,
            )
        }
        ItemKind::Folder => {
            let folder = tree.iter().find(|f| f.id == id).ok_or_else(missing)?;
            (
                map.level_for_folder(id),
                folder.parent_id.clone(),
                folder.owner_id.clone(),
            )
        }
    };
    let Some(level) = level else {
        return Err(missing());
    };
    if !owner.is_empty() {
        return Err(ApiError::BadRequest(PERSONAL_SPACE.to_owned()));
    }
    if level < Level::Manage {
        return Err(ApiError::PermissionDenied(NOT_MANAGER.to_owned()));
    }
    Ok(Subject {
        kind,
        id: id.to_owned(),
        ancestors: ancestors_of(&tree, &folder_id),
        level,
    })
}

fn parse_kind(raw: &str) -> Result<ItemKind, ApiError> {
    ItemKind::parse(raw.trim())
        .ok_or_else(|| ApiError::BadRequest("kind must be board or folder.".to_owned()))
}

/// The item's rules and mark as the People tab shows them: its own, the
/// inherited ones with the folder each comes from, and whether a folder above
/// is marked.
async fn access_json(state: &AppState, subject: &Subject) -> Result<Value, ApiError> {
    let pool = pool_of(state)?;
    let own = rules::rules_for_item(pool, subject.kind.as_str(), &subject.id)
        .await
        .map_err(|e| store_failed(&e))?;
    let own_mark = rules::restriction_for_item(pool, subject.kind.as_str(), &subject.id)
        .await
        .map_err(|e| store_failed(&e))?;
    let above_ids: Vec<String> = subject.ancestors.iter().map(|f| f.id.clone()).collect();
    let marked_above: HashSet<String> = rules::all_restrictions(pool)
        .await
        .map_err(|e| store_failed(&e))?
        .into_iter()
        .filter(|m| m.item_kind == "folder" && above_ids.contains(&m.item_id))
        .map(|m| m.item_id)
        .collect();
    // BI-19 B1 owner decision Q-R14: a restriction ends the rules inherited
    // from above it, as `AccessMap` resolves them. An item that is itself
    // restricted inherits nothing; otherwise the folders count up to and
    // including the nearest restricted one (ancestors are nearest first).
    let inherited_from: Vec<String> = if own_mark.is_some() {
        Vec::new()
    } else {
        let end = above_ids
            .iter()
            .position(|id| marked_above.contains(id))
            .map_or(above_ids.len(), |i| i + 1);
        above_ids[..end].to_vec()
    };
    let inherited = rules::rules_for_items(pool, "folder", &inherited_from)
        .await
        .map_err(|e| store_failed(&e))?;
    let users: Vec<Uuid> = own
        .iter()
        .chain(&inherited)
        .filter_map(|r| r.user_id)
        .collect();
    let roles: Vec<Uuid> = own
        .iter()
        .chain(&inherited)
        .filter_map(|r| r.role_id)
        .collect();
    let user_rows = rules::user_names(pool, &users)
        .await
        .map_err(|e| store_failed(&e))?;
    let role_rows = rules::role_names(pool, &roles)
        .await
        .map_err(|e| store_failed(&e))?;
    let grantee = |r: &rules::AccessRule| -> Value {
        if let Some(user) = r.user_id {
            let found = user_rows.iter().find(|(id, _, _)| *id == user);
            json!({
                "kind": "user", "id": user,
                "name": found.map_or("", |(_, name, _)| name.as_str()),
                "email": found.map_or("", |(_, _, email)| email.as_str()),
                "level": r.level,
            })
        } else {
            let role = r.role_id.unwrap_or_default();
            let found = role_rows.iter().find(|(id, _)| *id == role);
            json!({
                "kind": "role", "id": role,
                "name": found.map_or("", |(_, name)| name.as_str()),
                "level": r.level,
            })
        }
    };
    let folder_name = |id: &str| {
        subject
            .ancestors
            .iter()
            .find(|f| f.id == id)
            .map_or_else(String::new, |f| f.name.clone())
    };
    Ok(json!({
        "kind": subject.kind.as_str(),
        "id": subject.id,
        "yourLevel": subject.level.as_str(),
        "restricted": own_mark.is_some(),
        "rules": own.iter().map(grantee).collect::<Vec<_>>(),
        "inherited": inherited.iter().map(|r| {
            let mut v = grantee(r);
            v["folderId"] = json!(r.item_id);
            v["folderName"] = json!(folder_name(&r.item_id));
            v
        }).collect::<Vec<_>>(),
        "restrictedAbove": subject.ancestors.iter()
            .filter(|f| marked_above.contains(&f.id))
            .map(|f| json!({ "folderId": f.id, "folderName": f.name }))
            .collect::<Vec<_>>(),
    }))
}

/// Query of `GET /api/dashboard/access`.
#[derive(Debug, Deserialize)]
pub struct AccessQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

/// `GET /api/dashboard/access?kind=&id=` — who has what on a dashboard or
/// folder: its own rules, the mark, the rules inherited from the folders above
/// (each with its folder), and the caller's own level.
///
/// # Errors
///
/// `403` for somebody with neither `dashboard:read` nor a rule, and for one who
/// can see the item without managing it; `404` for an item that is unknown, in
/// the trash or invisible to the caller; `400` for the personal space, the
/// built-in dashboard or a bad kind; `503` without a database.
pub async fn get_access(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<AccessQuery>,
) -> ApiResult<ApiJson<Value>> {
    let kind = parse_kind(q.kind.as_deref().unwrap_or(""))?;
    let subject = managed_subject(&state, &principal, kind, q.id.as_deref().unwrap_or("")).await?;
    Ok(ApiJson(access_json(&state, &subject).await?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleInput {
    #[serde(default)]
    user_id: Option<Uuid>,
    #[serde(default)]
    role_id: Option<Uuid>,
    level: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutAccessBody {
    kind: String,
    id: String,
    rules: Vec<RuleInput>,
    restricted: bool,
}

/// The rules as stored: exactly one grantee each, a known level, no one twice.
fn checked_rules(input: &[RuleInput]) -> Result<(Vec<NewRule>, Vec<Level>), ApiError> {
    if input.len() > MAX_RULES {
        return Err(ApiError::BadRequest(TOO_MANY_RULES.to_owned()));
    }
    let mut seen: HashSet<(bool, Uuid)> = HashSet::new();
    let mut out = Vec::with_capacity(input.len());
    let mut levels = Vec::with_capacity(input.len());
    for r in input {
        let level = Level::parse(r.level.trim())
            .ok_or_else(|| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
        let key = match (r.user_id, r.role_id) {
            (Some(user), None) => (true, user),
            (None, Some(role)) => (false, role),
            _ => return Err(ApiError::BadRequest(BODY_SHAPE.to_owned())),
        };
        if !seen.insert(key) {
            return Err(ApiError::BadRequest(LISTED_TWICE.to_owned()));
        }
        out.push(NewRule {
            user_id: r.user_id,
            role_id: r.role_id,
            level: level.as_str().to_owned(),
        });
        levels.push(level);
    }
    Ok((out, levels))
}

/// `PUT /api/dashboard/access` with `{kind, id, rules, restricted}` — replace
/// the item's rules and its "only the people listed" mark.
///
/// # Errors
///
/// As [`get_access`]; also `400` for a body of another shape, a person or role
/// listed twice or that does not exist, more than 200 rules; `403` for a level
/// above the caller's own; `503` without a database or when the change cannot
/// be audited (it is audited before it is made).
pub async fn put_access(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: PutAccessBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let kind = parse_kind(&parsed.kind)?;
    let subject = managed_subject(&state, &principal, kind, &parsed.id).await?;
    let (new_rules, levels) = checked_rules(&parsed.rules)?;
    check_levels(subject.level, &levels)?;
    let pool = pool_of(&state)?;
    // BI-19 B1 review R11: every person and role must exist before anything is
    // audited, so a refused change leaves no "access_change" event behind.
    let wanted_users: Vec<Uuid> = new_rules.iter().filter_map(|r| r.user_id).collect();
    let wanted_roles: Vec<Uuid> = new_rules.iter().filter_map(|r| r.role_id).collect();
    let (found_users, found_roles) = (
        rules::user_names(pool, &wanted_users)
            .await
            .map_err(|e| store_failed(&e))?,
        rules::role_names(pool, &wanted_roles)
            .await
            .map_err(|e| store_failed(&e))?,
    );
    if wanted_users
        .iter()
        .any(|u| !found_users.iter().any(|(id, _, _)| id == u))
        || wanted_roles
            .iter()
            .any(|r| !found_roles.iter().any(|(id, _)| id == r))
    {
        return Err(ApiError::BadRequest(UNKNOWN_GRANTEE.to_owned()).into());
    }
    let before = rules::rules_for_item(pool, kind.as_str(), &subject.id)
        .await
        .map_err(|e| store_failed(&e))?;
    let was_restricted = rules::restriction_for_item(pool, kind.as_str(), &subject.id)
        .await
        .map_err(|e| store_failed(&e))?
        .is_some();
    // Ids and levels only: no names, no addresses in the audit trail.
    let listed = |rows: &[(Option<Uuid>, Option<Uuid>, String)]| -> Vec<Value> {
        rows.iter()
            .map(|(user, role, level)| match user {
                Some(id) => json!({ "kind": "user", "id": id, "level": level }),
                None => json!({ "kind": "role", "id": role, "level": level }),
            })
            .collect()
    };
    let before_rows: Vec<_> = before
        .iter()
        .map(|r| (r.user_id, r.role_id, r.level.clone()))
        .collect();
    let after_rows: Vec<_> = new_rules
        .iter()
        .map(|r| (r.user_id, r.role_id, r.level.clone()))
        .collect();
    crate::routes::dashboard::record_download_audit(
        &state,
        &principal,
        (
            "dashboard.access_change",
            if kind == ItemKind::Board {
                "dashboard"
            } else {
                "dashboard_folder"
            },
            &subject.id,
        ),
        json!({
            "before": listed(&before_rows),
            "after": listed(&after_rows),
            "wasRestricted": was_restricted,
            "restricted": parsed.restricted,
        }),
    )
    .await
    .map_err(|_| ApiError::Unavailable(AUDIT_DOWN.to_owned()))?;
    let by = match principal.id {
        PrincipalId::User(id) => Some(id),
        // `granted_by` points at a person; a service identity is not one.
        PrincipalId::Service(_) => None,
    };
    rules::replace_item(
        pool,
        kind.as_str(),
        &subject.id,
        &new_rules,
        parsed.restricted,
        by,
    )
    .await
    .map_err(|err| match err {
        lakehouse_store::StoreError::ForeignKeyViolation => {
            ApiError::BadRequest(UNKNOWN_GRANTEE.to_owned())
        }
        other => store_failed(&other),
    })?;
    Ok(ApiJson(access_json(&state, &subject).await?))
}

// ── What a move changes (T7, feature B7) ───────────────────────────────────

/// A person or role, by kind and id.
type GranteeKey = (bool, Uuid);

fn keys_of(rules: &[rules::AccessRule]) -> Vec<GranteeKey> {
    let mut out: Vec<GranteeKey> = rules
        .iter()
        .filter_map(|r| match (r.user_id, r.role_id) {
            (Some(user), None) => Some((true, user)),
            (None, Some(role)) => Some((false, role)),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// What moving a dashboard from one folder chain to another does to who sees
/// it: the people and roles the destination's counting folders list and the old
/// ones did not (they gain), the reverse (they lose), and whether the starting
/// point (everyone whose role gives them dashboards) is lost or regained
/// because a mark is on one chain and not the other. The dashboard's own rules
/// and mark travel with it, so they are not part of the difference. Pure.
fn move_difference(
    from: (&[rules::AccessRule], bool),
    to: (&[rules::AccessRule], bool),
) -> (Vec<GranteeKey>, Vec<GranteeKey>, bool, bool) {
    let (before, after) = (keys_of(from.0), keys_of(to.0));
    let gains = after
        .iter()
        .filter(|k| !before.contains(k))
        .copied()
        .collect();
    let loses = before
        .iter()
        .filter(|k| !after.contains(k))
        .copied()
        .collect();
    (gains, loses, to.1 && !from.1, from.1 && !to.1)
}

/// The folders whose rules count for an item that sits under `chain` (nearest
/// first), and whether the item is restricted by one of them: the rules count
/// up to and including the nearest marked folder, the way
/// [`lakehouse_bi::access::AccessMap`] resolves a level (`BI-19` B1 owner
/// decision Q-R14; `BI-19` B2 review S2). Pure.
fn counting_chain<'a>(chain: &'a [Folder], marked: &[String]) -> (&'a [Folder], bool) {
    match chain.iter().position(|f| marked.contains(&f.id)) {
        Some(at) => (&chain[..=at], true),
        None => (chain, false),
    }
}

/// The rules that count on a chain (see [`counting_chain`]) and whether it is
/// restricted.
async fn chain_rules(
    pool: &lakehouse_store::PgPool,
    chain: &[Folder],
) -> Result<(Vec<rules::AccessRule>, bool), ApiError> {
    let marked: Vec<String> = rules::all_restrictions(pool)
        .await
        .map_err(|e| store_failed(&e))?
        .into_iter()
        .filter(|m| m.item_kind == "folder")
        .map(|m| m.item_id)
        .collect();
    let (counted, restricted) = counting_chain(chain, &marked);
    let ids: Vec<String> = counted.iter().map(|f| f.id.clone()).collect();
    let found = rules::rules_for_items(pool, "folder", &ids)
        .await
        .map_err(|e| store_failed(&e))?;
    Ok((found, restricted))
}

/// Query of `GET /api/dashboard/access-preview`.
#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    folder: Option<String>,
}

/// `GET /api/dashboard/access-preview?board=&folder=` — who gains and who loses
/// access to a dashboard if it moves into `folder` (`""` = the top level).
/// Needs manage on the dashboard, like the move itself will.
///
/// # Errors
///
/// As [`get_access`]; `404` for a destination folder the caller cannot see and
/// `400` for a personal space (a move there makes the dashboard private).
pub async fn access_preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<PreviewQuery>,
) -> ApiResult<ApiJson<Value>> {
    let subject = managed_subject(
        &state,
        &principal,
        ItemKind::Board,
        q.board.as_deref().unwrap_or(""),
    )
    .await?;
    let destination = q.folder.as_deref().map(str::trim).unwrap_or_default();
    let tree = folders::list_folders(&state.clickhouse, &Viewer::system())
        .await
        .map_err(|e| classify_ch_error(&e))?;
    let to_chain = ancestors_of(&tree, destination);
    if !destination.is_empty() {
        let visible =
            folders::list_folders(&state.clickhouse, &viewer_for(&state, &principal).await?)
                .await
                .map_err(|e| classify_ch_error(&e))?;
        let Some(found) = visible.iter().find(|f| f.id == destination) else {
            return Err(ApiError::NotFound(FOLDER_NOT_FOUND.to_owned()).into());
        };
        if !found.owner_id.is_empty() {
            return Err(ApiError::BadRequest(PERSONAL_SPACE.to_owned()).into());
        }
    }
    let pool = pool_of(&state)?;
    // BI-19 B2 review S2 (Q-R14): a dashboard with its own mark has a fresh list
    // that no folder, old or new, adds to or takes from, so nothing changes for
    // anybody; the folders' rules only count for one that is not marked.
    let own_mark = rules::restriction_for_item(pool, "board", &subject.id)
        .await
        .map_err(|e| store_failed(&e))?;
    if own_mark.is_some() {
        return Ok(ApiJson(json!({
            "gains": [],
            "loses": [],
            "startingPointLoses": false,
            "startingPointGains": false,
        })));
    }
    let from = chain_rules(pool, &subject.ancestors).await?;
    let to = chain_rules(pool, &to_chain).await?;
    let (gains, loses, start_loses, start_gains) =
        move_difference((&from.0, from.1), (&to.0, to.1));
    let users: Vec<Uuid> = gains
        .iter()
        .chain(&loses)
        .filter(|k| k.0)
        .map(|k| k.1)
        .collect();
    let roles: Vec<Uuid> = gains
        .iter()
        .chain(&loses)
        .filter(|k| !k.0)
        .map(|k| k.1)
        .collect();
    let user_rows = rules::user_names(pool, &users)
        .await
        .map_err(|e| store_failed(&e))?;
    let role_rows = rules::role_names(pool, &roles)
        .await
        .map_err(|e| store_failed(&e))?;
    let named = |keys: &[GranteeKey]| -> Vec<Value> {
        keys.iter()
            .map(|(is_user, id)| {
                let name = if *is_user {
                    user_rows
                        .iter()
                        .find(|(u, _, _)| u == id)
                        .map(|(_, n, _)| n.clone())
                } else {
                    role_rows
                        .iter()
                        .find(|(r, _)| r == id)
                        .map(|(_, n)| n.clone())
                };
                json!({
                    "kind": if *is_user { "user" } else { "role" },
                    "id": id,
                    "name": name.unwrap_or_default(),
                })
            })
            .collect()
    };
    Ok(ApiJson(json!({
        "gains": named(&gains),
        "loses": named(&loses),
        "startingPointLoses": start_loses,
        "startingPointGains": start_gains,
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn input(user: Option<Uuid>, role: Option<Uuid>, level: &str) -> RuleInput {
        RuleInput {
            user_id: user,
            role_id: role,
            level: level.to_owned(),
        }
    }

    fn scope(baseline: Option<Level>, grants: &[Grant], marks: &[(ItemKind, String)]) -> Scope {
        let tree = [Folder {
            id: "f1".to_owned(),
            name: "f1".to_owned(),
            parent_id: String::new(),
            created_by: String::new(),
            updated_at: None,
            owner_id: String::new(),
        }];
        let map = AccessMap::new("u1", false, baseline, &tree, grants, marks);
        Scope {
            viewer: Viewer::user("u1", false),
            map,
        }
    }

    fn rule(kind: ItemKind, id: &str, level: Level) -> Grant {
        Grant {
            kind,
            item_id: id.to_owned(),
            level,
        }
    }

    fn folder(id: &str, parent: &str) -> Folder {
        Folder {
            id: id.to_owned(),
            name: id.to_owned(),
            parent_id: parent.to_owned(),
            created_by: String::new(),
            updated_at: None,
            owner_id: String::new(),
        }
    }

    /// `BI-19` B2 review S2 (Q-R14): the rules a move compares count up to and
    /// including the nearest marked folder, nearest first, and no further.
    #[test]
    fn a_chain_counts_up_to_and_including_its_nearest_marked_folder() {
        let chain = [folder("f3", "f2"), folder("f2", "f1"), folder("f1", "")];
        let ids = |counted: &[Folder]| counted.iter().map(|f| f.id.clone()).collect::<Vec<_>>();

        let (counted, restricted) = counting_chain(&chain, &[]);
        assert_eq!(ids(counted), ["f3", "f2", "f1"]);
        assert!(!restricted);

        let (counted, restricted) = counting_chain(&chain, &["f2".to_owned()]);
        assert_eq!(
            ids(counted),
            ["f3", "f2"],
            "f1 above the mark does not count"
        );
        assert!(restricted);

        // Two marks: only the nearest ends the list.
        let (counted, _) = counting_chain(&chain, &["f3".to_owned(), "f1".to_owned()]);
        assert_eq!(ids(counted), ["f3"]);
    }

    #[test]
    fn edit_on_one_folder_lets_a_person_edit_there_and_manage_nowhere_and_see_nothing_else() {
        let edit = scope(None, &[rule(ItemKind::Folder, "f1", Level::Edit)], &[]);
        assert!(edit.require_on_folder("f1", Level::Edit).is_ok());
        // Visible but not enough: 403, not 404.
        assert!(matches!(
            edit.require_on_folder("f1", Level::Manage),
            Err(ApiError::PermissionDenied(_))
        ));
        // Another folder and the root are not theirs: the same 404 as unknown.
        assert!(matches!(
            edit.require_on_folder("f2", Level::Edit),
            Err(ApiError::NotFound(_))
        ));
        assert!(matches!(
            edit.require_root(Level::Edit),
            Err(ApiError::NotFound(_))
        ));
    }

    #[test]
    fn a_viewer_by_the_starting_point_is_refused_every_write_and_a_writer_is_not() {
        let viewer = scope(Some(Level::View), &[], &[]);
        assert!(viewer.require_root(Level::View).is_ok());
        assert!(matches!(
            viewer.require_on_folder("f1", Level::Edit),
            Err(ApiError::PermissionDenied(_))
        ));
        let writer = scope(Some(Level::Manage), &[], &[]);
        assert!(writer.require_on_folder("f1", Level::Manage).is_ok());
        assert!(writer.require_root(Level::Edit).is_ok());
    }

    #[test]
    fn a_restriction_takes_the_starting_point_away_from_a_writer_below_it() {
        let marks = [(ItemKind::Folder, "f1".to_owned())];
        let writer = scope(Some(Level::Manage), &[], &marks);
        assert!(matches!(
            writer.require_on_folder("f1", Level::Edit),
            Err(ApiError::NotFound(_))
        ));
        assert!(writer.require_root(Level::Edit).is_ok());
    }

    #[test]
    fn a_trashed_dashboard_is_refused_with_403_only_when_the_caller_sees_it() {
        let editor = scope(None, &[rule(ItemKind::Folder, "f1", Level::Edit)], &[]);
        let placements = [
            BoardPlacement {
                id: "mine".to_owned(),
                folder_id: "f1".to_owned(),
                owner_id: String::new(),
            },
            BoardPlacement {
                id: "other".to_owned(),
                folder_id: "f2".to_owned(),
                owner_id: String::new(),
            },
        ];
        assert!(matches!(
            editor.refuse_below(&placements, "mine", Level::Manage),
            Err(ApiError::PermissionDenied(_))
        ));
        assert!(
            editor
                .refuse_below(&placements, "mine", Level::Edit)
                .is_ok()
        );
        // Not theirs and unknown pass: the restore's own 404 answers both alike.
        assert!(
            editor
                .refuse_below(&placements, "other", Level::Manage)
                .is_ok()
        );
        assert!(
            editor
                .refuse_below(&placements, "nope", Level::Manage)
                .is_ok()
        );
    }

    #[test]
    fn a_level_above_the_callers_own_is_refused() {
        assert!(check_levels(Level::Manage, &[Level::View, Level::Manage]).is_ok());
        assert!(check_levels(Level::Edit, &[Level::View, Level::Edit]).is_ok());
        assert!(matches!(
            check_levels(Level::Edit, &[Level::Manage]),
            Err(ApiError::PermissionDenied(_))
        ));
        assert!(matches!(
            check_levels(Level::View, &[Level::Edit]),
            Err(ApiError::PermissionDenied(_))
        ));
    }

    #[test]
    fn a_rule_names_one_grantee_a_known_level_and_nobody_twice() {
        let (a, r) = (Uuid::new_v4(), Uuid::new_v4());
        let (rules, levels) =
            checked_rules(&[input(Some(a), None, "view"), input(None, Some(r), "edit")]).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(levels, [Level::View, Level::Edit]);
        for bad in [
            input(Some(a), Some(r), "view"),
            input(None, None, "view"),
            input(Some(a), None, "owner"),
            input(Some(a), None, ""),
        ] {
            assert!(checked_rules(&[bad]).is_err());
        }
        assert!(
            checked_rules(&[input(Some(a), None, "view"), input(Some(a), None, "edit")]).is_err()
        );
        // The same id as a user and as a role are two different grantees.
        assert!(
            checked_rules(&[input(Some(a), None, "view"), input(None, Some(a), "view")]).is_ok()
        );
        let many: Vec<RuleInput> = (0..=MAX_RULES)
            .map(|_| input(Some(Uuid::new_v4()), None, "view"))
            .collect();
        assert!(checked_rules(&many).is_err());
    }

    #[test]
    fn the_folders_above_an_item_are_listed_nearest_first_and_a_loop_ends() {
        let f = |id: &str, parent: &str| Folder {
            id: id.to_owned(),
            name: id.to_owned(),
            parent_id: parent.to_owned(),
            created_by: String::new(),
            updated_at: None,
            owner_id: String::new(),
        };
        let tree = [f("a", ""), f("b", "a"), f("c", "b")];
        let ids = |v: Vec<Folder>| v.into_iter().map(|f| f.id).collect::<Vec<_>>();
        assert_eq!(ids(ancestors_of(&tree, "c")), ["c", "b", "a"]);
        assert_eq!(ancestors_of(&tree, "").len(), 0);
        assert_eq!(ancestors_of(&tree, "unknown").len(), 0);
        let looped = [f("x", "y"), f("y", "x")];
        assert!(ancestors_of(&looped, "x").len() <= folders::MAX_FOLDER_DEPTH + 1);
    }

    fn rule_for(user: Option<Uuid>, role: Option<Uuid>) -> rules::AccessRule {
        rules::AccessRule {
            item_kind: "folder".to_owned(),
            item_id: "f".to_owned(),
            user_id: user,
            role_id: role,
            level: "view".to_owned(),
            granted_by: None,
            granted_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn a_move_gains_what_the_destination_lists_and_loses_what_the_old_folder_listed() {
        let (ann, bob, readers) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let old = [rule_for(Some(ann), None), rule_for(Some(bob), None)];
        let new = [rule_for(Some(bob), None), rule_for(None, Some(readers))];
        let (gains, loses, start_loses, start_gains) =
            move_difference((&old, false), (&new, false));
        assert_eq!(gains, [(false, readers)]);
        assert_eq!(loses, [(true, ann)]);
        assert!(!start_loses && !start_gains);
        // Into a marked folder: the starting point is lost; out of one, regained.
        let (_, _, start_loses, start_gains) = move_difference((&[], false), (&[], true));
        assert!(start_loses && !start_gains);
        let (_, _, start_loses, start_gains) = move_difference((&[], true), (&[], false));
        assert!(!start_loses && start_gains);
        // Between two marked folders the starting point stays lost: no claim.
        let (_, _, start_loses, start_gains) = move_difference((&[], true), (&[], true));
        assert!(!start_loses && !start_gains);
        // Nothing listed on either side: nothing changes.
        let (gains, loses, ..) = move_difference((&[], false), (&[], false));
        assert_eq!(gains.len() + loses.len(), 0);
    }
}
