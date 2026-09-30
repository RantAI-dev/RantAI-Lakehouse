-- Dedup table for pipeline-run events the orchestrator emits (a sensor in
-- `dagster/dispar_orchestrate/pipeline_events.py` posts each one to
-- `POST /api/pipelines/events/run-failed`, the Rust route looks it up here
-- before alerting).
--
-- Primary key `(run_id, kind)` so a sensor retry never double-alerts: a
-- second `INSERT ... ON CONFLICT (run_id, kind) DO NOTHING` for the same
-- row is a no-op, and the route reads the boolean "was anything inserted"
-- back to decide whether to evaluate alert rules this turn. The same
-- table backs the kinds 1f adds (`slow`, `volume_drop`, `late`), so a
-- later part's sensor can write into it without a new migration.
CREATE TABLE IF NOT EXISTS pipeline_run_event (
    run_id      TEXT        NOT NULL,
    pipeline_id TEXT        NOT NULL,
    kind        TEXT        NOT NULL,
    seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, kind)
);
