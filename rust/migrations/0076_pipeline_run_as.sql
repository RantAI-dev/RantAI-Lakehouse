-- BLD-1 slice 1 (plan `2026-10-15-bld-1-run-as-author.md`, decision D1): a
-- pipeline reads as its author. `pipeline_definition.run_as` is the user the
-- pipeline reads as, and it is the ONLY thing that says who that is:
-- `pipeline_definition.owner` is free text a human typed into a form
-- (`TEXT NOT NULL DEFAULT 'Current user'`, `0007_pipelines.sql`), which is a
-- label and not an identity. Before this column the orchestrator ran every
-- pipeline as its own ClickHouse user and read every table unmasked, so
-- anyone holding `pipeline:write` could copy a masked column into a Silver
-- table in clear (`GOV-4`). The run now goes through
-- `sql_rewrite::enforce` as the author, the same rewrite Query Studio,
-- dashboards and the Gold export already apply to that user's own queries.
--
-- # Why the user table and not the principal id
--
-- `pipeline_definition_version.changed_by` already holds a principal UUID,
-- and `PrincipalId::uuid()` returns one for a SERVICE identity too
-- (`lakehouse-auth/src/principal.rs`), which has no `app_user` row. A
-- `changed_by` is therefore not always a user, so it cannot be the column
-- this backfills from unreconciled: the foreign key below would reject a
-- service-created pipeline's own author. `run_as` references `app_user`
-- (`ON DELETE SET NULL`, the shape `0042_tenant_provisioning.sql` used for
-- `pipeline_definition.tenant_id`) so "the author is a user who exists" is
-- enforced by the database, not by a comment.
--
-- # Why `NULL` is a real state and not a placeholder
--
-- `NULL` means "this pipeline has no author to read as". Under decision D2
-- such a pipeline DOES NOT RUN: the run fails, without retry, with a fixed
-- message, until a Platform Admin assigns an author through
-- `PUT /api/pipelines/{id}/run-as`. That is fail closed (AGENTS.md
-- principle 3) — a pipeline whose author is unknown must not quietly keep
-- reading as an identity that can see everything.
--
-- # Why the backfill takes only a `created` version
--
-- `0053` wrote a version row per definition change and `0071` added the
-- `baseline` event for pipelines that predate that history at all; a
-- `baseline` row carries `changed_by = NULL` because nobody was recorded.
-- The only version that names the pipeline's AUTHOR is its `created` one:
-- later rows (`updated`/`restored`) name whoever edited it last, and D1 says
-- editing someone's pipeline must not change who it reads as. So the
-- backfill reads `created` only.
--
-- `DISTINCT ON (pipeline_id) ... ORDER BY version ASC` makes the choice
-- deterministic if a row somehow carries more than one `created` version —
-- Postgres would otherwise pick an arbitrary source row in an
-- `UPDATE ... FROM`, and an arbitrary author is not something to guess.
--
-- The `EXISTS (SELECT 1 FROM app_user ...)` guard is what keeps a
-- service-created pipeline NULL instead of failing the migration on the
-- foreign key: a `created` version written by a service identity has a
-- `changed_by` that is not a user, and there is no user to read as.
--
-- Additive only — nothing here alters a previously-applied statement.
ALTER TABLE pipeline_definition
    ADD COLUMN run_as UUID REFERENCES app_user (id) ON DELETE SET NULL;

COMMENT ON COLUMN pipeline_definition.run_as IS
    'The app_user this pipeline reads as: its source SELECT is rewritten by '
    'sql_rewrite::enforce as this user before the orchestrator runs it. NULL '
    'means no author is recorded and the pipeline does not run (BLD-1 D2).';

UPDATE pipeline_definition p
SET run_as = src.changed_by
FROM (
    SELECT DISTINCT ON (pipeline_id) pipeline_id, changed_by
    FROM pipeline_definition_version
    WHERE event = 'created'
      AND changed_by IS NOT NULL
    ORDER BY pipeline_id, version ASC
) AS src
WHERE p.id = src.pipeline_id
  AND p.run_as IS NULL
  AND EXISTS (SELECT 1 FROM app_user u WHERE u.id = src.changed_by);

CREATE INDEX pipeline_definition_run_as_idx
    ON pipeline_definition (run_as);