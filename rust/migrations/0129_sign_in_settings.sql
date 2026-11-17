-- Deployment-wide sign-in limits, saved from the console (`ADM-8`, slice 2 of
-- `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
--
-- Why a table: the failed-login limit, the window it is counted in, the
-- lockout, how long sign-in records are kept and the longest life of a signed
-- embed link used to be environment variables (`LOGIN_MAX_FAILURES`,
-- `LOGIN_FAILURE_WINDOW_SECS`, `LOGIN_LOCKOUT_SECS`, `AUTH_RETENTION_DAYS`,
-- `EMBED_TOKEN_MAX_LIFETIME_SECONDS`), which an operator sets once at
-- start-up. `ADM-8` lets a holder of `settings:write` change them from the
-- console, and the next sign-in uses them with no restart. Precedence (plan
-- D1) is: the environment wins, then this saved row, then the code default.
-- The API merges the three in exactly one place (`lakehouse-api`
-- `runtime_settings.rs`), never here.
--
-- One row at most: `singleton` is the primary key and can only be true. No row
-- means "nothing saved in the console", which the API reads as the default or
-- the environment's value. Every setting column is nullable: NULL means "not
-- set in the console", so an absent field falls through to the environment or
-- the default.
--
-- The `CHECK`s are the ranges plan D12 names, so a value outside them can
-- never reach a row even if a caller skips the API's own validation: 3 to 20
-- failed tries, a window and a lockout of 60 to 86400 seconds, 7 to 3650 days
-- of sign-in records, and an embed link that lives 300 to 86400 seconds. The
-- upper bound of the embed lifetime is the code's own default
-- (`lakehouse_embed::DEFAULT_MAX_LIFETIME_SECONDS`, 24 hours); the
-- environment may set a longer one, but the console may not.
--
-- `updated_by` is the principal id of whoever saved last, for the audit trail
-- of a setting every sign-in reads.
CREATE TABLE IF NOT EXISTS sign_in_settings (
    singleton               BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    max_failures            INTEGER     CHECK (max_failures IS NULL OR max_failures BETWEEN 3 AND 20),
    failure_window_secs     INTEGER     CHECK (failure_window_secs IS NULL OR failure_window_secs BETWEEN 60 AND 86400),
    lockout_secs            INTEGER     CHECK (lockout_secs IS NULL OR lockout_secs BETWEEN 60 AND 86400),
    retention_days          INTEGER     CHECK (retention_days IS NULL OR retention_days BETWEEN 7 AND 3650),
    embed_max_lifetime_secs INTEGER     CHECK (embed_max_lifetime_secs IS NULL OR embed_max_lifetime_secs BETWEEN 300 AND 86400),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by              TEXT        NOT NULL DEFAULT ''
);
