//! [`Authenticator`]: the plug point every identity provider — local
//! password, session, service token, and (Task 3.5) `OIDC` — implements.

use async_trait::async_trait;

use crate::credential::Credential;
use crate::error::AuthError;
use crate::principal::Principal;

/// Turns a [`Credential`] into a [`Principal`].
///
/// Every implementor normalizes whatever the real world handed it (a
/// password, an opaque token, an id token's claims) into the exact same
/// [`Principal`] shape. A handler that calls `authenticate` and then reads
/// `principal.has(...)` never learns, and never needs to learn, which
/// implementor produced it.
///
/// # How an `OIDC` implementor plugs in (Task 3.5)
///
/// This is the seam the whole crate is built around, so it is worth
/// spelling out concretely. Adding Okta (or Google, Entra, Keycloak, any
/// generic `OIDC` provider) requires exactly one new type and no changes to
/// anything documented above it:
///
/// ```ignore
/// pub struct OidcAuthenticator {
///     issuer: String,           // e.g. "https://your-org.okta.com"
///     provider_label: String,   // e.g. "oidc:okta", used as Principal::provider
///     jwks: JwksClient,         // fetches/caches the provider's signing keys
///     pool: lakehouse_store::PgPool,
/// }
///
/// #[async_trait]
/// impl Authenticator for OidcAuthenticator {
///     fn provider_id(&self) -> &str { &self.provider_label }
///
///     async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
///         let Credential::Bearer(token) = credential else {
///             return Err(AuthError::UnsupportedCredential);
///         };
///         // 1. Verify the JWT's signature against `self.jwks` and validate
///         //    `iss`/`aud`/`exp` — ordinary JWT validation, no
///         //    lakehouse-auth types involved yet.
///         let claims = validate_id_token(&self.issuer, token.expose(), &self.jwks)?;
///         // 2. Resolve (or, on first login, create) the `auth_identity` row
///         //    for (self.provider_id(), claims.sub) — this is the ONLY
///         //    write this authenticator needs, and it is the same
///         //    find-or-link operation `session.rs`/`service_token.rs`
///         //    already use for their own providers.
///         let app_user_id = repository::find_or_link_identity(
///             &self.pool, self.provider_id(), &claims.sub, &claims.name, &claims.email,
///         ).await?;
///         // 3. Load the normalized Principal exactly like every other
///         //    authenticator does.
///         repository::load_principal_for_user(&self.pool, app_user_id, self.provider_id().to_owned(), false).await
///     }
/// }
/// ```
///
/// Concretely, what changes:
///
/// * **New file**: `oidc.rs` in this crate (or a new crate, if `OIDC`
///   pulls in a JWT/JWKS dependency this crate shouldn't carry for callers
///   that never use it) containing the type above.
/// * **New config**: issuer URL, client id, and whatever `JwksClient` needs
///   — plain configuration, not a schema or type change.
/// * **New `auth_identity` rows** at `provider = 'oidc:okta'` (or
///   `'oidc:google'`, `'oidc:entra'`, ...) — the *same* `auth_identity`
///   table `LocalPasswordAuthenticator` already writes to at `provider =
///   'local'`. No migration.
/// * **One line in Task 3.2's router wiring**: register the new
///   authenticator alongside the three in this crate.
///
/// What does **not** change: [`Principal`], [`Credential`] (already has
/// [`Credential::Bearer`] reserved for exactly this), [`AuthError`],
/// `auth_identity`'s schema, any existing [`Authenticator`] implementor, or
/// any handler that consumes a [`Principal`]. That is the seam this design
/// is judged against.
#[async_trait]
pub trait Authenticator: Send + Sync {
    /// A stable identifier for this authenticator, used as
    /// [`Principal::provider`] on success (e.g. `"local"`, `"session"`,
    /// `"service"`, or `"oidc:okta"` for a future `OIDC` implementor).
    fn provider_id(&self) -> &str;

    /// Attempt to turn `credential` into a [`Principal`].
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::UnsupportedCredential`] if `credential` is a
    /// variant this implementor does not handle (a caller trying several
    /// authenticators in turn should treat this as "not mine, try the
    /// next one"). Returns [`AuthError::InvalidCredentials`],
    /// [`AuthError::SessionInvalid`], or
    /// [`AuthError::ServiceCredentialInvalid`] if the credential is the
    /// right shape but does not authenticate. Returns
    /// [`AuthError::Database`] on an underlying storage failure.
    async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError>;
}
