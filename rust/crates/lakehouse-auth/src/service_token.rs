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

/// Shared insert behind [`create_service_credential`]/
/// [`ensure_service_credential`] — one place that actually writes a
/// `service_credential` row, so both callers persist the exact same shape.
///
/// `ON CONFLICT (token_hash) DO NOTHING` makes this idempotent against the
/// table's own `service_credential_token_hash_unique` constraint: inserting
/// the same (already-hashed) token twice is a no-op, not an error — the
/// property [`ensure_service_credential`] needs to be safely re-run on
/// every process boot.
async fn insert_service_credential(
    pool: &PgPool,
    service_identity_id: Uuid,
    token_hash: &str,
) -> Result<(), AuthError> {
    sqlx::query(
        "INSERT INTO service_credential (service_identity_id, token_hash) VALUES ($1, $2) \
         ON CONFLICT (token_hash) DO NOTHING",
    )
    .bind(service_identity_id)
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

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
    insert_service_credential(pool, service_identity_id, &hash_token(&token)).await?;
    Ok(token)
}

/// Idempotently persist a caller-supplied `token` as `service_identity_id`'s
/// credential, hashing it the exact same way [`create_service_credential`]
/// hashes its own generated token.
///
/// This is the shape a config-driven bootstrap needs and
/// [`create_service_credential`] cannot provide: the caller (e.g.
/// `lakehouse-api`'s `main::bootstrap_agent_run_service`) already knows the
/// token's value — it came from an env var an operator set, not from this
/// crate's CSPRNG — so it must be *this exact token's* hash that lands in
/// `service_credential`, not a freshly generated one. Re-running this with
/// the same `token` on every process restart is a no-op (see
/// [`insert_service_credential`]'s doc comment): the credential is neither
/// duplicated nor invalidated.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on any storage failure.
pub async fn ensure_service_credential(
    pool: &PgPool,
    service_identity_id: Uuid,
    token: &Secret,
) -> Result<(), AuthError> {
    insert_service_credential(pool, service_identity_id, &hash_token(token)).await
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

/// How long a `last_used_at` write is skipped after the previous one,
/// per service identity. `verify_service_token` runs on the authentication
/// hot path — every authenticated service request, unconditionally, not
/// just an occasional admin action — so writing `last_used_at` on every
/// call would turn "read telemetry" into "every request also does a
/// write". 300 is chosen to match `crate::oidc::DEFAULT_JWKS_TTL`
/// (`oidc.rs`, also 300s) — not because the two are functionally related,
/// but because it is an already-reviewed, already-familiar cadence in this
/// same crate for "a value that is fine to be up to a few minutes stale",
/// and picking a second, unrelated number would need its own separate
/// justification this crate has no new evidence to give. A service
/// identity queried thousands of times a minute costs at most one write
/// per five minutes, not one per request; a rarely-used identity's
/// `last_used_at` still updates the first time it is used after a long
/// gap. Closes the J18/T13a "always-null" gap honestly.
const LAST_USED_AT_THROTTLE_SECONDS: i64 = 300;

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
/// On every successful verification this also issues a best-effort,
/// throttled `UPDATE service_identity SET last_used_at = now()` (see
/// [`LAST_USED_AT_THROTTLE_SECONDS`]). The throttle predicate is the
/// entire throttle: a row within the window simply matches zero rows and
/// the `UPDATE` is a silent no-op. An `Err` from the `UPDATE` is logged
/// with `tracing::warn!` and swallowed — authentication is never blocked
/// by telemetry.
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

    // Best-effort, throttled telemetry — never a reason to fail an
    // otherwise-valid credential. `last_used_at` is `NOT NULL DEFAULT
    // now()` since `0001_init.sql:125`, so the `WHERE` clause needs no
    // `OR last_used_at IS NULL` branch; a row within the window simply
    // matches zero rows and this is a silent no-op. See this fn's
    // doc comment for the swallow-and-log rule on errors.
    if let Err(err) = sqlx::query(
        "UPDATE service_identity SET last_used_at = now() \
         WHERE id = $1 AND last_used_at < now() - ($2 * INTERVAL '1 second')",
    )
    .bind(service_identity_id)
    .bind(LAST_USED_AT_THROTTLE_SECONDS)
    .execute(pool)
    .await
    {
        tracing::warn!(
            %err,
            service_identity_id = %service_identity_id,
            "failed to update service_identity.last_used_at; authentication continues"
        );
    }

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
        // A service identity's scopes are not roles; no authored policy
        // names a service principal by role today.
        role_names: Vec::new(),
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
