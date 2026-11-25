-- Query history: a run ended with Stop is `stopped`, and a stored engine
-- error becomes a classified sentence.
--
-- WHY: two findings of the owner's staging QA of QS-2 and QS-4 on
-- 2026-11-04 and 2026-11-05 (cases QS-2-08, QS-4-02, QS-4-10 and QS-4-09).
--
-- 1. A run the person ended with Stop was stored as `failed` with the
--    message `Stopped`, so the History filter could not tell "I stopped
--    it" from "the engine failed it". The run path now writes `stopped`.
--    The status check gets the new value and keeps `cancelled`, which
--    nothing writes today but older databases may hold. The first UPDATE
--    moves the rows already stored: it needs both `status = 'failed'` and
--    the exact message `Stopped` (the fixed sentence `Stopped`, written by
--    `routes::query::run`), so a failure whose text merely starts with
--    those letters, and a time-limit stop with its own sentence, stay as
--    they are.
--
-- 2. History kept the author's diagnosis of a ClickHouse statement error,
--    that is the engine's first line (`Code: 60. DB::Exception: ...`),
--    which can hold a catalog name. History and its API now keep a fixed
--    sentence by cause instead (AGENTS.md principle 4, SEC-11); the
--    result tab keeps showing the engine's diagnosis. The second UPDATE
--    replaces the stored text of the rows written before this change. It
--    keeps the `Reference: <id>` the row ended with, so the log line is
--    still findable, and it uses the default sentence for every row: the
--    cause (unknown table, unknown column, syntax) is not recovered from
--    old text, which would be guessing (principle 2). The predicate is the
--    shape ClickHouse statement errors are stored in, `Code: <n>.
--    DB::Exception`. Stored Trino messages have no fixed prefix (they are
--    `<redacted first line> Reference: <id>`), so a predicate for them
--    would also hit text this repository wrote; those rows are not
--    touched and age out with the 14-day History window.
--
-- Written to be run again: the constraint is dropped if it exists before
-- it is added, and both UPDATEs only match rows still in the old shape.
ALTER TABLE query_history DROP CONSTRAINT IF EXISTS query_history_status_check;
ALTER TABLE query_history
    ADD CONSTRAINT query_history_status_check
    CHECK (status IN ('completed', 'failed', 'cancelled', 'stopped', 'blocked'));

UPDATE query_history
SET status = 'stopped'
WHERE status = 'failed'
  AND message = 'Stopped';

UPDATE query_history
SET message = 'The engine could not run this statement.'
    || COALESCE(' ' || substring(message FROM 'Reference: [0-9a-f]+$'), '')
WHERE status = 'failed'
  AND message ~ '^Code: [0-9]+\. DB::Exception';
