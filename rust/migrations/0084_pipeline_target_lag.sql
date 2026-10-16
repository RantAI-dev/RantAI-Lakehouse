-- BLD-2: a target lag per authored pipeline, as Snowflake `TARGET_LAG` /
-- `DOWNSTREAM` are to a dynamic table. A pipeline with a target lag runs
-- when one of its INPUTS changed and the lag would otherwise be missed,
-- instead of on a cron (plan D1/D3: the stored `schedule`/`depends_on`
-- stay untouched, so removing the lag returns the pipeline to them).
--
-- # Migration numbering
--
-- 0084, following 0080 on `main`. `0078`-`0083` are taken by PR #106's
-- open branch; 0084 is the next free number per plan D1 and review R4.
--
-- # Two columns, never both
--
-- `target_lag_seconds` and `target_lag_downstream` are the two halves of
-- one wire value (`{"kind":"lag","minutes":N}` or
-- `{"kind":"downstream"}`, plan D2). A table CHECK rather than a Rust
-- check: "downstream AND a number of minutes" is a contradiction the
-- database should refuse no matter which caller wrote it, including a
-- future one that forgets the route-level validation.
--
-- # The range is Snowflake's, and the store-wide floor
--
-- 60..604800 seconds is 1 minute to 7 days — the spec's signed range.
-- `% 60 = 0` because the wire format is whole MINUTES (plan D2): storing a
-- 90-second target would be a value no client can ask for and no page can
-- display. The route layer refuses out-of-range bodies with 400 so the
-- client never sees a constraint violation; this CHECK is defense in
-- depth. If the measured floor (plan D11 / `docs/plans/BLD-2-TARGET-LAG-
-- RESULT.md`) turns out to be above 60 seconds, THIS CHECK and the
-- route's own bound move together in the same PR — the minimum is never
-- claimed without a measurement behind it.
ALTER TABLE pipeline_definition
    ADD COLUMN target_lag_seconds INTEGER
        CHECK (target_lag_seconds BETWEEN 60 AND 604800 AND target_lag_seconds % 60 = 0),
    ADD COLUMN target_lag_downstream BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE pipeline_definition
    ADD CONSTRAINT pipeline_definition_target_lag_kind_check
        CHECK (NOT (target_lag_downstream AND target_lag_seconds IS NOT NULL));

COMMENT ON COLUMN pipeline_definition.target_lag_seconds IS
    'Target lag in WHOLE MINUTES × 60: 60..604800, NULL when the pipeline '
    'has no target lag or is "downstream" (see target_lag_downstream).';
COMMENT ON COLUMN pipeline_definition.target_lag_downstream IS
    'true = Snowflake DOWNSTREAM: this pipeline refreshes only when a '
    'pipeline with a target lag that reads it needs it. Mutually exclusive '
    'with target_lag_seconds: holding both is NOT allowed.';

-- The per-pipeline lag state, one row per pipeline the lag-check route has
-- ever looked at. It is NOT written by any user-facing route: the state is
-- a measurement, and a measurement written by the page that displays it is
-- a measurement the page can invent.
--
-- Why a table and not columns on `pipeline_definition`: `consumed_tokens`
-- is a JSONB document whose shape is the token map (plan D4), and the
-- whole row — consumed tokens, when they were consumed, what is pending,
-- the outstanding run request and the streak — is written together by one
-- transaction in the lag check. Splitting it across the definition table
-- would put a half-updated measurement on the row every other read (list,
-- detail, versions) already selects.
--
-- `ON DELETE CASCADE`: the state is meaningless without the pipeline it
-- describes, and a deleted pipeline must not leave a row that a later
-- re-created id could inherit.
--
-- `tenant_id` is the same column, and the same fail-closed reading rule,
-- the other pipeline tables carry (migration `0042`): an unassigned row is
-- invisible to every tenant-scoped read. Copied from the pipeline row on
-- insert rather than joined at read time so a scope check is one index
-- lookup and cannot be forgotten by a new reader.
CREATE TABLE pipeline_lag_state (
    pipeline_id TEXT PRIMARY KEY REFERENCES pipeline_definition (id) ON DELETE CASCADE,
    tenant_id UUID REFERENCES tenant (id) ON DELETE SET NULL,
    -- The input tokens (plan D4) the last SUCCESSFUL lag run read. NULL
    -- means "never consumed anything": a pipeline that has never run
    -- under its lag, which plan D5 treats as "run, and behind until it
    -- succeeds" rather than as an empty comparison that would report
    -- "met".
    consumed_tokens JSONB,
    consumed_at TIMESTAMPTZ,
    -- When the first check saw the current tokens differ from
    -- `consumed_tokens`; NULL after a check found them equal again. The
    -- lag is `now() - pending_since`, so it can read up to one check
    -- interval (30 s) low — the honest consequence of measuring from the
    -- check that noticed, not from the instant the row changed.
    pending_since TIMESTAMPTZ,
    -- The run this pipeline was last asked to make, and the tokens that
    -- run was asked to consume. A non-NULL pair means "a run is
    -- outstanding"; the next check asks Dagster for it by `run_key`
    -- before deciding anything new, so two checks can never request two
    -- runs for the same change.
    -- BLD-2 review R1 decision R4 (R1/R2 columns)
    requested_run_key TEXT,
    requested_tokens JSONB,
    requested_at TIMESTAMPTZ,
    request_attempt INTEGER NOT NULL DEFAULT 0,
    last_check_at TIMESTAMPTZ,
    -- 'met' | 'behind' | 'not_measured' — the same three words the API
    -- reports. 'not_measured' is what an unreadable input (plan D4) or a
    -- check that never ran leaves behind, and is deliberately NOT 'met':
    -- an unmeasured lag is not a met lag (AGENTS.md principle 2).
    last_status TEXT
        CHECK (last_status IS NULL OR last_status IN ('met', 'behind', 'not_measured')),
    -- Consecutive checks that ended 'behind'. The alert fires when this
    -- EQUALS 3 (plan D9), the same "once per streak" shape as
    -- `connectors.rs`'s REPEATED_FAILURE_STREAK, so no second table is
    -- needed for dedupe.
    behind_streak INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT pipeline_lag_state_request_pair_check
        CHECK ((requested_run_key IS NULL) = (requested_tokens IS NULL))
);

CREATE INDEX pipeline_lag_state_tenant_id_idx ON pipeline_lag_state (tenant_id);