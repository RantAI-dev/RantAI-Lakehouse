-- Where each threshold alert stands, so it fires once when the value crosses
-- the line and once more when it is back (`BI-19` part C, decision D11 of
-- `docs/superpowers/plans/2026-10-18-bi-19-sharing.md`; feature page
-- `docs/core/features/sharing-and-delivery.md`, decision C2).
--
-- Why a table of its own and not columns on `console.alert_rule`: the rule row
-- lives in ClickHouse and is replaced whole on every save (a
-- `ReplacingMergeTree`), so a state written there would be lost or rolled back
-- by a rule edit made between two checks, and every check would be a row
-- rewrite. The state is changed by the evaluator alone, one row per rule, in
-- the store that can do an atomic upsert. `rule_id` names a ClickHouse row, so
-- it has no foreign key; a deleted rule leaves a row that nothing reads, and
-- the API removes it when the rule is deleted.
--
-- `state`: `ok` (not over the line, or no check yet: a rule without a row reads
-- as `ok`, so the first check over the line sends once), `firing` (over the
-- line and the crossing was sent), `stopped` (the rule's owner can no longer
-- see its chart, or is disabled; nothing is sent and `reason` says why).
-- `since` is when the state last changed, not when it was last checked.
-- `last_value` is the last number read (NULL when the last check read none, or
-- the rule is a "returns any rows" rule). `last_checked_at` moves on every
-- check, including one that failed: a check that cannot run records itself and
-- the fixed `reason` and changes nothing else (C2).
--
-- `reason` is one of the product's own sentences, never text from the engine
-- (principle 4); the evaluator maps a failure to a fixed sentence before it
-- reaches this table.
CREATE TABLE IF NOT EXISTS alert_rule_state (
    rule_id         TEXT PRIMARY KEY,
    state           TEXT NOT NULL CHECK (state IN ('ok', 'firing', 'stopped')),
    since           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_value      DOUBLE PRECISION,
    last_checked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reason          TEXT
);
