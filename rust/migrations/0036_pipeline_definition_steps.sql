-- WS4 (grand plan §6): `pipeline_definition` (0007_pipelines.sql) stores
-- only name/kind/source/target/schedule/owner today — `CreatePipelineBody`
-- (routes/pipelines.rs) already ACCEPTS incremental_column/transforms/
-- fbic_enabled but drops them (`#[allow(dead_code, reason = "accepted for
-- contract compatibility, not yet stored")]`, routes/pipelines.rs:263-280)
-- because there was nowhere to put them. This migration adds that
-- storage so an authored pipeline's create form is not a lie about what
-- persists, and so `authored_factory.py` (Phase E) has something real to
-- read when building `authored__<id>` jobs.
--
-- Additive only: three new nullable/defaulted columns on an existing
-- table, no backfill needed (every pre-existing row simply gets the
-- defaults, which mean exactly "no incremental column configured", "no
-- transforms configured", "FBIC not enabled" — the honest state of a
-- pipeline created before this migration existed).
--
-- Numbered 0036, not 0035 as WS4's plan doc names it: 0035 was already
-- taken by 0035_connector_type.sql, landed on WS3 before this branch's
-- migration was written.
ALTER TABLE pipeline_definition
    ADD COLUMN incremental_column TEXT,
    ADD COLUMN fbic_enabled       BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN transforms         JSONB NOT NULL DEFAULT '[]';

COMMENT ON COLUMN pipeline_definition.transforms IS
    'A JSON array of transform-step strings from the fixed vocabulary '
    '(dedupe(key), filter(expr), rename(a,b), cast(col,type), '
    'select(cols)) — validated by lakehouse-api''s transform_grammar '
    'module BEFORE insert (POST /api/pipelines returns 400 on anything '
    'outside the grammar), never executed as raw SQL from this column.';
