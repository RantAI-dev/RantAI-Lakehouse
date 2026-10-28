-- The install-wide rule for remembered dashboard results (`BI-4` part B,
-- decision D7 of
-- `docs/superpowers/plans/2026-10-24-bi-4-caching-parallel-tiles.md`; feature
-- page `docs/core/features/result-caching-and-parallel-tiles.md`, decisions 1
-- and 2).
--
-- Why two columns on `reporting_settings` and not a table of its own: the
-- rule is one value for the whole deployment, set from the same Admin >
-- Settings screen and with the same `settings:write` permission as the report
-- time zone, and a deployment that never saved has no row (the caller then
-- supplies "default"). A dashboard's and a chart's own rules live with the
-- dashboard (ClickHouse `bi_board.cache_json`) and the chart definition, not
-- here.
--
-- `result_cache_mode`: `default` follows the signed rule (a result is used
-- until the tables under it change, and never longer than 24 hours);
-- `fresh` remembers nothing; `minutes` remembers for `result_cache_minutes`
-- (1 minute to 24 hours). The `CHECK`s are the backstop: the route refuses
-- anything else with a sentence first, and a row can only say minutes when
-- the mode is `minutes`.
--
-- Number choice: 0110, not the next free 0103, because the BI-19 part B
-- branch takes 0103 and up and a gap costs nothing.
ALTER TABLE reporting_settings
    ADD COLUMN IF NOT EXISTS result_cache_mode TEXT NOT NULL DEFAULT 'default'
        CHECK (result_cache_mode IN ('default', 'fresh', 'minutes')),
    ADD COLUMN IF NOT EXISTS result_cache_minutes INT NULL
        CHECK (result_cache_minutes IS NULL OR result_cache_minutes BETWEEN 1 AND 1440);

ALTER TABLE reporting_settings
    ADD CONSTRAINT reporting_settings_result_cache_minutes_with_mode
        CHECK ((result_cache_mode = 'minutes') = (result_cache_minutes IS NOT NULL));
