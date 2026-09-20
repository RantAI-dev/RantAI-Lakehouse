-- A pipeline that has never run has no last run.
--
-- `last_run_at` defaulted to now() and `freshness_lag_seconds` to 0, so a
-- pipeline created a moment ago reported "Last run 1m ago · Fresh · 0s"
-- before anything had executed it. Both become nullable, and the default
-- goes away: unknown is written as NULL and read as "Never".
ALTER TABLE pipeline_definition ALTER COLUMN last_run_at DROP NOT NULL;
ALTER TABLE pipeline_definition ALTER COLUMN last_run_at DROP DEFAULT;
ALTER TABLE pipeline_definition ALTER COLUMN freshness_lag_seconds DROP NOT NULL;
ALTER TABLE pipeline_definition ALTER COLUMN freshness_lag_seconds DROP DEFAULT;

-- Rows the console authored before this migration carry the same
-- fabricated "ran just now" stamp; clear it for the ones that have no
-- orchestrator job to have run them.
UPDATE pipeline_definition
   SET last_run_at = NULL, freshness_lag_seconds = NULL
 WHERE status = 'draft';
