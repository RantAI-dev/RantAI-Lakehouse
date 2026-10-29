-- SRC-4 T1.1 (plan Amendment 2): a third kind of discovery request, `test`.
--
-- Oracle's connection test cannot run in the API (it has no Oracle client;
-- `POST /api/connectors/{id}/test` answered `supported: false`). It runs in
-- the orchestrator, on the path `SRC-13` built: the API launches `discover_job`
-- with `kind = 'test'`, the job connects and posts the outcome back, and the
-- console polls. The row is the connector's own, separate from `list` and
-- `preview`, so a test never overwrites the person's table list ("latest
-- wins" is per connector and kind, see 0075).
--
-- Only the kind CHECK changes. A `test` row obeys every other CHECK of 0075 as
-- it stands: `done` holds a small result (`{"latencyMs": n}`, the time the
-- connection took), `failed` holds one of the closed error classes, and
-- `object` stays '' (the object check allows it for any kind but `list`).
-- 0075 is not edited; the constraint is dropped and added again here.
ALTER TABLE connector_discovery
    DROP CONSTRAINT IF EXISTS connector_discovery_kind_check;
ALTER TABLE connector_discovery
    ADD CONSTRAINT connector_discovery_kind_check
        CHECK (kind IN ('list', 'preview', 'test'));
