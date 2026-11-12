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
-- Why the backfill, and why it is functions: every existing row is a
-- one-input, at-most-one-transform-step pipeline under D3's rule (the table
-- name becomes the input name, or `input_1` when that name is not a usable
-- identifier; the transforms become one `transform` step named `transformed`,
-- or `transformed_1` when the input took that name without regard to case).
-- Putting that rule in SQL functions, rather than inline in the UPDATE, is
-- what makes "the migration's answer equals the store's answer on the same
-- fixtures" a test that can be run after the fact: the store test calls the
-- functions directly and compares them with `pipelines::legacy_definition`,
-- and calls the backfill function on rows it inserted itself. Inline SQL
-- could only be asserted by re-writing it in the test, which is the
-- duplicate this exists to avoid (AGENTS.md rule 4).
--
-- `source` is split the way the store splits it (`split_zone_table`): at the
-- FIRST dot. A location with no dot is a table with an empty zone, and a
-- location with two dots keeps the rest as the table
-- (`serving.sales.orders` -> `serving` + `sales.orders`). `split_part(.., 2)`
-- would have said `sales`, which is a different table: a stored pipeline
-- would have its definition rewritten to read somewhere else (review R1-B4).
--
-- Additive only: three columns with defaults, a backfill, and `IMMUTABLE`
-- helper functions. No existing column is dropped or renamed, so a rollback
-- target that predates this change still reads every row.

-- D2's name rule, in SQL: an `Ident` of at most 48 characters that does not
-- start with `__`.
CREATE OR REPLACE FUNCTION pipeline_legacy_name_is_usable(candidate TEXT)
RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT candidate IS NOT NULL
       AND length(candidate) BETWEEN 1 AND 48
       AND candidate ~ '^[A-Za-z_][A-Za-z0-9_]*$'
       AND left(candidate, 2) <> '__'
$$;

-- The half before the first dot, or the empty zone for a location with none.
CREATE OR REPLACE FUNCTION pipeline_legacy_zone(location TEXT)
RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN position('.' IN location) > 0
            THEN left(location, position('.' IN location) - 1)
        ELSE ''
    END
$$;

-- The rest after the first dot, or the whole location for one with none —
-- the inverse of `pipelines::split_zone_table`.
CREATE OR REPLACE FUNCTION pipeline_legacy_table(location TEXT)
RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN position('.' IN location) > 0
            THEN substr(location, position('.' IN location) + 1)
        ELSE location
    END
$$;

-- D3: one input named after the table, `input_1` when the table name is not a
-- usable name.
CREATE OR REPLACE FUNCTION pipeline_legacy_input(location TEXT)
RETURNS JSONB
LANGUAGE sql IMMUTABLE AS $$
    SELECT jsonb_build_object(
        'name', CASE
                    WHEN pipeline_legacy_name_is_usable(pipeline_legacy_table(location))
                        THEN pipeline_legacy_table(location)
                    ELSE 'input_1'
                END,
        'zone', pipeline_legacy_zone(location),
        'table', pipeline_legacy_table(location)
    )
$$;

-- D3: the transforms become one `transform` step that reads the input, named
-- `transformed` — or `transformed_1` when the input is called that, compared
-- WITHOUT regard to case (D2's uniqueness rule), so the compiler never sees a
-- duplicate name and refuses a pipeline that used to run (review R1-B4).
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
                            WHEN lower(pipeline_legacy_input(location) ->> 'name') = 'transformed'
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

-- Fill every row that has no definition yet from `source` and `transforms`,
-- by D3's rule. A row that already carries one is left alone, which is why
-- running this again is a no-op for every row the store has written since —
-- and the migration itself is the only time every row is in the first state.
CREATE OR REPLACE FUNCTION pipeline_backfill_legacy_definition()
RETURNS void
LANGUAGE sql AS $$
    UPDATE pipeline_definition
       SET inputs = jsonb_build_array(pipeline_legacy_input(source)),
           steps = pipeline_legacy_steps(source, transforms),
           input_tables = ARRAY[source]
     WHERE inputs = '[]'::jsonb
       AND steps = '[]'::jsonb;
$$;

SELECT pipeline_backfill_legacy_definition();
