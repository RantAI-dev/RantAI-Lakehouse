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

use lakehouse_auth::Principal;
use lakehouse_bi::access::{AccessMap, Grant, ItemKind, Level, TooManyIds, Viewer};
use lakehouse_bi::{folders, store};
use lakehouse_core::ApiError;
use lakehouse_store::dashboard_access as rules;

use crate::routes::dashboard_folders::classify_ch_error;
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;

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
    let baseline = baseline_of(principal);
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
