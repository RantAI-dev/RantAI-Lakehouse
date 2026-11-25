-- BLD-14 (DEC-22, D8, P9): one outcome per re-upload and downstream pipeline.
--
-- A re-upload starts the pipelines that read the loaded table, then the
-- pipelines downstream of those. This table is where the chain sensor records
-- what happened to each one, so a pipeline's page can say why it ran or did
-- not (section 4's sentences). A row is keyed by the file-load run and the
-- pipeline: the first sensor write wins and a retry changes nothing. The
-- outcome/reason and outcome/run-id CHECKs make an incoherent record
-- impossible at the store, not just in the route that validates the body.
--
-- `tenant_id` is copied from `pipeline_definition` at insert so the read can
-- name a cause only when it is in the same tenant (section 3). There is
-- deliberately NO foreign key on `cause_pipeline_id`: deleting a cause keeps
-- the record and its name is then answered as null. P9 keeps the newest 20
-- rows per pipeline; the delete happens in the same transaction as the insert.
CREATE TABLE pipeline_chain_step (
    root_run_id       TEXT NOT NULL,         -- the file_ingest_job run of the re-upload
    pipeline_id       TEXT NOT NULL REFERENCES pipeline_definition(id) ON DELETE CASCADE,
    tenant_id         UUID,                  -- the pipeline's, copied at insert
    source_table      TEXT NOT NULL,         -- 'bronze.<table>' the re-upload loaded
    outcome           TEXT NOT NULL CHECK (outcome IN ('started', 'not_run', 'superseded')),
    reason            TEXT CHECK (reason IN ('upstream_failed', 'upstream_cancelled', 'wait_expired')),
    run_id            TEXT,                  -- outcome = 'started'
    cause_pipeline_id TEXT,                  -- outcome = 'not_run'
    recorded_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (root_run_id, pipeline_id),
    CHECK ((outcome = 'not_run') = (reason IS NOT NULL)),
    CHECK ((outcome = 'started') = (run_id IS NOT NULL))
);

-- The page reads the newest step for one pipeline; the index serves that
-- ordered lookup and P9's "all but the newest 20" sweep.
CREATE INDEX pipeline_chain_step_newest ON pipeline_chain_step (pipeline_id, recorded_at DESC);
