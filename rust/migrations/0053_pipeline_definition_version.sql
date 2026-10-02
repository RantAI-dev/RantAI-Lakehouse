-- Definition version history for authored pipelines (Plan R4 2b).
--
-- # Why a separate table, not columns on pipeline_definition
--
-- Each `created` / `updated` / `restored` / `deleted` event must keep the
-- full editable state of that moment (`UpdatePipelineInput` fields plus
-- `name` and `status`), so a caller can answer "who changed this
-- pipeline, and what did it say before?". A history row outlives its
-- pipeline row — a `DELETE` writes one final `deleted` row, then the live
-- pipeline row is gone — and a `restore` reads back an older snapshot
-- through `get_pipeline_definition_version`, then re-applies it through
-- `update_pipeline`, which writes its own new `restored` row. Column-on-
-- `pipeline_definition` cannot model this: the live row cannot hold every
-- past snapshot, and a deleted row would erase its own history.
--
-- # Why no foreign key on pipeline_id
--
-- History outlives the live row. A `deleted` event is the row's last
-- entry, and it is written in the same transaction that removes the live
-- row from `pipeline_definition`. A foreign key would force the live row
-- to survive the delete (either by refusing the DELETE or by cascading
-- the history away with it), defeating the point. The integrity the
-- snapshot rows need is enforced by the application layer: every
-- `INSERT INTO pipeline_definition_version` is inside the same
-- transaction as the matching `INSERT INTO` / `UPDATE` / `DELETE` on
-- `pipeline_definition`, so a snapshot row can never appear without its
-- matching live write or vice versa.
--
-- # Why `snapshot` is jsonb, not columns
--
-- The snapshot's shape mirrors the editable state the API exposes
-- (`UpdatePipelineInput` fields plus `name` and `status`), which is the
-- shape a diff view will want to compare against API payloads. Adding
-- each new editable column as a typed SQL column here would force a
-- migration every time `update_pipeline` grows a field; a `jsonb`
-- snapshot carries whatever the `PipelineRow` has at the moment of the
-- write, and the `Pipeline` -> `Value` mapping in `pipelines.rs` is the
-- single source of truth for both the live row and every snapshot.
--
-- # Why `version = max(existing) + 1`, not a sequence
--
-- A `BIGSERIAL` on `(pipeline_id, version)` would couple the version
-- space across pipelines (a global sequence never restarts per
-- pipeline_id) and would not survive the deletion of every prior version
-- row. The store computes `version` per pipeline in the same transaction
-- as the write (`SELECT COALESCE(MAX(version), 0) + 1 FROM
-- pipeline_definition_version WHERE pipeline_id = $1` is the gap-free
-- allocator), so a `created`/`updated`/`restored` event is always one
-- greater than the last row for that pipeline, and a `deleted` row is
-- always one greater than the last `created`/`updated`/`restored` row.
--
-- # Why `event IN ('created','updated','restored','deleted')`
--
-- The four states a snapshot row can represent, fixed at write time:
-- the create endpoint, the update endpoint, the new restore endpoint,
-- and the delete endpoint. The CHECK constraint is the database-level
-- guard a typo at any store call site hits before the row is committed;
-- the route layer does not validate the enum in the application.
--
-- # Why `changed_by uuid NULL`
--
-- The route layer writes `principal.id.uuid()` (or `NULL` for a service
-- identity's own principal, which has no `app_user` row to FK against).
-- This matches `audit_event.principal_id`'s own nullable text column;
-- the historical record is the snapshot, the audit trail is the audit
-- table.
CREATE TABLE IF NOT EXISTS pipeline_definition_version (
    pipeline_id TEXT        NOT NULL,
    version     INT         NOT NULL,
    snapshot    JSONB       NOT NULL,
    event       TEXT        NOT NULL CHECK (event IN ('created', 'updated', 'restored', 'deleted')),
    changed_by  UUID        NULL,
    changed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (pipeline_id, version)
);