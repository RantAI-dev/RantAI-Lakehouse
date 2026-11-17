-- SRC-13 (decisions D1, D9, D10): the answer to a table listing or a column
-- preview that the ORCHESTRATOR produced.
--
-- The API lists SQL sources, object storage and REST endpoints itself and
-- answers in the same request. MongoDB, Kafka, SFTP and Oracle (and the column
-- preview of REST, MongoDB, Kafka and SFTP) can only be read by the
-- orchestrator, which holds their drivers. The API launches a short job
-- (`discover_job`); the job posts its result back on `ingest:read` (the same
-- service path as the SRC-11 run reports) and the console polls for it. This
-- table is where that result waits.
--
-- One row per connector and kind (`list` or `preview`): the latest request
-- wins (D1), so nothing grows without bound and no retention job is needed.
-- A new request overwrites the row and puts it back to `running`.
--
-- `request_id` is a random id the API generates and hands to the job in its run
-- config. The job echoes it, and a result is stored only if it matches the row's
-- id while the row is still `running`. That makes an old job's late answer, a
-- repeated post, and a post for a request that was since replaced all harmless,
-- without the API having to know the orchestrator's own run id before the job
-- may already have finished.
--
-- AGENTS.md principle 4: nothing here can hold a driver's, a source's or the
-- orchestrator's error text. A failure is `error_class`, CHECKed against the
-- closed list of decision D10 (the API repeats the list in
-- `lakehouse_store::connector_discovery::ErrorClass` and renders the fixed
-- sentence for each class). `result` is stored only after the API parsed it
-- into its typed shape with unknown fields refused, and is capped here too so a
-- bug in that code cannot store an unbounded document.
--
-- Rows go with their connector (ON DELETE CASCADE), as `connector_run` (0072)
-- does.

CREATE TABLE IF NOT EXISTS connector_discovery (
    connector_id TEXT        NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL,
    request_id   TEXT        NOT NULL,
    -- The source object a `preview` is about ("public.orders"); '' for a list.
    object       TEXT        NOT NULL DEFAULT '',
    -- What the list was narrowed by (a schema or a glob); '' when by nothing.
    scope        TEXT        NOT NULL DEFAULT '',
    state        TEXT        NOT NULL DEFAULT 'running',
    error_class  TEXT,
    result       JSONB,
    started_at   TIMESTAMPTZ NOT NULL,
    ended_at     TIMESTAMPTZ,
    PRIMARY KEY (connector_id, kind),
    CONSTRAINT connector_discovery_kind_check
        CHECK (kind IN ('list', 'preview')),
    CONSTRAINT connector_discovery_state_check
        CHECK (state IN ('running', 'done', 'failed')),
    CONSTRAINT connector_discovery_class_check
        CHECK (error_class IS NULL OR error_class IN (
            'unreachable', 'sign_in_refused', 'address_refused', 'no_permission',
            'timeout', 'too_many_objects', 'not_supported', 'failed')),
    -- A state says exactly what it holds: a running request holds nothing, a
    -- finished one a result, a failed one a class.
    CONSTRAINT connector_discovery_state_shape_check
        CHECK ((state = 'running' AND result IS NULL AND error_class IS NULL AND ended_at IS NULL)
            OR (state = 'done' AND result IS NOT NULL AND error_class IS NULL AND ended_at IS NOT NULL)
            OR (state = 'failed' AND result IS NULL AND error_class IS NOT NULL AND ended_at IS NOT NULL)),
    CONSTRAINT connector_discovery_object_check
        CHECK (char_length(object) <= 256 AND (kind = 'preview' OR object = '')),
    CONSTRAINT connector_discovery_scope_check
        CHECK (char_length(scope) <= 256),
    CONSTRAINT connector_discovery_result_size_check
        CHECK (result IS NULL OR octet_length(result::text) <= 3145728)
);
