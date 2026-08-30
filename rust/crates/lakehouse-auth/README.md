# lakehouse-auth

The authentication core for `lakehouse-api`: everything a caller can present
as proof of identity, normalized into one shape every handler and policy
check consumes.

This document covers the provider model, how to connect a real identity
provider (Okta, Entra, Google, Keycloak, or any generic `OIDC` issuer), how
JIT provisioning and role mapping behave, and how a *non*-`OIDC` provider
(SAML, LDAP) plugs into the same seam.

## The provider model

Three types make up the seam every provider — built-in or future — is
judged against:

- **[`Principal`](src/principal.rs)** — the normalized identity of an
  authenticated caller (`id`, `display_name`, `permissions`, `tenant_ids`,
  `provider`). No secret material, ever. A handler reads
  `principal.has("catalog:write")` and never learns, or needs to learn,
  whether the caller typed a password, presented a session cookie, or
  arrived via an `OIDC` id token.
- **[`Credential`](src/credential.rs)** — everything a caller can present:
  `Password`, `SessionToken`, `ServiceToken`, and `Bearer` (a signature-
  validated token — what an `OIDC` id token, or any future JWT-shaped
  credential, arrives as).
- **[`Authenticator`](src/authenticator.rs)** — the trait every provider
  implements: `provider_id(&self) -> &str` and `async fn
  authenticate(&self, credential: &Credential) -> Result<Principal,
  AuthError>`. Given the wrong `Credential` variant, an implementor returns
  `AuthError::UnsupportedCredential` so a caller trying several
  authenticators in turn knows to move to the next one, not treat it as a
  failed login.

Four authenticators exist today: [`password::LocalPasswordAuthenticator`],
[`session::SessionAuthenticator`], [`service_token::ServiceTokenAuthenticator`],
and [`oidc::OidcAuthenticator`]. Every one of them writes to and reads from
the same `auth_identity` table (`../../migrations/0019_auth.sql`): a local
password is one row with `provider = 'local'`; an `OIDC` identity is one row
with `provider = 'oidc:<provider_name>'`. Adding a provider never means a
new column, a new table, or a schema migration — it means new rows in a
table that was already designed to hold them.

## Connecting a real `OIDC` provider

`lakehouse_auth::oidc::OidcAuthenticator` is a **resource server**, not a
full `OIDC` client: it verifies an already-issued bearer token (an
`Authorization: Bearer <id_token>` header) against the provider's public
JWKS. It does not perform the authorization-code exchange, does not serve a
`/callback` route, and does not handle refresh tokens — that redirect/login
flow is a frontend concern, wired up separately. What this crate needs from
that flow is only the resulting `id_token`, presented as a bearer token on
every subsequent API request.

### Environment variables

| Variable | Required | Default | Meaning |
| --- | --- | --- | --- |
| `OIDC_ISSUER` | to enable OIDC | unset | The provider's issuer URL. Compared byte-for-byte against the token's `iss` claim. |
| `OIDC_CLIENT_ID` | to enable OIDC | unset | This application's client id, as registered with the provider. Must appear in the token's `aud` claim. |
| `OIDC_CLIENT_SECRET` | no | unset | Reserved for a future authorization-code exchange. Not read by `OidcAuthenticator` — verifying a JWKS-signed token needs no shared secret. Never logged; redacted in `Config`'s `Debug`. |
| `OIDC_PROVIDER_NAME` | no | `"default"` | Short label combined with `oidc:` to form `Principal::provider` / `auth_identity.provider` (e.g. `oidc:okta`). |
| `OIDC_JWKS_URL` | no | `{OIDC_ISSUER}/.well-known/jwks.json` | Explicit override for the JWKS endpoint, for a provider whose JWKS isn't at the conventional path. |
| `OIDC_JIT_PROVISIONING` | no | `false` | `"true"` to auto-create an `app_user` for a token that validates but has no linked `auth_identity` row yet. See "JIT provisioning" below. |
| `OIDC_ROLE_MAP` | no | empty | `"group1=Role One,group2=Role Two"` — maps an IdP group/role claim value to a local `role.name`. See "Role mapping" below. |
| `OIDC_GROUPS_CLAIM` | no | `"groups"` | Which claim in the token carries the caller's groups/roles. |
| `OIDC_CLOCK_SKEW_SECONDS` | no | `60` | Leeway applied to `exp`/`nbf` validation. |

**`OIDC` is enabled only when both `OIDC_ISSUER` and `OIDC_CLIENT_ID` are
set.** With either unset, `AuthState::oidc` stays `None`, the bearer
dispatch in `lakehouse-api`'s `crate::auth` never tries the `OIDC` path, and
the service behaves exactly as it does with no `OIDC` support at all — local
password auth, sessions, and service tokens keep working, and nothing fails
at boot. This is exercised directly: the parity suite runs the full 72-case
corpus with `OIDC` unconfigured and expects zero change in behavior.

### What to set on the IdP side

Register this service as an application/API on the provider and note down:

- **Issuer URL** → `OIDC_ISSUER`.
- **Client ID** → `OIDC_CLIENT_ID`.
- **Redirect/callback URI**: whatever the separate login-UI flow uses for
  the authorization-code redirect (this crate is not involved in that leg —
  it only ever sees the resulting `id_token`).
- **Token audience**: make sure the provider issues `id_token`s whose `aud`
  claim is exactly `OIDC_CLIENT_ID` — most providers do this automatically
  once the client id above is registered.
- **Groups/roles claim**: if role mapping is wanted, configure the provider
  to include a groups/roles claim in the `id_token` (not just the
  `access_token` — this crate only ever sees the token it's handed) under
  the name set in `OIDC_GROUPS_CLAIM`.

Provider-specific notes:

- **Okta.** Create an OIDC application (Web or SPA, depending on the
  frontend's flow). Issuer is `https://<your-org>.okta.com` (or
  `https://<your-org>.okta.com/oauth2/<auth-server-id>` for a custom
  authorization server — the JWKS then lives under that same path). To get
  a groups claim into the `id_token`, add a Groups Claim Filter under
  **Sign On → OpenID Connect ID Token** and set `OIDC_GROUPS_CLAIM=groups`.
- **Microsoft Entra ID (Azure AD).** Register an application; issuer is
  `https://login.microsoftonline.com/<tenant-id>/v2.0`. To get a `groups`
  claim, add the Groups optional claim (Token configuration → Add groups
  claim) to the ID token — Entra's default groups claim emits group *object
  IDs*, so `OIDC_ROLE_MAP` keys need to be those GUIDs, or configure
  group-name emission if the app registration supports it.
- **Google.** Issuer is `https://accounts.google.com`. Google's `id_token`
  carries no groups/roles claim by default (Google Groups membership isn't
  exposed this way) — role mapping is realistically only usable with
  Google Workspace + a custom claim added via a directory sync/Cloud
  Function, or simply left unconfigured (every OIDC caller then gets
  whatever their local `app_user_role` rows grant, with JIT-provisioned
  accounts starting with no roles at all).
- **Keycloak.** Issuer is
  `https://<host>/realms/<realm>`. The default groups claim is
  `groups` (a client scope may need enabling depending on realm config) —
  matches `OIDC_GROUPS_CLAIM`'s default.

### Role mapping

`OIDC_ROLE_MAP` maps an IdP group/role claim *value* to a local `role.name`
(one of the seeded roles like `"Platform Admin"`, `"Analyst"`, or any
role created via `POST /api/identity/roles`). A group present in the
token's claim but absent from the map is silently ignored — an operator
extending `OIDC_ROLE_MAP` incrementally, or an IdP group with no local
counterpart, is not a fatal misconfiguration.

**Precedence when a user has both locally-assigned roles and mapped
ones: union, not IdP-authoritative.** `OidcAuthenticator::mapped_permissions`
computes the permission set the token's mapped groups grant and merges it
with whatever the user's `app_user_role` rows already grant — it never
deletes or replaces local role assignments. Two reasons:

1. **No destructive write.** Role mapping only ever reads the `role` table;
   it never touches `app_user_role`. Making the IdP fully authoritative
   would require synchronizing `app_user_role` on every login (delete then
   reinsert), a much heavier and riskier operation to run on a hot path
   that must stay fast.
2. **A local Platform Admin who also uses SSO must not lose access** because
   an operator's `OIDC_ROLE_MAP` doesn't happen to cover their IdP group.
   Union means a locally-assigned role is a permanent floor; a mapped role
   is an addition scoped to that login's `Principal` only — it is
   recomputed from the token's claims on every authentication, never
   persisted. Removing a user from the IdP group takes effect on their
   very next login, without this codebase needing to run a background
   revocation sync.

If a deployment genuinely wants "the IdP's groups are the *only* source of
truth" (a pure-SSO shop with no local accounts), the natural extension is to
mutate `app_user_role` from the token's groups instead of (or in addition
to) merging in `OidcAuthenticator::mapped_permissions` — deliberately not
what this crate builds today, since the local-role model is the primary
path every seeded fixture uses and `OIDC` is additive to it.

### JIT (just-in-time) provisioning

Default **off**. With `OIDC_JIT_PROVISIONING` unset (or anything other than
`"true"`), a token that validates but whose `sub` has no matching
`auth_identity` row is rejected with the same `AuthError::InvalidCredentials`
a bad password gets — this crate never distinguishes "valid signature,
unknown subject" from "invalid credential" in what it returns, for the same
enumeration-resistance reason `lakehouse_auth::password` gives for local
login.

With it `true`, an unrecognized subject provisions a new `app_user` (name/
email taken from the token's `name`/`email` claims, falling back to a
synthetic `<sub>@<provider_name>.oidc.invalid` address if `email` is
absent) and links an `auth_identity` row to it. If an `app_user` with the
same email already exists (e.g. someone who first signed up with a local
password), the new `OIDC` identity links to that existing account instead
of creating a duplicate — the same "one person, several linked identities"
model `0019_auth.sql` was built around.

The default is off specifically because an over-broad `OIDC_ISSUER`/
`OIDC_CLIENT_ID` configuration (or a provider allowing self-service sign-up
into a tenant it shouldn't) would otherwise let any token that merely
*validates* mint a real account. Turning it on is a deliberate operator
decision, not something a misconfiguration falls into by default.

## Signature validation: the non-negotiables

`OidcAuthenticator` never hand-rolls JWT verification — [`jsonwebtoken`]
does the cryptography. What this crate is responsible for:

- **JWKS caching with rotation** ([`oidc::JwksClient`]). A cache hit serves
  a known `kid` without a network call; a `kid` the cache doesn't recognize
  (cold cache, expired TTL, or a genuinely new key) triggers exactly one
  refetch. This means a provider's key rotation is picked up on the very
  next token signed with the new key, without waiting out the cache TTL —
  see `tests/oidc.rs`'s
  `a_rotated_key_is_picked_up_on_first_use_without_waiting_out_the_ttl`,
  and `a_second_validation_within_ttl_does_not_refetch_the_jwks` for the
  caching side, both asserting the exact `wiremock` request count.
- **`iss`/`aud`/`exp`/`nbf` with configurable clock skew** — via
  [`jsonwebtoken::Validation`], `leeway` set from `OIDC_CLOCK_SKEW_SECONDS`.
- **An algorithm allowlist that rejects `none` and blocks algorithm
  confusion** — see [`oidc::ALLOWED_ALGORITHMS`]'s doc comment for the
  full reasoning. In short: the allowlist contains only asymmetric
  algorithms (`RS256`/`RS384`/`RS512`/`ES256`/`ES384`), so an attacker
  cannot re-sign a token with `alg: HS256` using a provider's public key
  bytes as an HMAC "secret" (the classic algorithm-confusion attack against
  RS256-issuing IdPs); `alg: none` is rejected at header-parsing time,
  before the allowlist is even consulted, because `jsonwebtoken`'s
  `Algorithm` type has no `none` variant to parse into.

Every one of these is tested against a locally generated RSA keypair and a
`wiremock` JWKS server in `tests/oidc.rs` — never a real identity provider.
See that file's module doc comment for exactly which failure modes are
tested (wrong `iss`, wrong `aud`, expired, `nbf` in the future, `alg: none`,
a signature from the wrong key, an unknown `kid`, a missing `kid`) and which
tests need Postgres (identity resolution, JIT, role mapping — these run
against the `lakehouse-test-support` testcontainers-managed Postgres, no
manual setup required; see that file's doc comment).

## Disambiguating a service token from an `OIDC` token

Both a service credential and an `OIDC` id token arrive as `Authorization:
Bearer <token>` — the same header. `lakehouse-api`'s `crate::auth` module
tells them apart by SHAPE: an opaque service token is always exactly 64
lowercase hex characters (no `.`); a JWT is always exactly three
`.`-separated base64url segments. These shapes are structurally disjoint,
so this is a correctness-preserving dispatch, not a heuristic that could
misroute a valid credential of either kind — see `crate::auth`'s module doc
comment in `lakehouse-api` for the full reasoning.

## Adding a *non*-`OIDC` provider (SAML, LDAP)

The same `Authenticator` trait covers this without any change to
`Principal`, `Credential`, `AuthError`, or `auth_identity`'s schema:

- **SAML.** A SAML response is XML, not a JWT, so it doesn't fit
  `Credential::Bearer` as-is. Two reasonable shapes: (a) add a new
  `Credential::Saml(Secret)` variant carrying the raw `SAMLResponse` (same
  pattern `Bearer` already established for JWTs — a new variant is a small,
  additive change, not a redesign), or (b) if the login flow already
  decodes the SAML assertion into a JWT-like bearer token before it reaches
  this crate (common when a gateway/BFF does the SAML dance), it fits
  `Credential::Bearer` directly. Either way: validate the assertion's
  signature against the IdP's certificate, extract the `NameID` as
  `external_subject`, and write/read `auth_identity` rows at
  `provider = 'saml:<idp_name>'` — the exact same find-or-link operation
  `oidc::OidcAuthenticator::resolve_principal` already does.
- **LDAP/Active Directory.** `Credential::Password` already fits an LDAP
  bind directly — `identifier` is the LDAP username/UPN, `password` is what
  gets bound against the directory. A new `LdapAuthenticator` implements
  `authenticate` by performing an LDAP simple bind instead of an `Argon2`
  hash comparison, then does the same `auth_identity` find-or-link at
  `provider = 'ldap:<domain>'`, loading the normalized `Principal` via
  `repository::load_principal_for_user` exactly like every other
  authenticator.

In every case, the concrete file-by-file answer to "what does adding this
provider require" is the same one `Authenticator`'s doc comment gives for
`OIDC`, and the same one this task proved out: a new file in this crate (or
a new crate, if the provider pulls in a dependency this crate shouldn't
carry for callers that never use it), new config, new `auth_identity` rows
at a new `provider` value, and one line registering the authenticator where
`lakehouse-api` builds `AuthState`. Nothing about `Principal`, `Credential`,
`AuthError`, the schema, or any existing authenticator changes.

[`jsonwebtoken`]: https://docs.rs/jsonwebtoken
[`password::LocalPasswordAuthenticator`]: src/password.rs
[`session::SessionAuthenticator`]: src/session.rs
[`service_token::ServiceTokenAuthenticator`]: src/service_token.rs
[`oidc::OidcAuthenticator`]: src/oidc.rs
[`oidc::JwksClient`]: src/oidc.rs
[`oidc::ALLOWED_ALGORITHMS`]: src/oidc.rs
