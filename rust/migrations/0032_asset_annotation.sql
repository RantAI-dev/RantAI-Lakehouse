-- WS2 §13: console-only catalog metadata (owner, steward, tags,
-- description) the registry itself never captures.
--
-- Numbered 0032, not 0029: `0029` was reserved for this task while
-- `0030_table_maintenance_policy.sql` and
-- `0031_governance_admin_catalog_read.sql` landed first. Filling `0029` now
-- would number this migration BELOW two that are already applied on every
-- database that has run this branch, so it would apply out of order there.
-- `0029` is left permanently unused rather than backfilled — nobody should
-- go looking for a migration file that was never written.
--
-- The primary key is `asset_id`, not `table_name`: `routes/catalog.rs`
-- emits a different id shape per layer, and none of them is a bare table
-- name:
--   * Bronze rows use the registry slug with no layer prefix
--     (`bronze_catalog_row`, `catalog.rs:531`; `bronze_detail_body`,
--     `catalog.rs:946`);
--   * Silver rows use `format!("silver.{name}")`
--     (`silver_catalog_row`, `catalog.rs:560`);
--   * serving (Gold) rows use `format!("serving.{name}")`
--     (`gold_catalog_row`, `catalog.rs:588`);
--   * `build_namespaces` emits a bare namespace name as its own `id`
--     (`catalog.rs:645`) — not an asset id, but the same "no fixed prefix"
--     shape;
--   * `clickhouse_detail_body` passes through whatever id `detail` was
--     called with verbatim (`catalog.rs:878`).
-- So this table's key is the full, already-computed catalog id string
-- (`GET /api/catalog/{id}`'s own path parameter), not a re-derived table
-- name.
--
-- Bounds exist so any `catalog:write` holder cannot store unbounded text
-- through this table — `PUT /api/catalog/{id}/annotation` enforces the
-- same bounds in the handler (returning 400 naming the field) before a
-- request ever reaches this table; the CHECK constraints here are defense
-- in depth, not a substitute for that validation:
--   * `asset_id`: at most 200 characters (the `{id}` path segment's bound);
--   * `owner`/`steward`: at most 128 characters each;
--   * `description`: at most 4,000 characters;
--   * `tags`: at most 20 entries, each 1-64 characters matching
--     `^[a-z0-9][a-z0-9_-]*$`. A `CHECK` cannot contain a subquery in
--     Postgres (even one that, like `unnest(tags)`, only ever reads the
--     row's own column), so the per-element shape is validated by a small
--     `IMMUTABLE` helper function instead of an inline `EXISTS`.
CREATE FUNCTION asset_annotation_tags_are_valid(tags TEXT[])
RETURNS BOOLEAN AS $$
DECLARE
    tag TEXT;
BEGIN
    IF cardinality(tags) > 20 THEN
        RETURN FALSE;
    END IF;
    FOREACH tag IN ARRAY tags LOOP
        IF char_length(tag) > 64 OR tag !~ '^[a-z0-9][a-z0-9_-]*$' THEN
            RETURN FALSE;
        END IF;
    END LOOP;
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

CREATE TABLE asset_annotation (
    asset_id    TEXT PRIMARY KEY,
    owner       TEXT,
    steward     TEXT,
    tags        TEXT[] NOT NULL DEFAULT '{}',
    description TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT asset_annotation_asset_id_length_check
        CHECK (char_length(asset_id) <= 200),
    CONSTRAINT asset_annotation_owner_length_check
        CHECK (owner IS NULL OR char_length(owner) <= 128),
    CONSTRAINT asset_annotation_steward_length_check
        CHECK (steward IS NULL OR char_length(steward) <= 128),
    CONSTRAINT asset_annotation_description_length_check
        CHECK (description IS NULL OR char_length(description) <= 4000),
    CONSTRAINT asset_annotation_tags_shape_check
        CHECK (asset_annotation_tags_are_valid(tags))
);
