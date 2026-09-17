//! Shared queries used by every authenticator: loading the normalized
//! [`Principal`] for an already-identified user.
//!
//! # Why this lives in `lakehouse-auth`, not `lakehouse-store`
//!
//! `lakehouse_store::identity` is a generic CRUD read model for the
//! console's Identity screens (list/create/delete users, roles, tenants) —
//! its `StoreError` mapping and access patterns exist to serve that screen,
//! not to enforce authentication-specific security policy. This crate's
//! repository functions have a different job and a different set of
//! invariants to hold: they must never let a lookup failure distinguish
//! "no such identity" from "wrong credential" (see
//! [`crate::error::AuthError`]'s doc comment), and every secret they touch
//! (a password, a session/service token, a stored hash) must stay wrapped
//! in [`crate::secret::Secret`] end to end. Bolting that policy onto
//! `lakehouse-store::identity` — a module serving four other domains with
//! no such requirement — would either weaken it there or force every
//! caller of that module to reason about auth-specific rules that don't
//! apply to it. Keeping it here colocates the policy with the types
//! ([`Principal`], [`crate::credential::Credential`],
//! [`crate::error::AuthError`]) it exists to serve.
//!
//! This module does reuse [`lakehouse_store::PgPool`] (a bare type alias
//! for `sqlx::PgPool`) rather than defining a second pool wrapper type, so
//! `AppState` (Task 3.2) can hand this crate the exact same pool
//! `lakehouse-store` uses without a wrapping/unwrapping dance.

use uuid::Uuid;

use crate::error::AuthError;
use crate::permissions::PermissionSet;
use crate::principal::{Principal, PrincipalId};

pub use lakehouse_store::PgPool;

/// Load the normalized [`Principal`] for `user_id`, tagging it with
/// `provider` (the authenticator that identified this user — `"local"`,
/// `"session"`, or a future `"oidc:*"`) and `must_change_password`.
///
/// `must_change_password` is taken as a parameter rather than queried
/// here: the caller (a `local` password verification, a session lookup,
/// an OIDC resolution) either already has this value from a query it ran
/// anyway (a `local` `auth_identity` row it just joined against), or
/// knows by construction that it must be `false` (a non-`local` login has
/// no `auth_identity.must_change_password` to be flagged on). Querying it
/// again in here would add a database round trip to the hottest path in
/// this crate — session validation, run on every authenticated request —
/// for a value the caller can supply for free.
///
/// Merges `role.permissions` across every role the user holds (see
/// [`crate::permissions`] for the parsing/merge semantics), folds in every
/// still-active `access_grant` (WS7 item E4 — see this function's own
/// "Access grants" section below), and collects every tenant the user
/// belongs to.
///
/// # Access grants (WS7 item E4) widen `permissions`, never `role_names`
///
/// An approved [`lakehouse_store::agents::AccessGrant`] (WS7 item E3) adds
/// ONE permission token to this call's returned [`Principal::permissions`]
/// — through the exact same [`PermissionSet::merge`] the role permissions
/// already go through, so a granted permission is indistinguishable from a
/// role-held one at every `principal.has(...)` check site. This is
/// deliberately the ONLY thing an access grant widens: [`Principal::role_names`]
/// stays exactly the user's real role memberships — an access grant never
/// fabricates a role the user was not actually assigned, matching
/// [`Principal::role_names`]'s own doc comment (WS7 item A1). Only a grant
/// with `revoked_at IS NULL AND expires_at > now()` is folded in — an
/// EXPIRED-but-not-yet-swept grant already stops applying here immediately
/// (the periodic revocation sweep, WS7 item E3 Step 4, is cleanup, not the
/// enforcement point), and a REVOKED grant never applies again.
///
/// This is one extra query on the hottest auth path (session validation,
/// run on every authenticated request) — justified the same way
/// `tenant_ids`'s own extra query already is: this file's existing pattern
/// of "one query per real-time-varying fact", rather than folding grants
/// into the role-permissions query and losing the ability to reason about
/// each source independently.
///
/// # Errors
///
/// Returns [`AuthError::NotFound`] if `user_id` does not name an
/// `app_user` row, or [`AuthError::Database`] on any other failure.
pub async fn load_principal_for_user(
    pool: &PgPool,
    user_id: Uuid,
    provider: String,
    must_change_password: bool,
) -> Result<Principal, AuthError> {
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM app_user WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    let Some((display_name,)) = name else {
        return Err(AuthError::NotFound);
    };

    let role_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.name, r.permissions FROM app_user_role ur \
         JOIN role r ON r.id = ur.role_id WHERE ur.user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let role_names: Vec<String> = role_rows.iter().map(|(name, _)| name.clone()).collect();

    // WS7 item E4: every still-active access_grant permission, each
    // treated as its own single-token PermissionSet (mirroring how a
    // role's own free-text `permissions` column parses into one) so it
    // merges through the SAME PermissionSet::merge call every role's
    // permissions already go through — never a second, parallel grant
    // mechanism (see this function's own doc comment).
    let grant_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT permission FROM access_grant \
         WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let permissions = PermissionSet::merge(
        role_rows
            .iter()
            .map(|(_, raw)| PermissionSet::parse(raw))
            .chain(grant_rows.iter().map(|(perm,)| PermissionSet::parse(perm))),
    );

    let tenant_ids: Vec<(Uuid,)> =
        sqlx::query_as("SELECT tenant_id FROM app_user_tenant WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(pool)
            .await?;

    Ok(Principal {
        id: PrincipalId::User(user_id),
        tenant_ids: tenant_ids.into_iter().map(|(id,)| id).collect(),
        display_name,
        permissions,
        provider,
        must_change_password,
        role_names,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Pure-logic regression: multi-role permission merging happens through
    /// [`PermissionSet::merge`], already covered in `permissions.rs`; this
    /// just pins down that [`load_principal_for_user`] is the only
    /// production caller expected to feed it multiple raw strings (the
    /// integration behaviour itself needs a live Postgres — see
    /// `tests/repository.rs`).
    #[test]
    fn merge_input_shape_matches_what_the_role_permissions_query_returns() {
        let rows: Vec<(String,)> = vec![
            ("query:read, catalog:read".to_owned(),),
            ("agent:approve".to_owned(),),
        ];
        let merged = PermissionSet::merge(rows.iter().map(|(raw,)| PermissionSet::parse(raw)));
        assert!(merged.has("query:read"));
        assert!(merged.has("agent:approve"));
    }
}
