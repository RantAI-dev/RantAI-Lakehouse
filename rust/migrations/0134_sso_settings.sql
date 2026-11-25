-- Deployment-wide single sign-on (OIDC) settings and test runs, saved from the
-- console (`ADM-12`, plan `docs/superpowers/plans/2026-11-09-adm-12-sso-in-settings.md`).
--
-- Why a table: single sign-on used to be configured only through `OIDC_*`
-- environment variables, read once at start-up. `ADM-12` lets a Platform
-- Admin configure the OIDC provider, client credentials, groups claim,
-- role map, and JIT account creation from the console without restarting the
-- service.
-- Precedence (plan D1/D3): if `OIDC_ISSUER` or `OIDC_CLIENT_ID` is set in the
-- environment, the connection is server-owned as a unit; otherwise the saved
-- row is used, falling through to code defaults.
--
-- One row at most: `singleton` is the primary key and can only be true. No row
-- means "nothing saved in the console". Every setting column is nullable:
-- NULL means "not set in the console", falling through to the environment or
-- the default. `oidc_client_secret` is BYTEA holding the client secret SEALED
-- with the installation key (`CONNECTOR_SECRETS_KEY`, `lakehouse-api`
-- `secret_crypto.rs`) under the associated-data name
-- `settings/sso/client_secret`; plaintext never reaches a row, a log, or a
-- response.
--
-- `sso_test_run` stores temporary sign-in test runs before saving.
-- `draft_secret` is BYTEA sealed with the installation key. Test runs expire
-- after 10 minutes (`expires_at`), and expired runs are deleted when a new
-- test is created.
--
-- `updated_by` is the principal id of whoever saved last.
CREATE TABLE IF NOT EXISTS sso_settings (
    singleton             BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    oidc_issuer           TEXT,
    oidc_client_id        TEXT,
    oidc_client_secret    BYTEA,
    oidc_provider_name    TEXT,
    oidc_groups_claim     TEXT,
    oidc_role_map         JSONB,
    oidc_jit_provisioning BOOLEAN,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by            TEXT        NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS sso_test_run (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    created_by   UUID        NOT NULL,
    draft        JSONB       NOT NULL,
    draft_secret BYTEA,
    result       JSONB,
    expires_at   TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS sso_test_run_expires_at_idx ON sso_test_run (expires_at);
