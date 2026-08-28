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
/// `"session"`, or a future `"oidc:*"`).
///
/// Merges `role.permissions` across every role the user holds (see
/// [`crate::permissions`] for the parsing/merge semantics) and collects
/// every tenant the user belongs to.
///
/// # Errors
///
/// Returns [`AuthError::NotFound`] if `user_id` does not name an
/// `app_user` row, or [`AuthError::Database`] on any other failure.
pub async fn load_principal_for_user(
    pool: &PgPool,
    user_id: Uuid,
    provider: String,
) -> Result<Principal, AuthError> {
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM app_user WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    let Some((display_name,)) = name else {
        return Err(AuthError::NotFound);
    };

    let permission_strings: Vec<(String,)> = sqlx::query_as(
        "SELECT r.permissions FROM app_user_role ur \
         JOIN role r ON r.id = ur.role_id WHERE ur.user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let permissions = PermissionSet::merge(
        permission_strings
            .iter()
            .map(|(raw,)| PermissionSet::parse(raw)),
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
