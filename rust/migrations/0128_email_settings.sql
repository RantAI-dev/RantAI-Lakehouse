-- Deployment-wide email settings, saved from the console (`ADM-7`, slice 1 of
-- `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
--
-- Why a table: email delivery used to be configured only through `SMTP_*`
-- environment variables, which an operator sets once at start-up. `ADM-7`
-- lets a holder of `settings:write` change the mail server, the account and
-- the `From` address from the console without touching the environment.
-- Precedence (plan D1) is: the environment wins, then this saved row, then the
-- code default. The API merges the three in exactly one place
-- (`lakehouse-api` `runtime_settings.rs`), never here.
--
-- One row at most: `singleton` is the primary key and can only be true. No row
-- means "nothing saved in the console", which the API reads as the default or
-- the environment's value. Every setting column is nullable: NULL means "not
-- set in the console", so an absent field falls through to the environment or
-- the default. `smtp_pass` is BYTEA holding the password SEALED with the
-- installation key (`CONNECTOR_SECRETS_KEY`, `lakehouse-api` `secret_crypto.rs`)
-- under the associated-data name `settings/email/password`; the plaintext never
-- reaches a row, a log or a response. A row whose seal does not open (the key
-- changed) is read as "not set" and logged once.
--
-- `updated_by` is the principal id of whoever saved last, for the audit trail
-- of a setting every alert and scheduled report reads.
CREATE TABLE IF NOT EXISTS email_settings (
    singleton          BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    smtp_host          TEXT,
    smtp_port          INTEGER     CHECK (smtp_port IS NULL OR smtp_port BETWEEN 1 AND 65535),
    smtp_security      TEXT        CHECK (smtp_security IS NULL OR smtp_security IN ('tls', 'starttls')),
    smtp_user          TEXT,
    smtp_pass          BYTEA,
    smtp_from          TEXT,
    console_public_url TEXT,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by         TEXT        NOT NULL DEFAULT ''
);
