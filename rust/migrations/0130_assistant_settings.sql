-- Deployment-wide assistant settings, saved from the console (`ADM-9`, slice 2
-- of `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
--
-- Why a table: the assistant's provider address, model and key used to be
-- environment variables (`LLM_URL`, `LLM_MODEL`, `LLM_KEY`), which an
-- operator sets once at start-up. `ADM-9` lets a holder of `settings:write`
-- give the assistant its key from the console, and the next message uses it
-- with no restart. Precedence (plan D1) is: the environment wins, then this
-- saved row, then the code default. The API merges the three in exactly one
-- place (`lakehouse-api` `runtime_settings.rs`), never here.
--
-- One row at most: `singleton` is the primary key and can only be true. No row
-- means "nothing saved in the console", which the API reads as the default or
-- the environment's value. Every setting column is nullable: NULL means "not
-- set in the console", so an absent field falls through to the environment or
-- the default.
--
-- The key is BYTEA holding the key SEALED with the installation key
-- (`CONNECTOR_SECRETS_KEY`, `lakehouse-api` `secret_crypto.rs`) under the
-- associated-data name `settings/assistant/key`; the plaintext never reaches
-- a row, a log or a response. A row whose seal does not open (the key
-- changed) is read as "not set" and logged once.
--
-- The address, the model and the key are ONE unit: they are how the
-- assistant reaches one provider, and the API treats them as the server's
-- (all read-only, and the console may change none of them) as soon as the
-- environment holds a key, so a key kept in files is never sent to a provider
-- address chosen in the console. The table itself stores the three fields
-- separately, because "the console saved an address but no key yet" is a
-- state an operator may pass through.
--
-- `updated_by` is the principal id of whoever saved last, for the audit trail
-- of a setting every assistant message reads.
CREATE TABLE IF NOT EXISTS assistant_settings (
    singleton    BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    llm_url      TEXT,
    llm_model    TEXT,
    llm_key      BYTEA,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by   TEXT        NOT NULL DEFAULT ''
);
