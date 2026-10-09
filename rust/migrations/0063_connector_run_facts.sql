-- SRC-7 (F4): a connector's health changed only when someone pressed Test
-- (`record_test_result` was the only writer of `connector.health`), and no
-- column held when a real run last succeeded or failed, or how many runs in a
-- row had failed. The load-failure alerts need the streak (the "3 failures in
-- a row" rule, decision D2) and the Sources list must show the last success,
-- the last failure and the streak without a manual test.
--
-- Three additive columns, written together with `health` by one store
-- function (`connectors::record_run_result`, decision D6) from the orchestrator's
-- run reports:
--   last_run_success_at  when the latest successful run finished
--   last_run_failure_at  when the latest failed run finished
--   failure_streak       failed runs since the last success; 0 after a success
--
-- All existing rows start with no run time and a streak of 0: nothing has
-- been measured, so nothing is invented (AGENTS.md principle 2). Idempotent
-- (`IF NOT EXISTS`), never edited once applied.

ALTER TABLE connector ADD COLUMN IF NOT EXISTS last_run_success_at TIMESTAMPTZ;
ALTER TABLE connector ADD COLUMN IF NOT EXISTS last_run_failure_at TIMESTAMPTZ;
ALTER TABLE connector
    ADD COLUMN IF NOT EXISTS failure_streak INTEGER NOT NULL DEFAULT 0
    CHECK (failure_streak >= 0);
