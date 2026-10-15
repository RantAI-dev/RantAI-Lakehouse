-- SRC-16 F1 (delivery review 2026-10-12): release the connectors that the
-- SRC-8 "pause" policy left stopped with nothing to approve.
--
-- A connector whose changes are seen only after the load (files, REST,
-- MongoDB, Kafka, SFTP) used to record its first change as `applied` and
-- still set the schema-change pause. Nothing was `pending`, so
-- `POST /api/connectors/{id}/schema-changes/approve` answered 404 and the
-- pause could not be lifted from the product. The code no longer does this
-- (a change seen after the load is `pending` under `pause`, SRC-16 task 1);
-- this statement releases the connectors it already left behind.
--
-- Targeted: only a pause whose reason is the schema-change one
-- (`lakehouse_store::schema_change::SCHEMA_CHANGE_PAUSE_REASON`, kept
-- identical by a test), and only when the connector has no pending change
-- at all. A pause set for another reason, or one a person can still lift by
-- approving a change, is not touched. Re-running changes nothing.

UPDATE connector
SET paused_reason = NULL,
    paused_at = NULL
WHERE paused_reason = 'Paused by the schema-change policy until a change is approved'
  AND NOT EXISTS (
    SELECT 1
    FROM connector_schema_change c
    WHERE c.connector_id = connector.id
      AND c.status = 'pending'
  );
