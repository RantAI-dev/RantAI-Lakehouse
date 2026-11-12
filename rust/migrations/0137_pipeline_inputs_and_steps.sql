-- BLD-1 part 1 (D1, D3, D13): a pipeline becomes a list of named inputs, an
-- ordered list of named steps, and one output table. `source` and
-- `transforms` stay, and keep being written, because every reader that
-- predates this change (the orchestrator's `sourceZone`/`sourceTable`, the
-- target-lag readers, `GET /api/pipelines/{id}`'s flat fields) still reads
-- them; `inputs` and `steps` are the new shape, and `input_tables` is what
-- the target-lag readers use for "which tables does this pipeline read"
-- (D11). All three are written from ONE place in the store
-- (`pipelines::definition_columns`), never by hand.
--
-- Why the backfill, and why it is a function: every existing row is a
-- one-input, at-most-one-transform-step pipeline under D3's rule (the table
-- name becomes the input name, or `input_1` when that name is not a usable
-- identifier; the transforms become one `transform` step named `transformed`,
-- or `transformed_1` when the input took that name). Putting that rule in two
-- SQL functions, rather than inline in the UPDATE, is what makes
-- "the migration's answer equals the store's answer on the same fixtures"
-- a test that can be run after the fact: the store test calls the functions
-- directly and compares them with `pipelines::legacy_definition`. Inline
-- SQL could only be asserted by re-writing it in the test, which is the
-- duplicate this exists to avoid (AGENTS.md rule 4).
--
-- Additive only: three columns with defaults, a backfill, and two `IMMUTABLE`
-- helper functions. No existing column is dropped or renamed, so a rollback
-- target that predates this change still reads every row.

-- D2's name rule, in SQL: an `Ident` of at most 48 characters that does not
-- start with `__` (the orchestrator's reserved prefix).
CREATE OR REPLACE FUNCTION pipeline_legacy_name_is_usable(candidate TEXT)
RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT candidate IS NOT NULL
       AND length(candidate) BETWEEN 1 AND 48
       AND candidate ~ '^[A-Za-z_][A-Za-z0-9_]*$'
       AND left(candidate, 2) <> '__'
$$;

-- D3: one input named after the table, `input_1` when the table name is not a
-- usable name. `split_part(..., 2)` is the table half of the stored
-- `<zone>.<table>` location `create_pipeline` builds.
CREATE OR REPLACE FUNCTION pipeline_legacy_input(location TEXT)
RETURNS JSONB
LANGUAGE sql IMMUTABLE AS $$
    SELECT jsonb_build_object(
        'name', CASE
                    WHEN pipeline_legacy_name_is_usable(split_part(location, '.', 2))
                        THEN split_part(location, '.', 2)
                    ELSE 'input_1'
                END,
        'zone', split_part(location, '.', 1),
        'table', split_part(location, '.', 2)
    )
$$;

-- D3: the transforms become one `transform` step that reads the input, named
-- `transformed` (`transformed_1` when the input itself is called that); a row
-- with no transforms has no steps.
CREATE OR REPLACE FUNCTION pipeline_legacy_steps(location TEXT, transforms JSONB)
RETURNS JSONB
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN transforms IS NULL
          OR jsonb_typeof(transforms) <> 'array'
          OR jsonb_array_length(transforms) = 0
            THEN '[]'::jsonb
        ELSE jsonb_build_array(
            jsonb_build_object(
                'name', CASE
                            WHEN pipeline_legacy_input(location) ->> 'name' = 'transformed'
                                THEN 'transformed_1'
                            ELSE 'transformed'
                        END,
                'kind', 'transform',
                'reads', pipeline_legacy_input(location) ->> 'name',
                'transforms', transforms
            )
        )
    END
$$;

ALTER TABLE pipeline_definition
    ADD COLUMN IF NOT EXISTS inputs JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS steps JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS input_tables TEXT[] NOT NULL DEFAULT '{}';

-- Every row that predates this change was a single `source`, so its
-- `input_tables` is exactly that location. A row written after this change
-- carries the same value through the store's own write path; running this
-- UPDATE again is idempotent for those rows because both are derived from
-- `source` and `transforms`, which D3 keeps in step with the new shape.
UPDATE pipeline_definition
   SET inputs = jsonb_build_array(pipeline_legacy_input(source)),
       steps = pipeline_legacy_steps(source, transforms),
       input_tables = ARRAY[source];
