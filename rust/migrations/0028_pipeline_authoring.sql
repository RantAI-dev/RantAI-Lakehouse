-- What the Create Pipeline wizard collects, actually stored.
--
-- The wizard asks for an incremental column, a set of transforms and the
-- FBIC toggle across three steps; `routes::pipelines::CreatePipelineBody`
-- accepted all three and dropped them on the floor (its own fields carried
-- `#[allow(dead_code, reason = "accepted for contract compatibility, not
-- yet stored")]`). Asking someone for configuration and then discarding it
-- is worse than not asking, so the columns exist from here on.
--
-- `description` joins them: the detail page used to display one fixed
-- sentence for every pipeline, invented in the browser.
ALTER TABLE pipeline_definition ADD COLUMN description TEXT;
ALTER TABLE pipeline_definition ADD COLUMN incremental_column TEXT;
ALTER TABLE pipeline_definition ADD COLUMN transforms TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE pipeline_definition ADD COLUMN fbic_enabled BOOLEAN NOT NULL DEFAULT false;
