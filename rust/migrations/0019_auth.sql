-- Phase 3, Task 3.1: the authentication core's schema.
--
-- WHAT THIS IS: storage for `lakehouse-auth` — the crate that normalizes
-- every way a caller can prove who they are into one `Principal` shape.
-- `0001_init.sql` already has `app_user`/`role`/`service_identity` as an
-- identity *directory* (who exists, what they're called); nothing in that
-- migration stores a credential, because Phase 2 had no authentication at
-- all. This migration adds exactly the credential/session storage that was
-- missing, without touching `0001_init`'s tables.
--
-- THE KEY ABSTRACTION: `auth_identity`. A local password is not a special
-- case bolted onto `app_user` — it is one row in `auth_identity` with
-- `provider = 'local'`. Adding Okta, Google, Entra, Keycloak, or any other
-- OIDC/SAML provider later means inserting rows with `provider =
-- 'oidc:okta'` (etc.) into this SAME table — no new column, no new table,
-- no schema migration. A single `app_user` can hold several linked
-- identities (e.g. a `local` password AND an `oidc:okta` link, useful
-- during a provider migration), which is exactly what letting `provider`
-- vary per row, rather than putting a `password_hash` column directly on
-- `app_user`, buys.
--
-- SECRETS ARE NEVER STORED RAW. `auth_identity.password_hash` is an Argon2id
-- PHC string, never a plaintext password. `session.token_hash` and
-- `service_credential.token_hash` are SHA-256 hex digests of a
-- high-entropy, CSPRNG-generated opaque token, never the token itself.
-- Losing this table in a backup leak reveals no credential usable to log
-- in.

-- ── auth_identity ───────────────────────────────────────────────────────
-- One row per (provider, external_subject) an `app_user` can authenticate
-- as. `provider = 'local'` uses the user's own id (as text) for
-- `external_subject`, since a local login has no external subject of its
-- own; a future `oidc:*` provider uses whatever subject claim the IdP
-- issues. `password_hash` is populated ONLY for `provider = 'local'` — the
-- check constraint below makes "an OIDC row somehow carries a password
-- hash" a schema-level impossibility rather than an application-code
-- discipline.
CREATE TABLE auth_identity (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider               TEXT NOT NULL,
    external_subject       TEXT NOT NULL,
    app_user_id            UUID NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
    password_hash          TEXT,
    -- Set on every `provider = 'local'` identity created by an
    -- administrative bootstrap (e.g. the very first admin account); cleared
    -- the first time the holder successfully changes their password. Lets
    -- a caller refuse to treat a bootstrapped credential as good for
    -- anything beyond "go change your password" — see
    -- `lakehouse_auth::password`.
    must_change_password  BOOLEAN NOT NULL DEFAULT false,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT auth_identity_local_password_shape CHECK (
        (provider = 'local' AND password_hash IS NOT NULL)
        OR (provider <> 'local' AND password_hash IS NULL)
    ),
    -- The natural key of this table: one row per external identity per
    -- provider, and no two `app_user`s can claim the same external
    -- identity.
    CONSTRAINT auth_identity_provider_subject_unique UNIQUE (provider, external_subject)
);

-- A user's identities are always looked up by owner (e.g. "does this user
-- already have a local identity"), not just by (provider, subject).
CREATE INDEX auth_identity_app_user_id_idx ON auth_identity (app_user_id);

-- ── session ─────────────────────────────────────────────────────────────
-- One row per issued browser session. `token_hash` is a SHA-256 hex digest
-- of a 32-byte CSPRNG token (see `lakehouse_auth::session::create_session`)
-- — opaque and revocable, unlike a self-contained JWT, which is exactly why
-- the browser path uses this instead of a bearer token. `revoked_at` being
-- non-null is an explicit, immediate revocation (sign-out, password
-- change); `expires_at` being in the past is a passive one.
CREATE TABLE session (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_user_id   UUID NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
    token_hash    TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    revoked_at    TIMESTAMPTZ,
    -- Audit-only; never used in an authorization decision.
    created_ip    TEXT,
    user_agent    TEXT,
    CONSTRAINT session_token_hash_unique UNIQUE (token_hash)
);

CREATE INDEX session_app_user_id_idx ON session (app_user_id);

-- ── service_credential ──────────────────────────────────────────────────
-- What `service_identity` (`0001_init.sql`) was missing to actually
-- authenticate a service: `service_identity` stores that a credential
-- exists (name/scopes/expiry) but, by design, never the credential itself.
-- This table is where the (hashed) token side lives, following the exact
-- same never-store-the-raw-secret rule as `session.token_hash`. Deleting
-- the owning `service_identity` cascades away its credential.
CREATE TABLE service_credential (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    service_identity_id UUID NOT NULL REFERENCES service_identity(id) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at          TIMESTAMPTZ,
    CONSTRAINT service_credential_token_hash_unique UNIQUE (token_hash)
);

CREATE INDEX service_credential_service_identity_id_idx
    ON service_credential (service_identity_id);
