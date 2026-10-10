-- Deployment-wide reporting settings (`BI-9`, with `AI-3`): the time zone the
-- console reports in and the first day of the week.
--
-- Why a table: dashboards used to read "today" and "this week" from the
-- database server's clock, with Monday hard-coded, so nothing said which zone
-- a "month" meant. Grouping a timestamp by day, week or month (`BI-9`) and the
-- relative date filters (`BI-18`) now both read this one row, so a chart and
-- a filter cannot disagree about where a month begins. Written by
-- `PUT /api/settings/reporting` (permission `settings:write`, see
-- `lakehouse-api` `policy.rs`), read by every dashboard route.
--
-- One row at most: `singleton` is the primary key and can only be true. No row
-- means "the defaults" (`Asia/Jakarta`, `monday`), which the API supplies, so
-- this migration seeds nothing and an operator's first save is a plain
-- insert. The API checks `time_zone` against the engine's own list
-- (`system.time_zones`) before it saves; the CHECK below is only the shape
-- (letters, digits, `_`, `+`, `-`, `/` between segments, at most 64
-- characters), which keeps a name from ever being able to leave a SQL string
-- literal even if a row is edited by hand. `week_start` is closed to the two
-- values the builder implements.
--
-- `updated_by` is the principal id of whoever saved last, for the audit trail
-- of a setting that moves every date on every dashboard.
CREATE TABLE IF NOT EXISTS reporting_settings (
    singleton  BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    time_zone  TEXT        NOT NULL
        CHECK (length(time_zone) BETWEEN 1 AND 64
               AND time_zone ~ '^[A-Za-z][A-Za-z0-9_+-]*(/[A-Za-z0-9_+-]+)*$'),
    week_start TEXT        NOT NULL CHECK (week_start IN ('monday', 'sunday')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by TEXT        NOT NULL DEFAULT ''
);
