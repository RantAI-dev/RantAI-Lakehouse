-- Query Studio: a history row keeps what a run read and what it said.
--
-- WHY: `query_history` recorded only the shape of a completed run. It
-- has no `tables_read`, so nobody can find "the query on `orders` last
-- week"; it has no `message`; and the run route wrote `"completed"`
-- only, so the `failed` and `blocked` statuses the table's own check
-- allows had no producer at all. The history page (backlog `QS-4`)
-- records every run that reaches the server's answer, and to be
-- findable and explainable a run needs the tables it read and the
-- message its caller saw.
--
-- `tables_read TEXT[] NOT NULL DEFAULT '{}'`: the tables
-- `sql_rewrite::referenced_tables` finds in the statement as written,
-- empty when the statement does not parse. There is no backfill: the
-- runs recorded before this migration carry no table list, and
-- re-deriving one after the fact would be a guess about what the run's
-- policies, dialects and engines resolved on the day. The empty default
-- reads as "no tables recorded", the page says so, and `NOT NULL` keeps
-- every read query simple.
--
-- `message TEXT NULL`: the exact text the response gave the caller for
-- a failed or refused run — fixed and classified before it is stored,
-- because the engine's raw text never reaches a response and so never
-- reaches this column. NULL where there is nothing to say: the
-- completed runs, and every row written before this migration.
ALTER TABLE query_history
    ADD COLUMN tables_read TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN message TEXT NULL;
