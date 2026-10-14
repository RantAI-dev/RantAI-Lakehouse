-- SRC-11 (decisions D5, D6, D7, D8): the product's own record of a connector
-- run, of each attempt at each table, and of the run's log.
--
-- Until now a run was only visible in two places the product does not own:
-- the orchestrator's run list (searched by time, newest 100 runs of all
-- connectors together, so one connector's list could be short or empty) and
-- ClickHouse `bronze_meta.ingest_run` (one row per table and run, no run id,
-- no attempt, never deleted). The console joined the two by time.
--
-- Written ONLY by the API, from reports the orchestrator posts on its
-- `ingest:read` service path (`routes::connector_runs`). The orchestrator
-- sends structured facts and fixed class words; the API renders the log
-- lines from them. Nothing here holds a driver's, a source's or the
-- orchestrator's error text (AGENTS.md principle 4): `error_class` and
-- `end_reason` are CHECKed against closed lists, so even a buggy
-- orchestrator cannot store free text in them. `table_name` is the source
-- object's name as the connector's own spec spells it.
--
-- 1. `connector_run`: one row per orchestrator run id. `attempts` and
--    `rows_loaded` are totals kept on the row, so the list of the last 50
--    runs stays exact after retention (D6) removed the attempts themselves.
--    `status` is `running` until the orchestrator reports the end, or until
--    the run sensor confirms the orchestrator's own final status (a run that
--    was killed never reports); `paused` is a run that loaded nothing
--    because the connector was paused (D13).
-- 2. `connector_run_attempt`: one row per table and attempt, written when
--    the attempt ends. `wait_seconds` is the wait before the next attempt
--    (NULL when there is none). A failed attempt always has a class.
-- 3. `connector_run_log`: the lines of the run log, rendered by the API.
--    The log is kept 30 days (D5); a daily job removes the lines and the
--    attempts older than that and leaves `connector_run` (D6).
--
-- Rows are removed with their connector (ON DELETE CASCADE), as
-- `connector_probe_result` (0044) is.

CREATE TABLE IF NOT EXISTS connector_run (
    run_id       TEXT        PRIMARY KEY,
    connector_id TEXT        NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    trigger_kind TEXT        NOT NULL,
    status       TEXT        NOT NULL DEFAULT 'running',
    started_at   TIMESTAMPTZ NOT NULL,
    ended_at     TIMESTAMPTZ,
    rows_loaded  BIGINT      NOT NULL DEFAULT 0,
    attempts     INTEGER     NOT NULL DEFAULT 0,
    end_reason   TEXT,
    CONSTRAINT connector_run_trigger_check
        CHECK (trigger_kind IN ('schedule', 'user', 'refresh', 'clear', 'other')),
    CONSTRAINT connector_run_status_check
        CHECK (status IN ('running', 'succeeded', 'failed', 'paused')),
    CONSTRAINT connector_run_end_reason_check
        CHECK (end_reason IS NULL
               OR end_reason IN ('completed', 'table_failed', 'connector_paused', 'other')),
    CONSTRAINT connector_run_rows_check CHECK (rows_loaded >= 0),
    CONSTRAINT connector_run_attempts_check CHECK (attempts >= 0)
);

CREATE INDEX IF NOT EXISTS connector_run_by_connector
    ON connector_run (connector_id, started_at DESC);

CREATE TABLE IF NOT EXISTS connector_run_attempt (
    run_id       TEXT        NOT NULL REFERENCES connector_run(run_id) ON DELETE CASCADE,
    table_name   TEXT        NOT NULL,
    attempt      INTEGER     NOT NULL,
    started_at   TIMESTAMPTZ NOT NULL,
    ended_at     TIMESTAMPTZ NOT NULL,
    rows_loaded  BIGINT      NOT NULL DEFAULT 0,
    result       TEXT        NOT NULL,
    error_class  TEXT,
    wait_seconds INTEGER,
    PRIMARY KEY (run_id, table_name, attempt),
    CONSTRAINT connector_run_attempt_number_check CHECK (attempt >= 1),
    CONSTRAINT connector_run_attempt_rows_check CHECK (rows_loaded >= 0),
    CONSTRAINT connector_run_attempt_result_check CHECK (result IN ('succeeded', 'failed')),
    CONSTRAINT connector_run_attempt_class_check
        CHECK (error_class IS NULL OR error_class IN (
            'unreachable', 'timeout', 'source_busy', 'api_unreachable',
            'sign_in_refused', 'address_refused', 'unsupported', 'setting_rejected',
            'schema_decision_pending', 'other')),
    -- A failure always says what kind; a success never does.
    CONSTRAINT connector_run_attempt_class_iff_failed
        CHECK ((result = 'failed') = (error_class IS NOT NULL)),
    CONSTRAINT connector_run_attempt_wait_check
        CHECK (wait_seconds IS NULL OR (result = 'failed' AND wait_seconds >= 0))
);

CREATE INDEX IF NOT EXISTS connector_run_attempt_by_start
    ON connector_run_attempt (started_at);

CREATE TABLE IF NOT EXISTS connector_run_log (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id     TEXT        NOT NULL REFERENCES connector_run(run_id) ON DELETE CASCADE,
    at         TIMESTAMPTZ NOT NULL,
    event      TEXT        NOT NULL,
    table_name TEXT,
    attempt    INTEGER,
    line       TEXT        NOT NULL,
    CONSTRAINT connector_run_log_event_check
        CHECK (event IN ('run_started', 'attempt', 'retry_wait', 'schema_decision_pending',
                         'columns_left_out', 'report_gap', 'run_ended'))
);

CREATE INDEX IF NOT EXISTS connector_run_log_by_run ON connector_run_log (run_id, id);
CREATE INDEX IF NOT EXISTS connector_run_log_by_time ON connector_run_log (at);
