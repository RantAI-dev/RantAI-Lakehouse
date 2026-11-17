-- SRC-39 (`TC-1702`): one more word for why a connector run ended,
-- `timed_out`.
--
-- ── Why ───────────────────────────────────────────────────────────────
--
-- QA `TC-1702` found a run that never finished holding its connector's lock
-- for days. The orchestrator's run monitoring now ends such a run, and the
-- API ends the record of a run it had to stop itself (`SRC-39` decisions
-- D10, D11). Both record it as `failed` with `end_reason = 'timed_out'`, so
-- the history says the run outlived its limit instead of "a table failed".
-- The word is `connector_run::EndReason::TimedOut`; the API's run report
-- route (`lakehouse-api/src/routes/load_alerts.rs`) and the launch route
-- (`lakehouse-api/src/routes/connectors.rs`) write it.
--
-- ── Why DROP+ADD the CHECK ────────────────────────────────────────────
--
-- Postgres cannot extend a CHECK's set in place (see `0043` and `0097`).
-- Every value `0072` already allowed stays allowed, so no stored row is
-- invalidated; the constraint keeps `0072`'s own name and its
-- `end_reason IS NULL OR` part.
ALTER TABLE connector_run DROP CONSTRAINT connector_run_end_reason_check;
ALTER TABLE connector_run
    ADD CONSTRAINT connector_run_end_reason_check
    CHECK (end_reason IS NULL
           OR end_reason IN ('completed', 'table_failed', 'connector_paused', 'other', 'timed_out'));
