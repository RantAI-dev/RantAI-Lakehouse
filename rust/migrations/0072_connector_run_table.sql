-- SRC-11 review 1 BLOCKER 2: a refresh or a clear of one table says which
-- table, so the run list can name it ("Clear · public.orders", SRC-10
-- decision 5, load-modes feature page).
--
-- 0070 created `connector_run` without it: the run's `trigger_kind` says
-- `refresh` or `clear` but not for what, and the console's run list lost the
-- table the previous (orchestrator-search) list showed. 0070 is not edited
-- (it may already be applied); this migration only adds.
--
-- `table_name` is the source object's name as the orchestrator's start report
-- carries it (the run config's `object`), validated by the API with the same
-- name check as the other report fields. NULL for every other trigger and for
-- every run recorded before this migration: nothing is invented for them. The
-- CHECK keeps a table off a run that is about every table.

ALTER TABLE connector_run
    ADD COLUMN IF NOT EXISTS table_name TEXT;

ALTER TABLE connector_run
    ADD CONSTRAINT connector_run_table_only_for_one_table_check
        CHECK (table_name IS NULL OR trigger_kind IN ('refresh', 'clear'));
