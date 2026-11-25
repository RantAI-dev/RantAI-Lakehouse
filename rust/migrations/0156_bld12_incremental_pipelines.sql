-- BLD-12: real incremental materialization for authored pipelines.
--
-- `pipeline_definition` already carried one incremental hint
-- (`incremental_column`, migration `0036`) but every authored run rebuilt the
-- whole target with a staging table + `EXCHANGE TABLES`. This migration adds
-- the three fields that decide HOW a run materializes: the strategy (watermark
-- append vs time-partition replace), the partition granularity, and the
-- lookback window that keeps late-arriving rows from replacing a partial
-- partition. Defaults keep every existing row on the old full-rebuild
-- behaviour, so a deploy does not change any pipeline's semantics by itself.
--
-- The CHECK constraints are the database-level backstop for the route's 400s
-- (`routes::pipelines::create` and `routes::authored_pipelines::update`); a
-- value that reaches the store from any other caller is refused here rather
-- than reaching the orchestrator as an unsupported strategy.
--
-- The orchestrator reads these columns through `GET /api/pipelines/runnable`
-- and passes them as `IncrementalConfig` to
-- `dagster/dispar_orchestrate/incremental_write.py`.

ALTER TABLE pipeline_definition
    ADD COLUMN incremental_strategy TEXT NOT NULL DEFAULT 'append'
        CHECK (incremental_strategy IN ('append', 'partition_replace')),
    ADD COLUMN partition_granularity TEXT NOT NULL DEFAULT 'day'
        CHECK (partition_granularity IN ('day', 'month')),
    ADD COLUMN lookback_hours INTEGER NOT NULL DEFAULT 0
        CHECK (lookback_hours >= 0 AND lookback_hours <= 720);
