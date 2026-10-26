-- Time and row limits an administrator sets for charts and Query Studio
-- (`BI-5`, decision D1 of
-- `docs/superpowers/plans/2026-10-24-bi-5-limits-and-stop.md`; feature page
-- `docs/core/features/query-limits-and-stop.md`, decisions 1 and 2).
--
-- Why Postgres and not constants: the limits used to be written into the
-- code (30 s and 2,000 rows on a SQL source, 5,000 map points, 2,000 Query
-- Studio rows) and a customer could not change them. They are read per
-- request (through a 30 s in-process memo), so a change needs no deploy.
--
-- `query_limits`: one row (`singleton`), seeded here with the feature
-- defaults so a read never finds nothing. Every column has a `CHECK` for the
-- feature's range; the route refuses out-of-range values with a sentence and
-- these are the backstop. The defaults below are the feature page's (time 60
-- s, chart rows 2,000, map points 5,000, Query Studio rows 2,000); the
-- store's `QueryLimits::default` and its test pin the same numbers.
--
-- `query_limits_role`: a role's own time limit (decision 1: a person with
-- several roles gets the longest; a role without a row uses the install's).
-- Deleting a role deletes its limit (`ON DELETE CASCADE`). Row limits are per
-- install only (decision 2).
CREATE TABLE IF NOT EXISTS query_limits (
    singleton       BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    timeout_seconds INT NOT NULL DEFAULT 60
        CHECK (timeout_seconds BETWEEN 10 AND 1800),
    chart_rows      INT NOT NULL DEFAULT 2000
        CHECK (chart_rows BETWEEN 100 AND 100000),
    map_points      INT NOT NULL DEFAULT 5000
        CHECK (map_points BETWEEN 100 AND 50000),
    studio_rows     INT NOT NULL DEFAULT 2000
        CHECK (studio_rows BETWEEN 100 AND 100000),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by      TEXT NOT NULL DEFAULT ''
);

INSERT INTO query_limits (singleton) VALUES (TRUE) ON CONFLICT DO NOTHING;

CREATE TABLE IF NOT EXISTS query_limits_role (
    role_id         UUID PRIMARY KEY REFERENCES role (id) ON DELETE CASCADE,
    timeout_seconds INT NOT NULL CHECK (timeout_seconds BETWEEN 10 AND 1800)
);
