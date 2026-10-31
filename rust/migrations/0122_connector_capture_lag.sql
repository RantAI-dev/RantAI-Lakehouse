-- SRC-4 slice 4 (decisions S4-b to S4-e; feature page `databases-and-change-
-- capture.md`, decisions 6 and 7): how far behind the source a change-capture
-- connector is, and the limit at which a person is told.
--
-- 1. `connector_capture_table` keeps, per connector and per source table, what
--    the orchestrator last saw: `backlog` (messages on the table's topic that
--    the applier has not read) with the time it was seen (`observed_at`), the
--    source commit time of the last change the applier wrote to Bronze
--    (`last_source_ts`), and the last moment the table was known to be up to
--    date (`caught_up_at`: backlog 0 while `Kafka Connect` said `running`).
--    The API decides what these mean (`capture_lag.rs`); nothing here is a lag.
--    Every column but the key is NULL until the orchestrator reports it, so
--    "not known yet" is representable and no number is invented.
--
-- 2. `connector.lag_alert_minutes` (D7): the limit, 1 to 1440 minutes, default
--    15. The CHECK repeats the route's validation.
--
-- 3. `connector.lag_alert_open`: set while a `connector_lag` alert has fired and
--    the lag has not come back under the limit, so each crossing alerts once.
--    It is changed only by a compare-and-set UPDATE (`capture_lag.rs`).
--
-- Additive; never edited once applied.

CREATE TABLE IF NOT EXISTS connector_capture_table (
    connector_id   TEXT NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    table_name     TEXT NOT NULL,
    last_source_ts TIMESTAMPTZ,
    caught_up_at   TIMESTAMPTZ,
    backlog        BIGINT CHECK (backlog IS NULL OR backlog >= 0),
    observed_at    TIMESTAMPTZ,
    PRIMARY KEY (connector_id, table_name)
);

ALTER TABLE connector
    ADD COLUMN IF NOT EXISTS lag_alert_minutes INTEGER NOT NULL DEFAULT 15
        CHECK (lag_alert_minutes BETWEEN 1 AND 1440);

ALTER TABLE connector
    ADD COLUMN IF NOT EXISTS lag_alert_open BOOLEAN NOT NULL DEFAULT false;
