//! Task 3.5: `OIDC` identity-provider support — the concrete implementor
//! [`crate::Authenticator`]'s doc comment sketched.
//!
//! # What this proves
//!
//! The whole point of this module's existence, per the task it was built
//! under, is to test the seam [`crate::authenticator`] claims to offer: can
//! a real identity provider be wired in without touching [`crate::Principal`],
//! [`crate::Credential`], [`crate::AuthError`], `auth_identity`'s schema, or
//! any existing [`crate::Authenticator`]? It can — this module is one new
//! file plus config, exactly as promised. See this task's final report for
//! the complete list of what *did* have to change outside this file (the
//! router's [`crate::Authenticator`] registry and bearer-token dispatch,
//! which [`crate::authenticator`]'s doc comment already anticipated by
//! name).
//!
//! # What this module is NOT
//!
//! It is not an OIDC *client* in the full sense — there is no
//! authorization-code exchange, no `/callback` route, no refresh-token
//! handling. Those are login-UI concerns (out of this task's scope; see the
//! login/redirect flow, a separate task). This module is a *resource
//! server*: it verifies a bearer token some other piece of the system
//! (typically a frontend that already completed the `IdP`'s login redirect)
//! hands it, exactly the way [`crate::credential::Credential::Bearer`] was
//! always going to be consumed. That is also why [`OidcConfig`] carries no
//! client secret: verifying a signature against a provider's public JWKS
//! needs no shared secret, only the public key material the provider
//! already publishes.
//!
//! # Non-negotiables, and where each one lives
//!
//! * Signature verification against the provider's JWKS, never hand-rolled
//!   — [`jsonwebtoken`] does the actual cryptography; this module's job is
//!   choosing which key and which algorithm it's allowed to verify with.
//! * JWKS caching with rotation — [`JwksClient`].
//! * `iss`/`aud`/`exp`/`nbf` with configurable clock skew —
//!   [`OidcAuthenticator::validate_token`], via [`jsonwebtoken::Validation`].
//! * An algorithm allowlist that rejects `none` and blocks algorithm
//!   confusion — [`ALLOWED_ALGORITHMS`]; see its doc comment for exactly how.
//! * JIT provisioning is opt-in, default off — [`OidcConfig::jit_provisioning`].
//! * Claim-to-role mapping — [`OidcAuthenticator::mapped_permissions`]; see
//!   its doc comment for the union-with-local-roles precedence rule and why.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::authenticator::Authenticator;
use crate::credential::Credential;
use crate::error::AuthError;
use crate::permissions::PermissionSet;
use crate::principal::Principal;
use crate::repository::{self, PgPool};

/// Algorithms this authenticator will ever verify a signature with —
/// every one asymmetric.
///
/// Two attacks this list exists to close:
///
/// * **`alg: none`.** Not in this list, and cannot be: [`Algorithm`] (the
///   type [`jsonwebtoken::decode_header`] parses the header's `alg` field
///   into) has no `none` variant at all, so a token whose header literally
///   says `"alg":"none"` fails to *parse* before [`ALLOWED_ALGORITHMS`] is
///   even consulted — this is a type-level guarantee, not a runtime check
///   that could be forgotten.
/// * **Algorithm confusion (`RS256` -> `HS256`).** A JWKS publishes only
///   PUBLIC keys. If this list included an `HS*` (HMAC) algorithm, an
///   attacker could take a provider's public RSA key, feed its bytes to
///   HMAC-SHA256 as the "secret", and mint a token this authenticator would
///   otherwise accept — HMAC verification only requires *a* shared secret,
///   and the attacker now has one nobody meant to be secret. Excluding
///   every symmetric algorithm here makes that attack impossible
///   regardless of what an attacker puts in the header, and
///   [`jsonwebtoken::decode`] additionally refuses to verify at all unless
///   every algorithm in [`Validation::algorithms`] and the token's own
///   `alg` share the *same key family* as the [`DecodingKey`] in hand (see
///   `jsonwebtoken::decoding::verify_signature`), so a `DecodingKey` built
///   from an RSA JWK can never be used to satisfy an EC- or HMAC-family
///   `alg` claim either.
const ALLOWED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::ES256,
    Algorithm::ES384,
];

/// How long a fetched JWKS is trusted before [`JwksClient`] will fetch it
/// again, absent a cache miss forcing an earlier refetch (see
/// [`JwksClient::key_for`]'s doc comment for the rotation path).
const DEFAULT_JWKS_TTL: StdDuration = StdDuration::from_secs(300);

/// The default `leeway` (in seconds) applied to `exp`/`nbf` validation when
/// [`OidcConfig::clock_skew_seconds`] is not overridden. Matches
/// [`jsonwebtoken::Validation`]'s own default.
const DEFAULT_CLOCK_SKEW_SECONDS: u64 = 60;

/// Static, per-provider configuration for one [`OidcAuthenticator`].
///
/// Deliberately carries no client secret: this module verifies tokens
/// against a provider's public JWKS, which needs no shared secret — see
/// the module doc comment's "what this module is NOT" section. A caller
/// wiring in an authorization-code exchange later would carry that secret
/// in its own config, not here.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// The provider's issuer URL. Must match the token's `iss` claim
    /// exactly (byte-for-byte, not just "resolves to the same host") —
    /// this is deliberate: `iss` is compared as an opaque string, exactly
    /// as OIDC requires.
    pub issuer: String,
    /// This application's client id as registered with the provider. Must
    /// appear in the token's `aud` claim.
    pub client_id: String,
    /// A short, operator-chosen label for this provider (e.g. `"okta"`,
    /// `"entra"`, `"google"`). Combined with `"oidc:"` to form
    /// [`crate::Principal::provider`] and the `auth_identity.provider`
    /// value this authenticator reads and writes.
    pub provider_name: String,
    /// The URL [`JwksClient`] fetches the provider's signing keys from.
    pub jwks_url: String,
    /// Whether an unrecognized `sub` (no existing `auth_identity` row) is
    /// allowed to provision a new `app_user` on the spot. Defaults to
    /// `false` at the config layer — see [`crate::authenticator`] module
    /// doc's non-negotiables list for why the default matters: an
    /// over-broad issuer/audience configuration would otherwise let any
    /// token that merely validates mint an account.
    pub jit_provisioning: bool,
    /// Maps an external group/role claim value (e.g. an Okta group name)
    /// to a local `role.name`. Unmapped groups are ignored, not fatal —
    /// see [`OidcAuthenticator::mapped_permissions`].
    pub role_map: HashMap<String, String>,
    /// Which claim in the token carries the caller's groups/roles (e.g.
    /// `"groups"`, `"roles"`, or a provider-specific custom claim URI).
    pub groups_claim: String,
    /// Clock-skew tolerance (seconds) applied to `exp`/`nbf` validation.
    pub clock_skew_seconds: u64,
}

impl OidcConfig {
    /// Build a config with [`DEFAULT_CLOCK_SKEW_SECONDS`] clock skew, no
    /// role mapping, and JIT provisioning off — the safe defaults a caller
    /// only needs to override explicitly.
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        provider_name: impl Into<String>,
        jwks_url: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            provider_name: provider_name.into(),
            jwks_url: jwks_url.into(),
            jit_provisioning: false,
            role_map: HashMap::new(),
            groups_claim: "groups".to_owned(),
            clock_skew_seconds: DEFAULT_CLOCK_SKEW_SECONDS,
        }
    }

    /// The [`crate::Principal::provider`] / `auth_identity.provider` value
    /// this config's authenticator reads and writes: `"oidc:<provider_name>"`.
    #[must_use]
    pub fn provider_label(&self) -> String {
        format!("oidc:{}", self.provider_name)
    }
}

/// One cached JWKS fetch, timestamped so [`JwksClient`] knows when to
/// distrust it.
struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
}

/// Fetches and caches a provider's JSON Web Key Set.
///
/// # Why caching is load-bearing, not an optimization
///
/// Every bearer request this authenticator sees would otherwise trigger an
/// HTTP round trip to the provider before any cryptography even starts —
/// under load, that is a self-inflicted denial-of-service against the
/// provider (and, transitively, against every other caller depending on
/// it) mounted from this service's own traffic. A short TTL cache turns
/// "one fetch per request" into "one fetch per [`DEFAULT_JWKS_TTL`]
/// window, plus one extra fetch the instant a `kid` this service hasn't
/// seen shows up" — see [`Self::key_for`].
pub struct JwksClient {
    url: String,
    http: reqwest::Client,
    ttl: StdDuration,
    cache: RwLock<Option<CachedJwks>>,
}

impl JwksClient {
    /// Build a client fetching from `url`, caching for [`DEFAULT_JWKS_TTL`].
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_ttl(url, DEFAULT_JWKS_TTL)
    }

    /// Build a client fetching from `url`, caching for `ttl`. Exposed
    /// mainly for tests that need a short TTL to exercise expiry without
    /// waiting [`DEFAULT_JWKS_TTL`] in real time.
    #[must_use]
    pub fn with_ttl(url: impl Into<String>, ttl: StdDuration) -> Self {
        Self {
            url: url.into(),
            http: reqwest::Client::new(),
            ttl,
            cache: RwLock::new(None),
        }
    }

    /// Fetch the JWKS document fresh over HTTP. Never consults or updates
    /// the cache itself — callers decide when a fetch is warranted.
    async fn fetch(&self) -> Result<JwkSet, AuthError> {
        let response = self
            .http
            .get(&self.url)
            .send()
            .await
            .map_err(|_| AuthError::InvalidCredentials)?;
        if !response.status().is_success() {
            return Err(AuthError::InvalidCredentials);
        }
        response
            .json::<JwkSet>()
            .await
            .map_err(|_| AuthError::InvalidCredentials)
    }

    /// Look for `kid` in the cached JWKS, but only if the cache is still
    /// within [`Self::ttl`]. Returns `None` on a cold cache, an expired
    /// cache, or a `kid` the cached set doesn't contain — every one of
    /// those is a signal to refetch, not a hard failure, so this never
    /// itself returns an error.
    fn cached_key(&self, kid: &str) -> Option<DecodingKey> {
        let guard = self.cache.read().ok()?;
        let cached = guard.as_ref()?;
        if cached.fetched_at.elapsed() >= self.ttl {
            return None;
        }
        decoding_key_from_jwk(cached.set.find(kid)?).ok()
    }

    /// Resolve `kid` to a [`DecodingKey`], fetching the JWKS if (and only
    /// if) [`Self::cached_key`] can't serve it from cache.
    ///
    /// # Rotation
    ///
    /// A `kid` the cache doesn't recognize — because it's genuinely new (the
    /// provider rotated its signing key since the last fetch) or because the
    /// cache is simply cold or expired — always triggers exactly one
    /// refetch before giving up. This is what makes key rotation transparent
    /// to a caller: the *next* token signed with a freshly rotated key is
    /// accepted on its first presentation, without waiting out
    /// [`DEFAULT_JWKS_TTL`], and without any operator action. A `kid` the
    /// refreshed JWKS still doesn't contain (a genuinely unknown/attacker-
    /// supplied `kid`) is rejected — see the caller,
    /// [`OidcAuthenticator::validate_token`].
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidCredentials`] if the JWKS can't be
    /// fetched/parsed, or if the (possibly refetched) set has no key
    /// matching `kid` this crate knows how to build a [`DecodingKey`] from.
    async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(key) = self.cached_key(kid) {
            return Ok(key);
        }
        let set = self.fetch().await?;
        let key = set
            .find(kid)
            .and_then(|jwk| decoding_key_from_jwk(jwk).ok());
        if let Ok(mut guard) = self.cache.write() {
            *guard = Some(CachedJwks {
                set,
                fetched_at: Instant::now(),
            });
        }
        key.ok_or(AuthError::InvalidCredentials)
    }
}

/// Build a [`DecodingKey`] from one JWK entry. Only RSA and EC key types are
/// supported — the only families [`ALLOWED_ALGORITHMS`] ever needs, and the
/// only families every mainstream `OIDC` provider (Okta, Entra, Google,
/// Keycloak) publishes for `id_token` signing.
fn decoding_key_from_jwk(jwk: &Jwk) -> Result<DecodingKey, AuthError> {
    match &jwk.algorithm {
        AlgorithmParameters::RSA(params) => DecodingKey::from_rsa_components(&params.n, &params.e)
            .map_err(|_| AuthError::InvalidCredentials),
        AlgorithmParameters::EllipticCurve(params) => {
            DecodingKey::from_ec_components(&params.x, &params.y)
                .map_err(|_| AuthError::InvalidCredentials)
        }
        AlgorithmParameters::OctetKey(_) | AlgorithmParameters::OctetKeyPair(_) => {
            Err(AuthError::InvalidCredentials)
        }
    }
}

/// The claims this module reads out of a validated token. `sub`/`email`/
/// `name` are named fields; everything else (including whichever claim
/// [`OidcConfig::groups_claim`] names) lands in `extra`, since the claim
/// name carrying group membership varies per provider and is only known at
/// config time, not compile time.
///
/// Does NOT itself validate `iss`/`aud`/`exp`/`nbf` — that happens inside
/// [`jsonwebtoken::decode`] against the raw claims JSON before this type is
/// ever constructed (see [`jsonwebtoken::Validation`]), so a caller
/// receiving an [`OidcClaims`] already knows those checks passed.
#[derive(Debug, Deserialize)]
struct OidcClaims {
    /// The subject identifier — this provider's stable id for the caller.
    /// Becomes `auth_identity.external_subject`.
    sub: String,
    /// The caller's email, if the token carries one. Used for JIT
    /// provisioning only.
    #[serde(default)]
    email: Option<String>,
    /// The caller's display name, if the token carries one. Used for JIT
    /// provisioning only.
    #[serde(default)]
    name: Option<String>,
    /// Every other claim, keyed by name.
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl OidcClaims {
    /// Read `claim_name` as a list of group/role strings. A missing claim,
    /// a non-array claim, or a non-string array element is treated as "no
    /// groups" rather than an error — a token that simply doesn't carry
    /// group membership is not a malformed token, and role mapping degrades
    /// gracefully to "no mapped roles" rather than failing the whole
    /// authentication.
    fn groups(&self, claim_name: &str) -> Vec<String> {
        match self.extra.get(claim_name) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// [`Authenticator`] for [`Credential::Bearer`] — the concrete `OIDC`
/// implementor the module doc comment (and [`crate::authenticator`]'s doc
/// comment before it) sketched.
pub struct OidcAuthenticator {
    provider_label: String,
    config: OidcConfig,
    jwks: JwksClient,
    pool: PgPool,
}

impl OidcAuthenticator {
    /// Build an authenticator for `config`, backed by `pool`, fetching JWKS
    /// from `config.jwks_url` with the default cache TTL.
    #[must_use]
    pub fn new(config: OidcConfig, pool: PgPool) -> Self {
        let jwks = JwksClient::new(config.jwks_url.clone());
        Self::with_jwks_client(config, jwks, pool)
    }

    /// Build an authenticator with an explicit [`JwksClient`] — the seam
    /// tests use to install a short cache TTL and point at a `wiremock`
    /// server instead of a real provider.
    #[must_use]
    pub fn with_jwks_client(config: OidcConfig, jwks: JwksClient, pool: PgPool) -> Self {
        let provider_label = config.provider_label();
        Self {
            provider_label,
            config,
            jwks,
            pool,
        }
    }

    /// Verify `token`'s signature, algorithm, `iss`, `aud`, `exp`, and
    /// (when present) `nbf`, returning its claims on success.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidCredentials`] if the token is
    /// malformed, its `alg` is not in [`ALLOWED_ALGORITHMS`], its header
    /// carries no `kid`, no known key matches that `kid`, the signature
    /// doesn't verify, or `iss`/`aud`/`exp`/`nbf` fail validation. This
    /// crate deliberately does not distinguish these cases in the returned
    /// error — see [`crate::error`]'s non-enumeration doc comment; the
    /// same reasoning applies to a bearer token as to a password.
    async fn validate_token(&self, token: &str) -> Result<OidcClaims, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::InvalidCredentials)?;
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(AuthError::InvalidCredentials);
        }
        let kid = header.kid.as_deref().ok_or(AuthError::InvalidCredentials)?;
        let decoding_key = self.jwks.key_for(kid).await?;

        let mut validation = Validation::new(header.alg);
        // Restrict to exactly the algorithm this token's header claims, not
        // the whole allowlist — `jsonwebtoken` requires every algorithm in
        // this list to share `decoding_key`'s key family (see
        // `ALLOWED_ALGORITHMS`'s doc comment), and pinning to the header's
        // own (already-allowlisted) algorithm is what makes that check
        // meaningful rather than vacuous.
        validation.algorithms = vec![header.alg];
        validation.set_issuer(&[self.config.issuer.as_str()]);
        validation.set_audience(&[self.config.client_id.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = self.config.clock_skew_seconds;

        let data = decode::<OidcClaims>(token, &decoding_key, &validation)
            .map_err(|_| AuthError::InvalidCredentials)?;
        Ok(data.claims)
    }

    /// Resolve `claims` to a [`Principal`]: find (or, if
    /// [`OidcConfig::jit_provisioning`] allows it, create) the
    /// `auth_identity` row for this provider/subject, load the normalized
    /// principal exactly like every other authenticator, then layer on
    /// mapped permissions from `claims`' groups.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidCredentials`] if no `auth_identity` row
    /// matches `claims.sub` and JIT provisioning is off, or
    /// [`AuthError::Database`] on a storage failure.
    async fn resolve_principal(&self, claims: &OidcClaims) -> Result<Principal, AuthError> {
        let existing: Option<(Uuid,)> = sqlx::query_as(
            "SELECT app_user_id FROM auth_identity WHERE provider = $1 AND external_subject = $2",
        )
        .bind(&self.provider_label)
        .bind(&claims.sub)
        .fetch_optional(&self.pool)
        .await?;

        let app_user_id = if let Some((id,)) = existing {
            id
        } else {
            if !self.config.jit_provisioning {
                // Deliberately the same variant a bad password gets —
                // "this bearer token does not authenticate anyone
                // here" is exactly what it means, and distinguishing
                // "valid signature, unknown subject" from "invalid
                // signature" would let a caller enumerate which
                // subjects are (not yet) provisioned.
                return Err(AuthError::InvalidCredentials);
            }
            self.provision_user(claims).await?
        };

        let mut principal = repository::load_principal_for_user(
            &self.pool,
            app_user_id,
            self.provider_label.clone(),
            // An OIDC-provisioned/linked login is never a `local`
            // identity, so it has no `must_change_password` to inherit —
            // always `false`, same as `service_token`.
            false,
        )
        .await
        .map_err(|_| AuthError::InvalidCredentials)?;
        let mapped = self.mapped_permissions(claims).await?;
        principal.permissions = PermissionSet::merge([principal.permissions, mapped]);
        Ok(principal)
    }

    /// Create (or link to an existing, same-email) `app_user` for `claims`,
    /// plus the `auth_identity` row that lets future logins find it
    /// directly. Only ever called when [`OidcConfig::jit_provisioning`] is
    /// `true`.
    ///
    /// Linking by email rather than always inserting a fresh `app_user`
    /// matters for the same reason `0019_auth.sql` lets one `app_user` hold
    /// several `auth_identity` rows: a person who already has a local
    /// password account (or a different provider's identity) and then logs
    /// in via this provider for the first time should end up as one
    /// account with two linked identities, not two accounts with the same
    /// email.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Conflict`] on an email race (two concurrent
    /// first-time logins), or [`AuthError::Database`] on any other storage
    /// failure.
    async fn provision_user(&self, claims: &OidcClaims) -> Result<Uuid, AuthError> {
        let email = claims.email.clone().unwrap_or_else(|| {
            format!("{}@{}.oidc.invalid", claims.sub, self.config.provider_name)
        });
        let name = claims.name.clone().unwrap_or_else(|| email.clone());

        let mut tx = self.pool.begin().await?;

        let existing_user: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
                .bind(&email)
                .fetch_optional(&mut *tx)
                .await?;
        let app_user_id = if let Some((id,)) = existing_user {
            id
        } else {
            let (id,): (Uuid,) =
                sqlx::query_as("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
                    .bind(&name)
                    .bind(&email)
                    .fetch_one(&mut *tx)
                    .await?;
            id
        };

        sqlx::query(
            "INSERT INTO auth_identity (provider, external_subject, app_user_id, password_hash) \
             VALUES ($1, $2, $3, NULL) \
             ON CONFLICT (provider, external_subject) DO NOTHING",
        )
        .bind(&self.provider_label)
        .bind(&claims.sub)
        .bind(app_user_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(app_user_id)
    }

    /// Compute the [`PermissionSet`] `claims`' mapped groups grant, per
    /// [`OidcConfig::role_map`]/[`OidcConfig::groups_claim`].
    ///
    /// # Precedence: union, not IdP-authoritative
    ///
    /// The permissions this returns are ADDED to whatever the user's
    /// locally-assigned `app_user_role` rows already grant (see
    /// [`Self::resolve_principal`]'s `PermissionSet::merge`) — never
    /// replace them. Two reasons:
    ///
    /// 1. **No destructive write.** This function only ever reads `role`;
    ///    it never touches `app_user_role`. An "IdP-authoritative" design
    ///    would need to delete-then-reinsert the user's role assignments on
    ///    every login to make the replacement durable outside a single
    ///    request, which reintroduces exactly the kind of write this
    ///    module's non-negotiables (JIT default-off, no schema change)
    ///    argue against doing lightly — and does it on a path (every
    ///    authenticated request) that must stay fast and side-effect-free.
    /// 2. **A local Platform Admin who also uses SSO must not lose access**
    ///    because an operator's `OIDC_ROLE_MAP` doesn't happen to cover
    ///    their `IdP` group. Union means a locally-assigned role is a
    ///    permanent floor; a mapped role is an addition for the lifetime of
    ///    that login only, not a persisted grant — remove the user from the
    ///    `IdP` group and their next login simply doesn't carry that mapped
    ///    permission anymore. That is close enough to "`IdP` is authoritative
    ///    for what it explicitly grants" without the codebase taking on the
    ///    much larger job of being authoritative for revocation too via a
    ///    background sync.
    ///
    /// If a deployment genuinely wants "the `IdP`'s groups are the *only*
    /// source of truth, full stop" (common in a pure-SSO shop with no local
    /// accounts at all), that is a straightforward follow-up: mutate
    /// `app_user_role` from `claims`' groups instead of/in addition to
    /// merging here. That is deliberately not what this task builds, since
    /// this codebase's local-role model (used by every seeded fixture) is
    /// the primary path today and OIDC is additive to it.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Database`] on a storage failure. Never fails
    /// because a group in `claims` has no entry in
    /// [`OidcConfig::role_map`] — unmapped groups are silently ignored.
    async fn mapped_permissions(&self, claims: &OidcClaims) -> Result<PermissionSet, AuthError> {
        if self.config.role_map.is_empty() {
            return Ok(PermissionSet::default());
        }
        let groups = claims.groups(&self.config.groups_claim);
        let role_names: Vec<&str> = groups
            .iter()
            .filter_map(|group| self.config.role_map.get(group))
            .map(String::as_str)
            .collect();
        if role_names.is_empty() {
            return Ok(PermissionSet::default());
        }
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT permissions FROM role WHERE name = ANY($1)")
                .bind(&role_names)
                .fetch_all(&self.pool)
                .await?;
        Ok(PermissionSet::merge(
            rows.iter().map(|(raw,)| PermissionSet::parse(raw)),
        ))
    }
}

#[async_trait]
impl Authenticator for OidcAuthenticator {
    fn provider_id(&self) -> &str {
        &self.provider_label
    }

    async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
        let Credential::Bearer(token) = credential else {
            return Err(AuthError::UnsupportedCredential);
        };
        let claims = self.validate_token(token.expose()).await?;
        self.resolve_principal(&claims).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn provider_label_combines_the_oidc_prefix_and_name() {
        let config = OidcConfig::new(
            "https://issuer.example",
            "client-1",
            "okta",
            "https://issuer.example/jwks",
        );
        assert_eq!(config.provider_label(), "oidc:okta");
    }

    #[test]
    fn allowed_algorithms_never_include_a_symmetric_alg() {
        // Regression for the algorithm-confusion defense described on
        // `ALLOWED_ALGORITHMS`'s doc comment: this list must never grow an
        // HS256/HS384/HS512 entry.
        for alg in ALLOWED_ALGORITHMS {
            assert!(
                !matches!(alg, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512),
                "{alg:?} is a symmetric algorithm and must not be in ALLOWED_ALGORITHMS"
            );
        }
    }

    #[test]
    fn claims_groups_reads_a_string_array_claim() {
        let claims = OidcClaims {
            sub: "user-1".to_owned(),
            email: None,
            name: None,
            extra: HashMap::from([(
                "groups".to_owned(),
                Value::Array(vec![Value::String("lakehouse-admins".to_owned())]),
            )]),
        };
        assert_eq!(claims.groups("groups"), vec!["lakehouse-admins".to_owned()]);
    }

    #[test]
    fn claims_groups_degrades_to_empty_when_claim_is_missing() {
        let claims = OidcClaims {
            sub: "user-1".to_owned(),
            email: None,
            name: None,
            extra: HashMap::new(),
        };
        assert!(claims.groups("groups").is_empty());
    }

    #[test]
    fn claims_groups_degrades_to_empty_when_claim_is_not_an_array() {
        let claims = OidcClaims {
            sub: "user-1".to_owned(),
            email: None,
            name: None,
            extra: HashMap::from([(
                "groups".to_owned(),
                Value::String("not-an-array".to_owned()),
            )]),
        };
        assert!(claims.groups("groups").is_empty());
    }
}
