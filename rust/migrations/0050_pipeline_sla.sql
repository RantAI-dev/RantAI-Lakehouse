-- WS6 plan 1f: per-pipeline service-level agreement. The detail route and
-- the runs route read this to surface `late`, `overDuration`, and `slaOk`
-- alongside each pipeline and run; the alert pipeline reads it to decide
-- whether `pipeline_slow`, `pipeline_late`, and `pipeline_volume_drop`
-- should fire.
--
-- Keyed by TEXT, not by a foreign key to `pipeline_definition.id`:
-- the orchestrator publishes runs under a Dagster job name, which may be
-- either the slug (`silver_orders`) or the upstream `pl-` id used by the
-- Dagster `pipeline_run_failed_sensor` (the same identifier
-- `pipeline_run_event.pipeline_id` accepts). One row per job, no joins,
-- idempotent upserts from the PUT route.
--
-- Both thresholds are nullable and strictly positive when present (a
-- zero-second SLA would be "always late" or "always over" by
-- construction). The CHECK is the database-level guarantee; the route
-- rejects the same shape at the API boundary.
CREATE TABLE pipeline_sla (
    pipeline_id          TEXT PRIMARY KEY,
    -- Per-run duration SLA, in whole seconds. NULL = "no duration SLA";
    -- the runs route then emits `overDuration: null` for every run.
    max_duration_seconds INT  CHECK (max_duration_seconds > 0),
    -- "Late by N seconds since last SUCCESS" threshold. NULL = "no late
    -- threshold"; the detail route emits `late: null` in that case.
    late_after_seconds   INT  CHECK (late_after_seconds > 0),
    updated_by           UUID NOT NULL,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
