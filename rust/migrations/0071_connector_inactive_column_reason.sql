-- SRC-11 (decisions D10, D11): a column the user leaves out of a table that
-- keeps its rows stays in the Bronze table with no new values, and is marked
-- inactive as a column removed at the source is (SRC-8, 0064).
--
-- `connector_inactive_column` was written only for removals the user
-- approved. Two kinds of mark now share it, told apart by `reason`:
--
--   * `removed_at_source` -- the SRC-8 mark. Cleared when the source shows
--     the column again and the change is accepted.
--   * `excluded` -- the column is left out of the load (`excludedColumns` on
--     the source object). Written and cleared by the ingest-spec save only.
--
-- The two must not clear each other: a before-load observation sees the whole
-- source table, including a column that is left out, and SRC-8 deletes the
-- mark of every column it sees accepted again. Those deletes now name
-- `reason = 'removed_at_source'`.
--
-- Every row that exists before this migration is a SRC-8 mark, so the
-- default is the right value for them (no UPDATE needed). Additive; 0064 and
-- 0065 are not edited.

ALTER TABLE connector_inactive_column
    ADD COLUMN IF NOT EXISTS reason TEXT NOT NULL DEFAULT 'removed_at_source';

ALTER TABLE connector_inactive_column
    ADD CONSTRAINT connector_inactive_column_reason_check
        CHECK (reason IN ('removed_at_source', 'excluded'));
