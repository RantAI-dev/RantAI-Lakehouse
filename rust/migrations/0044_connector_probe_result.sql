-- Per-connector probe HISTORY -- distinct from `connector.health`/
-- `last_test_at` (0013_connectors.sql, 0028_connector_health_unknown_until_tested.sql),
-- which only ever hold the CURRENT state of the most recent probe.
-- `connector.health`/`last_test_at` answer "what is this connector's
-- status right now"; this table answers "what has this connector's status
-- been over time" -- a question the current-state columns cannot answer
-- once they are overwritten by the next test.
--
-- Written ONLY from `lakehouse_store::connectors::record_test_result`, in
-- the SAME transaction as the `connector` UPDATE that stamps
-- `health`/`last_test_at` -- see that function's doc comment. Both writes
-- committing together means the newest row here and `connector.health`/
-- `last_test_at` can never drift apart: there is no window where one has
-- been written and the other has not.
--
-- Only a SUPPORTED probe is recorded here, matching the rule
-- `record_test_result` already applies to `connector.health`/
-- `last_test_at`: an unsupported probe type was never actually dialed, so
-- it has no outcome to record -- writing a row for it would fabricate
-- history for a test that never ran (AGENTS.md principle 2).
--
-- Bounded growth: every insert immediately trims that connector's rows
-- down to the newest 200. This is a per-connector COUNT cap, not a
-- calendar window -- a connector tested every few minutes (a schedule)
-- and one tested once a month both keep exactly the same number of rows,
-- and neither can grow this table without bound just by being tested
-- often. A calendar window would let a frequently-tested connector
-- accumulate unboundedly within the window instead.
CREATE TABLE connector_probe_result (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    connector_id  TEXT NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    tested_at     TIMESTAMPTZ NOT NULL,
    ok            BOOLEAN NOT NULL,
    latency_ms    BIGINT,
    message       TEXT NOT NULL
);

-- Serves both `list_probe_results` (newest-first, per connector) and the
-- trim's own `ORDER BY id DESC LIMIT 200` subquery.
CREATE INDEX connector_probe_result_connector_id_id_idx
    ON connector_probe_result (connector_id, id DESC);
