-- F2.3 / F2.4 (PR #59 review, plans/pipelines/day-1-fixes/
-- f2-tenant-scope-and-run-config.md): every `pipeline_definition_version`
-- row carries the owning tenant of its live `pipeline_definition` row.
-- The live row's own `tenant_id` was added by migration `0042`; the version
-- table got neither column nor constraint at creation, so a per-version
-- scope check at any HTTP route had to either (a) join through the live
-- row at every read (extra round trip, and broken for a deleted pipeline,
-- whose live row is gone by design — see migration `0053`'s "Why no
-- foreign key" comment) or (b) carry the value on the version row
-- itself. This migration takes (b): same shape as `0042`'s own
-- `pipeline_definition.tenant_id`, never interpolated into SQL, never
-- widens the table's primary key.
--
-- # Why a brand-new `event` value, `'baseline'`
--
-- Migration `0053`'s header comment claims "a `created` row is always 1
-- and every subsequent event is one greater than the last" and lists the
-- four values `'created' | 'updated' | 'restored' | 'deleted'`. Three of
-- its claims are wrong, and this migration corrects them in its own
-- header (not by editing `0053`, which is applied and immutable per the
-- "never edit a migration once applied" rule):
--
--   1. `0053` names the per-version reader as
--      `get_pipeline_definition_version`; the actual function is
--      `pipelines::get_definition_version` (PR #59 review).
--   2. `0053` says "a `restore` reads back an older snapshot through
--      `get_pipeline_definition_version`, then re-applies it through
--      `update_pipeline`". The store's restore path is
--      `pipelines::restore_pipeline`, not `update_pipeline` (the two
--      share `update_pipeline_with_event` internally but the public
--      surface is distinct — `restore_pipeline` is what `routes::
--      authored_pipelines::restore_version` calls).
--   3. `0053` says `changed_by` is NULL "for a service identity's own
--      principal, which has no `app_user` row to FK against". A service
--      identity has a UUID too (`lakehouse_auth::service_token` /
      `PrincipalId::Service(uuid)`); the route layer writes its
--      `principal.id.uuid()` verbatim, the column is just nullable for
--      the no-principal call sites (`create_pipeline` /
--      `delete_pipeline` when the route did not thread one through, e.g.
--      `routes::pipelines::generate`). The original comment conflated
--      service identity with absent principal.
--
-- The `baseline` event is the fourth correction. Every `pipeline_
-- definition` row that existed before migration `0053` shipped had no
-- version row at all — the table itself was created by `0053` — so the
-- version list for one of those legacy rows, asked today, would be empty
-- rather than carrying the row's original "who created this, with what?"
-- fact. The simplest honest backfill is one synthetic row per legacy
-- pipeline, with `version = 1`, `changed_by = NULL`, and a snapshot of
-- the live row's current state. Reusing `created` for those rows would
-- conflate a synthesized backfill with a real create event, so the
-- governance trail could not distinguish "this row was synthesized
-- during the F2.3 backfill" from "a real create ever existed"; that is
-- the same reason `0053` carries four distinct events instead of
-- overloading `created` with the backfill too. `baseline` is the missing
-- fifth value: it never appears in a real `created`/`updated`/`restored`/
-- `deleted` write path (the store function takes the event verbatim —
-- `update_pipeline_with_event(..., event)` — and the routes all pass one
-- of those four literals). The CHECK is widened to match the
-- documented vocabulary, not the other way around.
--
-- # Why no foreign key from `pipeline_definition_version.pipeline_id`
--
-- `0053` documents this in its own header: a `deleted` row outlives its
-- live row, so no FK. The same applies to `tenant_id`: a `baseline`
-- row's `tenant_id` is the snapshot of the live row's `tenant_id` at
-- backfill time; if the live row is later reassigned
-- (`PUT /api/pipelines/{id}/tenant`), historical versions stay on the old
-- tenant. A FK would force one of those rows to vanish, which is the
-- wrong direction: history should be immutable.
--
-- # Why `tenant_id` is nullable on this table too
--
-- A row inserted between `0053` and `0057` (every existing
-- `pipeline_definition_version` row today) was never stamped, because
-- the column did not exist when it was written. This migration backfills
-- those rows from the still-live `pipeline_definition` (see below); the
-- column is NULL only for the read paths that historically could not see one.
-- Post-`0057` writes always stamp `tenant_id` from the same transaction
-- (the create/update/restore paths read it off the live row's
-- `RETURNING`; the delete path reads it from the pre-delete `SELECT`
-- inside the same tx). F2.3 keeps the column nullable rather than NOT
-- NULL exactly so this migration can land in one statement without a
-- pre-condition that today's existing rows would break.
--
-- Additive only — nothing here alters a previously-applied statement.
ALTER TABLE pipeline_definition_version
    ADD COLUMN tenant_id UUID;

-- Widen the `event` CHECK to admit the `baseline` value the backfill below
-- writes. The store's own vocabulary is still only
-- `'created' | 'updated' | 'restored' | 'deleted'`; the four real write
-- paths never produce `baseline`, so a `'baseline'` row in the table is
-- by construction a migration artefact.
ALTER TABLE pipeline_definition_version
    DROP CONSTRAINT pipeline_definition_version_event_check;
ALTER TABLE pipeline_definition_version
    ADD CONSTRAINT pipeline_definition_version_event_check
        CHECK (event IN ('created', 'updated', 'restored', 'deleted', 'baseline'));

-- Backfill `tenant_id` for every existing version row from the live
-- `pipeline_definition`. A deleted pipeline's last `deleted` version row
-- would otherwise be left NULL: the inner join is intentional — a
-- pipeline row was removed, so the version row's tenant_id cannot be
-- recovered from the live row, and the version row will simply stay NULL
-- (visible only to an unrestricted reader; see F2.1's scope rule for
-- deleted-pipeline version reads).
UPDATE pipeline_definition_version v
SET tenant_id = p.tenant_id
FROM pipeline_definition p
WHERE v.pipeline_id = p.id
  AND v.tenant_id IS NULL;

-- F2.4: every pre-`0053` `pipeline_definition` row has no version row
-- at all. INSERT one synthetic baseline row per such pipeline so the
-- version-list / detail "this pipeline was created in version 1, by no
-- recorded principal, with this snapshot" answer is consistent across
-- old and new pipelines. `changed_by = NULL` because there is no
-- recorded principal to attribute the backfill to (a backfill row
-- attributed to the migration runner would be a fabricated attribution;
-- the original create event has no recoverable principal). `changed_at`
-- is the live row's `created_at` so the timeline stays monotonic with
-- the row's own history.
--
-- `sourceZone`/`sourceTable`/`targetZone`/`targetTable` mirror the
-- `lakehouse_store::pipelines::split_zone_table` Rust helper's exact
-- semantics (split on the first `.`, leave the empty side empty when no
-- dot is present). `split_part(..., 1)` alone does NOT match — it would
-- assign the whole un-dotted string to `sourceZone` and the empty
-- string to `sourceTable` — so each side is wrapped in a `strpos`-driven
-- CASE for the no-dot path. `0053`'s seeded rows were all pruned by
-- `0027_prune_seeded_activity.sql` before this migration applies, so a
-- no-dot `source`/`target` is purely a defence-in-depth branch.
INSERT INTO pipeline_definition_version
    (pipeline_id, version, snapshot, event, changed_by, changed_at, tenant_id)
SELECT
    p.id,
    1,
    jsonb_build_object(
        'kind',         p.kind,
        'sourceZone',   CASE WHEN strpos(p.source, '.') > 0
                             THEN substr(p.source, 1, strpos(p.source, '.') - 1)
                             ELSE '' END,
        'sourceTable',  CASE WHEN strpos(p.source, '.') > 0
                             THEN substr(p.source, strpos(p.source, '.') + 1)
                             ELSE p.source END,
        'incrementalColumn', p.incremental_column,
        'transforms',   to_jsonb(p.transforms),
        'fbicEnabled',  p.fbic_enabled,
        'targetZone',   CASE WHEN strpos(p.target, '.') > 0
                             THEN substr(p.target, 1, strpos(p.target, '.') - 1)
                             ELSE '' END,
        'targetTable',  CASE WHEN strpos(p.target, '.') > 0
                             THEN substr(p.target, strpos(p.target, '.') + 1)
                             ELSE p.target END,
        'schedule',     p.schedule,
        'owner',        p.owner,
        'description',  p.description,
        'maxRetries',   p.max_retries,
        'dependsOn',    to_jsonb(p.depends_on),
        'name',         p.name,
        'status',       p.status
    ),
    'baseline',
    NULL,
    COALESCE(p.created_at, now()),
    p.tenant_id
FROM pipeline_definition p
WHERE NOT EXISTS (
    SELECT 1 FROM pipeline_definition_version v WHERE v.pipeline_id = p.id
);

CREATE INDEX pipeline_definition_version_tenant_id_idx
    ON pipeline_definition_version (tenant_id);