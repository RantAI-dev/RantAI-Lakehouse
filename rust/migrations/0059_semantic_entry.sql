-- AI-16 (first line, docs/core/specs/ai-16.md): the semantic layer. One
-- row per table, and one per column, holding what that table or column
-- means in plain words, the other names people call it, and what a column
-- is for. The Copilot reads these rows into the DATA MAP in its system
-- prompt (`routes/ai/data_map.rs`), so a question that uses a word the
-- column name does not contain can still find the column.
--
-- Two writers share the table. The API drafts rows in the background with
-- the deployment's model (`status = 'draft'`, `model` set, `written_by`
-- NULL), and a person corrects a row through `PUT /api/semantic/{asset}`
-- (`status = 'confirmed'`, `written_by` set, `model` NULL). A person's text
-- always wins: the draft insert is `ON CONFLICT DO NOTHING`, so a draft
-- never replaces an existing row of either status, and only the confirm
-- upsert overwrites. The CHECK below that forbids a draft with an author
-- keeps the two states from blurring when a row is written around the
-- store functions.
--
-- This is a new table and not `asset_annotation` (`0032`): that table is
-- one row per asset, a write replaces the whole row, and the Catalog pages
-- own it. A per-column description with its own status could not live in
-- it without changing what those pages write. The Catalog's description of
-- a table is read beside this table, not merged into it.
--
-- There is no tenant column: a deployment has one set of tables in
-- `serving` and `silver`, so it has one description of each. The routes
-- that read and write this table refuse the same principals the other
-- catalog routes refuse (`catalog_tenant_refusal`).
--
-- `asset` is the qualified table name, `serving.<table>` or
-- `silver.<table>`, the shape `asset_annotation.asset_id` already uses for
-- those layers. `column_name` is `''` for the table itself and not NULL,
-- because a primary key cannot hold NULL; the table's own row and its
-- columns' rows then share one key, `(asset, column_name)`, and one
-- `list_for_asset` read.
--
-- Every bound is a CHECK, so a row written around the routes still cannot
-- crowd the prompt: `asset` and `column_name` at most 200 characters, a
-- description at most 400 for a table and 200 for a column, at most 6
-- synonyms of 1 to 40 characters each, a `role` that is NULL or one of
-- `measure`, `dimension`, `time`, `key` (and NULL for the table itself).
-- As in `0032`, a CHECK cannot hold a subquery, so the per-synonym shape is
-- an IMMUTABLE helper function.
CREATE FUNCTION semantic_entry_synonyms_are_valid(synonyms TEXT[])
RETURNS BOOLEAN AS $$
DECLARE
    synonym TEXT;
BEGIN
    IF cardinality(synonyms) > 6 THEN
        RETURN FALSE;
    END IF;
    FOREACH synonym IN ARRAY synonyms LOOP
        IF synonym IS NULL OR char_length(synonym) NOT BETWEEN 1 AND 40 THEN
            RETURN FALSE;
        END IF;
    END LOOP;
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE TABLE semantic_entry (
    asset       TEXT NOT NULL,
    column_name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL,
    synonyms    TEXT[] NOT NULL DEFAULT '{}',
    role        TEXT,
    status      TEXT NOT NULL,
    written_by  UUID,
    model       TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (asset, column_name),
    CONSTRAINT semantic_entry_asset_length_check
        CHECK (char_length(asset) <= 200),
    CONSTRAINT semantic_entry_column_name_length_check
        CHECK (char_length(column_name) <= 200),
    CONSTRAINT semantic_entry_description_length_check
        CHECK (char_length(description)
               <= CASE WHEN column_name = '' THEN 400 ELSE 200 END),
    CONSTRAINT semantic_entry_synonyms_shape_check
        CHECK (semantic_entry_synonyms_are_valid(synonyms)),
    CONSTRAINT semantic_entry_role_check
        CHECK (role IS NULL
               OR (column_name <> ''
                   AND role IN ('measure', 'dimension', 'time', 'key'))),
    CONSTRAINT semantic_entry_status_check
        CHECK (status IN ('draft', 'confirmed')),
    CONSTRAINT semantic_entry_draft_has_no_author_check
        CHECK (status <> 'draft' OR written_by IS NULL)
);
