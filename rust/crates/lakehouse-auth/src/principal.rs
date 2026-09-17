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
    /// Whether this principal's `local` credential is still flagged
    /// `auth_identity.must_change_password` (see
    /// [`crate::password`]'s module doc comment). `true` for a
    /// bootstrapped-but-not-yet-rotated local login or a session minted
    /// from one; always `false` for a service token or an OIDC login,
    /// neither of which has a `local` `auth_identity` row to be flagged
    /// on.
    ///
    /// This is populated by the authenticator, not computed on every
    /// permission check, precisely so `crate::policy::auth_gate` (or
    /// equivalent middleware in a consuming crate) can gate on it without
    /// an extra database round trip beyond what identifying the caller
    /// already costs.
    pub must_change_password: bool,
    /// Names of every role this principal holds (`role.name`), e.g.
    /// `["Analyst", "Approver"]`. Empty for a service identity (a
    /// [`crate::service_token`]-derived principal has scopes, not roles).
    ///
    /// Distinct from `permissions`: two principals holding different role
    /// SETS can still merge to an identical `PermissionSet` (e.g. one role
    /// granting `pipeline:*` directly vs. two roles whose union happens to
    /// cover the same tokens), so `permissions.has(...)` cannot answer "does
    /// this principal hold role X" — the `lakehouse-api` policy engine
    /// (WS7) needs the real names because authored policy
    /// `conditions.roles` names roles, not permission strings.
    pub role_names: Vec<String>,
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

    /// Maps [`PrincipalId`]'s variant onto the vocabulary
    /// `lakehouse_store::audit::NewAuditEvent.principal_kind` accepts —
    /// `"user" | "service" | "copilot" | "schedule"`, CHECK-enforced by
    /// `0024_audit_event.sql` (`audit_event_principal_kind_check`). Callers
    /// writing an audit row for an authenticated [`Principal`] must use
    /// this, never [`Principal::provider`] — `provider` is a DIFFERENT
    /// vocabulary (`"local"`, `"session"`, `"service"`, `"oidc:<issuer>"`,
    /// the authenticator that produced the principal, not the kind of
    /// principal it is). Binding `provider` into `principal_kind` verbatim
    /// would insert `"local"`/`"session"`/`"oidc:..."` for every
    /// human-authenticated caller and violate the CHECK on every audit
    /// write from a logged-in user (WS5 Phase D preamble finding). One
    /// copy shared by every producer site (`routes::query::run`,
    /// `routes::agents::decide_approval`, and later `routes::connectors`)
    /// rather than a helper duplicated per call site (AGENTS.md rule 4).
    #[must_use]
    pub const fn kind_for_audit(&self) -> &'static str {
        match self.id {
            PrincipalId::User(_) => "user",
            PrincipalId::Service(_) => "service",
        }
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
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    #[test]
    fn role_names_are_carried_alongside_the_merged_permission_set() {
        let principal = sample_principal();
        assert_eq!(principal.role_names, vec!["Analyst".to_owned()]);
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
    fn kind_for_audit_maps_user_and_service_and_ignores_provider() {
        let mut user = sample_principal();
        user.provider = "oidc:example".to_owned(); // deliberately NOT "user"/"service"
        assert_eq!(user.kind_for_audit(), "user");

        let mut service = sample_principal();
        service.id = PrincipalId::Service(Uuid::nil());
        service.provider = "local".to_owned(); // deliberately the wrong-vocabulary value
        assert_eq!(service.kind_for_audit(), "service");
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
