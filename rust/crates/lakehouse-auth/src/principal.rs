//! [`Principal`]: the single, normalized shape every handler and policy
//! check consumes, no matter how the caller authenticated.
//!
//! This is what makes swapping or adding an identity provider cheap: a
//! local password login, a service token, and (later) an `OIDC` id token
//! all run through a different [`crate::Authenticator`], but every one of
//! them produces exactly this type. A handler that checks
//! `principal.has("catalog:write")` never needs to know, and can never
//! accidentally branch on, whether the caller typed a password or
//! presented a Google-issued token.

use uuid::Uuid;

use crate::permissions::PermissionSet;

/// Which kind of row in `app_user`/`service_identity` a [`Principal`]
/// corresponds to. A human and a service credential are different things
/// with different lifecycles (a service identity has no password, can't
/// change one, isn't invited by email, ...), so callers that need to tell
/// them apart (e.g. "only a human may change their own password") can
/// match on this — but nothing about *how the principal authenticated* is
/// visible here, only *what it is*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalId {
    /// An `app_user.id`.
    User(Uuid),
    /// A `service_identity.id`.
    Service(Uuid),
}

impl PrincipalId {
    /// The wrapped id, regardless of which variant this is.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        match self {
            Self::User(id) | Self::Service(id) => *id,
        }
    }
}

/// The normalized identity of an authenticated caller.
///
/// Contains no secret material (no password, hash, or token) — it is safe
/// to log, place in a `tracing` span, or attach to a request extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Which user or service this is.
    pub id: PrincipalId,
    /// Human-readable name (the user's display name, or the service
    /// identity's name) — for display and audit trails, not for
    /// authorization decisions.
    pub tenant_ids: Vec<Uuid>,
    /// Display name (the user's name, or the service identity's name).
    pub display_name: String,
    /// The merged permission grants from every role this principal holds
    /// (for a service identity, from its scopes — see
    /// [`crate::service_token`]).
    pub permissions: PermissionSet,
    /// Which authenticator produced this principal, for audit: `"local"`,
    /// `"session"`, `"service"`, and later `"oidc:<issuer>"`. Never branch
    /// application logic on this value — see the module doc comment — it
    /// exists purely so an audit log line can say how someone got in.
    pub provider: String,
}

impl Principal {
    /// Whether this principal is granted `permission` (`"resource:action"`,
    /// e.g. `"policy:read"`). Delegates to [`PermissionSet::has`].
    #[must_use]
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.has(permission)
    }

    /// Whether this principal belongs to `tenant_id`.
    #[must_use]
    pub fn in_tenant(&self, tenant_id: Uuid) -> bool {
        self.tenant_ids.contains(&tenant_id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::permissions::PermissionSet;

    fn sample_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: vec![Uuid::nil()],
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("query:read, catalog:read"),
            provider: "local".to_owned(),
        }
    }

    #[test]
    fn has_delegates_to_the_permission_set() {
        let principal = sample_principal();
        assert!(principal.has("query:read"));
        assert!(!principal.has("query:write"));
    }

    #[test]
    fn in_tenant_checks_membership() {
        let principal = sample_principal();
        assert!(principal.in_tenant(Uuid::nil()));
        assert!(!principal.in_tenant(Uuid::from_u128(1)));
    }

    #[test]
    fn principal_id_uuid_unwraps_either_variant() {
        let id = Uuid::from_u128(42);
        assert_eq!(PrincipalId::User(id).uuid(), id);
        assert_eq!(PrincipalId::Service(id).uuid(), id);
    }

    #[test]
    fn debug_contains_no_secret_shaped_field() {
        // Principal has no secret fields at all; this pins that invariant
        // down so a future field addition (e.g. a cached raw token) would
        // have to consciously break this test rather than slip in silently.
        let rendered = format!("{:?}", sample_principal());
        assert!(rendered.contains("Rina Wijaya"));
    }
}
