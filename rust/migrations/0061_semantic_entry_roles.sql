-- AI-16 (docs/core/specs/ai-16.md): two more column roles for the semantic
-- layer (`0059`), and a one-time deletion of the drafts written before them.
--
-- `non_additive` marks a number that must not be added up across rows: a
-- count of distinct things, an average, a rate, a percentage, a price.
-- `flag` marks a 0/1 or yes/no column. Before this migration the only
-- numeric role was `measure`, so the drafting pass called both kinds of
-- column a measure. The first automated eval showed what that costs: a
-- mart's distinct-count column was summed over rows, and the chat answered
-- 806 where the answer was 313.
--
-- Postgres cannot change the body of a CHECK in place, so the constraint is
-- dropped and added back under the same name with the six roles. A role
-- stays NULL for the table's own row (`column_name = ''`), as in `0059`.
--
-- Every row with `status = 'draft'` is deleted. Those drafts were written
-- under a prompt that let them copy values from the facts into a
-- description (a range, a count of rows, a span of years, all of which go
-- stale as rows arrive) and call a flag a measure. The drafting pass writes
-- them again under the new prompt: it asks about every table that has no
-- table-level row, so a table whose own row was a draft is drafted afresh.
-- A person's confirmed entries are not touched. A table whose own row is
-- confirmed is skipped by the pass, so its draft columns are deleted here
-- and are not redrafted; a person fills those through
-- `PUT /api/semantic/{asset}`.
ALTER TABLE semantic_entry
    DROP CONSTRAINT semantic_entry_role_check,
    ADD CONSTRAINT semantic_entry_role_check
        CHECK (role IS NULL
               OR (column_name <> ''
                   AND role IN ('measure', 'dimension', 'time', 'key',
                                'flag', 'non_additive')));

DELETE FROM semantic_entry WHERE status = 'draft';
