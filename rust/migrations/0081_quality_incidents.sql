-- DATA-14 (docs/core/specs/data-14.md; plan
-- docs/superpowers/plans/2026-10-16-data-14-data-quality-in-depth.md, task 2;
-- findings F2, F3, F5): incidents, per-table schedules and a table's load
-- history, the three records "data quality in depth" needs and the platform
-- did not keep.
--
-- `quality_incident`: a failed check used to tell nobody. An incident is
-- opened by a failed run of an authored rule (`kind = 'check'`, `rule_id`
-- set) or by a learned anomaly (`kind = 'volume_anomaly'` or
-- `'freshness_anomaly'`, no rule), and stays open until a person resolves
-- it: a later passing run does not close it, because the person who owns
-- the table should see that it happened. At most one incident is open for
-- one (table, rule) or (table, anomaly kind) at a time; a failure that is
-- seen again moves `last_seen_at` instead of opening a second one. The
-- partial unique index is that rule, and it is what the store's upsert
-- conflicts on.
--   * `asset` is lower-cased by the store (`silver.orders`), as the audit
--     rows keyed by table are.
--   * `rule_id` is the `quality_rule.id` as text with NO foreign key: deleting
--     a rule resolves its open incidents ("rule deleted") and keeps them as
--     history, so the incident must outlive the rule row.
--   * `resolved_by_user_id` is the `PrincipalId` UUID of whoever resolved it
--     (no foreign key either: a service identity may resolve through the
--     API too). Resolving takes a written reason (`resolution`).
--
-- `quality_schedule`: when a table's rules run. No row means "every 60
-- minutes", today's behaviour. `mode = 'on_load'` also runs a table's rules
-- after each successful load (the API's `run_finished_event`); the interval
-- stays as the backstop for loads that never reach that event (change
-- capture, a failed connector run that committed some tables), so it is
-- always set, 60 minutes to one day.
--
-- `table_load`: one row per successful load of a table, written by the same
-- on-load hook whatever the table's schedule, so volume and freshness
-- anomalies can be learned from the table's own history (the Iceberg
-- snapshots are kept only 7 days and the ingest log has no per-table run
-- id). `rows_after` is the table's row count after the load. Kept 90 days
-- (the API purges older rows on its tick).
--
-- Never edit this file once applied; change it with a later migration.

CREATE TABLE quality_incident (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    asset               TEXT NOT NULL,
    rule_id             TEXT,
    kind                TEXT NOT NULL,
    opened_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    detail              TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'open',
    resolved_at         TIMESTAMPTZ,
    resolved_by_user_id UUID,
    resolution          TEXT,
    CONSTRAINT quality_incident_kind_check
        CHECK (kind IN ('check', 'volume_anomaly', 'freshness_anomaly')),
    CONSTRAINT quality_incident_status_check
        CHECK (status IN ('open', 'resolved')),
    -- A check incident names its rule; an anomaly names none.
    CONSTRAINT quality_incident_rule_check
        CHECK ((kind = 'check') = (rule_id IS NOT NULL)),
    CONSTRAINT quality_incident_resolved_check
        CHECK ((status = 'open' AND resolved_at IS NULL)
            OR (status = 'resolved' AND resolved_at IS NOT NULL)),
    CONSTRAINT quality_incident_length_check
        CHECK (char_length(asset) BETWEEN 1 AND 256
           AND char_length(detail) <= 1000
           AND (resolution IS NULL OR char_length(resolution) <= 500))
);

-- One open incident per (table, rule) or (table, anomaly kind).
CREATE UNIQUE INDEX quality_incident_one_open
    ON quality_incident (asset, (COALESCE(rule_id, kind)))
    WHERE status = 'open';
CREATE INDEX quality_incident_status_opened
    ON quality_incident (status, opened_at DESC);
CREATE INDEX quality_incident_asset
    ON quality_incident (asset);

CREATE TABLE quality_schedule (
    asset            TEXT PRIMARY KEY,
    mode             TEXT NOT NULL,
    interval_minutes INTEGER NOT NULL DEFAULT 60,
    updated_by       UUID,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT quality_schedule_mode_check
        CHECK (mode IN ('on_load', 'interval')),
    CONSTRAINT quality_schedule_interval_check
        CHECK (interval_minutes BETWEEN 60 AND 1440),
    CONSTRAINT quality_schedule_asset_length_check
        CHECK (char_length(asset) BETWEEN 1 AND 256)
);

CREATE TABLE table_load (
    asset      TEXT NOT NULL,
    loaded_at  TIMESTAMPTZ NOT NULL,
    rows_after BIGINT NOT NULL,
    PRIMARY KEY (asset, loaded_at),
    CONSTRAINT table_load_rows_check CHECK (rows_after >= 0),
    CONSTRAINT table_load_asset_length_check
        CHECK (char_length(asset) BETWEEN 1 AND 256)
);
-- The purge reads by age alone.
CREATE INDEX table_load_loaded_at ON table_load (loaded_at);
