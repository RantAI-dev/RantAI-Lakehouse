-- DATA-12 part 2 (docs/core/specs/data-12.md, "Governed tags"; plan
-- docs/superpowers/plans/2026-10-14-data-12-parts-2-3-governed-tags-and-picker-marks.md,
-- task 1): tag keys with agreed values, beside today's free tags.
--
-- Free tags (`asset_annotation.tags`, `0032`) stay exactly as they are: one
-- lowercase word, anyone with `catalog:write` types it. A governed tag is a
-- `key = value` pair where an admin (`governance:write`) defined the key and
-- the values it may take, and a table can only be given one of those values.
--
-- Two tables, not a column on `asset_annotation`, because a governed pair
-- points at a key row that must be able to say "still in use":
--   * `governed_tag_key`: the definition. `allowed_values` is an array, at
--     most 50 entries, each 1-64 characters matching `^[a-z0-9][a-z0-9_-]*$`
--     (the same word shape as a free tag). A key is never renamed.
--   * `asset_governed_tag`: one row per (table, key). `asset_id` is the FULL
--     catalog id, the same string `asset_annotation.asset_id` holds (see
--     `0032`'s header). `ON DELETE RESTRICT` means a key still given to a
--     table cannot be dropped by accident; the store refuses first, with the
--     count, so the foreign key is the backstop rather than the message.
--
-- What a CHECK cannot say, and who says it instead. Postgres CHECKs see one
-- row, so these rules live in `lakehouse_store::governed_tag`, inside the
-- transaction that writes, not here:
--   * "the value is in the key's `allowed_values`": `set_asset_tags` reads the
--     key rows under a row lock and refuses a value outside the list, so a
--     concurrent edit of the list cannot slip a stale value in;
--   * "at most 20 governed tags per table": counted in `set_asset_tags`;
--   * "removing a value no table still uses": `update_key_values` counts the
--     rows that would be left holding a removed value, under the same lock.
-- The handlers answer 400 with the store's sentence; the CHECKs below are
-- defense in depth for shape and length, like `0032` and `0070`.
CREATE FUNCTION governed_tag_values_are_valid(vals TEXT[])
RETURNS BOOLEAN AS $$
DECLARE
    v TEXT;
BEGIN
    IF vals IS NULL OR cardinality(vals) > 50 THEN
        RETURN FALSE;
    END IF;
    FOREACH v IN ARRAY vals LOOP
        IF char_length(v) > 64 OR v !~ '^[a-z0-9][a-z0-9_-]*$' THEN
            RETURN FALSE;
        END IF;
    END LOOP;
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE TABLE governed_tag_key (
    key            TEXT PRIMARY KEY,
    allowed_values TEXT[] NOT NULL,
    created_by     TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT governed_tag_key_shape_check
        CHECK (char_length(key) <= 64 AND key ~ '^[a-z0-9][a-z0-9_-]*$'),
    CONSTRAINT governed_tag_key_values_shape_check
        CHECK (governed_tag_values_are_valid(allowed_values)),
    CONSTRAINT governed_tag_key_created_by_length_check
        CHECK (char_length(created_by) <= 128)
);

CREATE TABLE asset_governed_tag (
    asset_id TEXT NOT NULL,
    tag_key  TEXT NOT NULL REFERENCES governed_tag_key (key) ON DELETE RESTRICT,
    value    TEXT NOT NULL,
    set_by   TEXT NOT NULL,
    set_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (asset_id, tag_key),
    CONSTRAINT asset_governed_tag_asset_id_length_check
        CHECK (char_length(asset_id) <= 200),
    CONSTRAINT asset_governed_tag_value_shape_check
        CHECK (char_length(value) <= 64 AND value ~ '^[a-z0-9][a-z0-9_-]*$'),
    CONSTRAINT asset_governed_tag_set_by_length_check
        CHECK (char_length(set_by) <= 128)
);

-- The in-use count and the overlay both look rows up by key, not by asset.
CREATE INDEX asset_governed_tag_by_key ON asset_governed_tag (tag_key);
