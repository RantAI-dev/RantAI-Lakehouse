//! Service-to-service authentication: opaque tokens backing a
//! `service_identity` row.
//!
//! Mirrors [`crate::session`]'s never-store-the-raw-token design — see
//! that module's doc comment for why SHA-256-over-a-CSPRNG-token is the
//! right primitive here, not `Argon2`. The one structural difference is
//! that a service credential's validity window comes from
//! `service_identity.expires_at`/`rotation_status` (columns
//! `lakehouse_store::identity` already owns and the console's Service
//! Identities screen already surfaces), not a per-credential `expires_at`:
//! rotating a service credential's *lifetime* is the same operation as
//! rotating the identity's, so there is exactly one place that decides
//! when a service identity goes stale.

use async_trait::async_trait;
use uuid::Uuid;

use crate::authenticator::Authenticator;
use crate::credential::Credential;
use crate::error::AuthError;
use crate::permissions::PermissionSet;
use crate::principal::{Principal, PrincipalId};
use crate::repository::PgPool;
use crate::secret::Secret;
use crate::token::{generate_opaque_token, hash_token};

/// Issue a new token for `service_identity_id`.
///
/// Returns the raw token — only its hash is persisted.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on any storage failure (including the
/// referenced `service_identity` not existing, surfaced as a foreign-key
/// violation and classified by [`crate::error::AuthError::from<sqlx::Error>`]).
pub async fn create_service_credential(
    pool: &PgPool,
    service_identity_id: Uuid,
) -> Result<Secret, AuthError> {
    let token = generate_opaque_token();
    let token_hash = hash_token(&token);
    sqlx::query("INSERT INTO service_credential (service_identity_id, token_hash) VALUES ($1, $2)")
        .bind(service_identity_id)
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(token)
}

/// Revoke `token`. Idempotent, for the same reason
/// [`crate::session::revoke_session`] is — see its doc comment.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn revoke_service_credential(pool: &PgPool, token: &Secret) -> Result<(), AuthError> {
    let token_hash = hash_token(token);
    sqlx::query(
        "UPDATE service_credential SET revoked_at = now() \
         WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// Verify `token`, returning the [`Principal`] for the `service_identity`
/// it belongs to if the credential is unrevoked and the identity itself
/// hasn't expired.
///
/// The returned [`Principal::permissions`] are derived from
/// `service_identity.scopes` (e.g. `["query:read", "catalog:read"]`),
/// joined and parsed exactly like a role's `permissions` string — a scope
/// and a permission token share the same `resource:action` shape in this
/// data. [`Principal::tenant_ids`] is always empty: nothing in the schema
/// ties a `service_identity` to a tenant today, so reporting an empty list
/// is honest rather than guessed.
///
/// # Errors
///
/// Returns [`AuthError::ServiceCredentialInvalid`] if `token` doesn't hash
/// to a known, unrevoked credential whose `service_identity` has not
/// expired, or [`AuthError::Database`] on any other failure.
pub async fn verify_service_token(pool: &PgPool, token: &Secret) -> Result<Principal, AuthError> {
    let token_hash = hash_token(token);
    let row: Option<(Uuid, String, Vec<String>)> = sqlx::query_as(
        "SELECT si.id, si.name, si.scopes FROM service_credential sc \
         JOIN service_identity si ON si.id = sc.service_identity_id \
         WHERE sc.token_hash = $1 AND sc.revoked_at IS NULL AND si.expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    let Some((service_identity_id, name, scopes)) = row else {
        return Err(AuthError::ServiceCredentialInvalid);
    };
    let permissions = PermissionSet::parse(&scopes.join(","));
    Ok(Principal {
        id: PrincipalId::Service(service_identity_id),
        tenant_ids: Vec::new(),
        display_name: name,
        permissions,
        provider: "service".to_owned(),
        // A service identity has no `local` `auth_identity` row (and
        // cannot itself change a password), so there is nothing for this
        // flag to mean here — always `false`.
        must_change_password: false,
    })
}

/// [`Authenticator`] for [`Credential::ServiceToken`].
pub struct ServiceTokenAuthenticator {
    pool: PgPool,
}

impl ServiceTokenAuthenticator {
    /// Build an authenticator backed by `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Authenticator for ServiceTokenAuthenticator {
    // The trait returns `&str` tied to `&self`, not `&'static str`, because a
    // future `OidcAuthenticator` needs to return an owned, per-instance
    // provider label (e.g. "oidc:okta" built from config) -- see
    // `Authenticator`'s doc comment. This impl happens to return a literal.
    #[allow(clippy::unnecessary_literal_bound)]
    fn provider_id(&self) -> &str {
        "service"
    }

    async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
        let Credential::ServiceToken(token) = credential else {
            return Err(AuthError::UnsupportedCredential);
        };
        verify_service_token(&self.pool, token).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn scopes_join_into_a_parseable_permission_set() {
        let scopes = ["query:read".to_owned(), "catalog:read".to_owned()];
        let permissions = PermissionSet::parse(&scopes.join(","));
        assert!(permissions.has("query:read"));
        assert!(permissions.has("catalog:read"));
        assert!(!permissions.has("query:write"));
    }
}
