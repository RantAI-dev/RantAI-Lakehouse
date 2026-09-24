-- Pipeline detail page: a real description instead of one fixed sentence
-- invented in the browser for every pipeline.
--
-- `incremental_column`, `fbic_enabled` and `transforms` are NOT added
-- here: `0036_pipeline_definition_steps.sql` already added them (as
-- `transforms JSONB`, validated by `transform_grammar` before insert,
-- not a `TEXT[]`). `description` is the only one of these columns the
-- schema did not already have.
ALTER TABLE pipeline_definition ADD COLUMN description TEXT;
