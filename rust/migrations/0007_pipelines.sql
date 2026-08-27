-- Phase 2, Task 2.5: authored pipeline definitions (createPipeline,
-- generatePipelineFromPrompt).
--
-- WHAT THIS IS: the Postgres backing for the two `PipelineService` methods
-- that author a brand-new pipeline the console knows about but no Dagster
-- job implements -- there is no generic "define an arbitrary pipeline and
-- have it run" engine behind Dagster; jobs are code-defined. A row here is
-- therefore a *draft declaration*, the same status a freshly authored
-- pipeline gets in `mock/pipelines.ts`'s `fromCreateInput`
-- (`status: "draft"`).
--
-- WHAT THIS IS NOT: a replacement for `GET /api/pipelines`'s Dagster-backed
-- list. `routes::pipelines::list` unions this table's rows onto that list
-- (same "don't let an authored thing vanish" fix as Task 2.3's governance
-- gap) so a freshly created pipeline is visible immediately, but its
-- `runs`/`triggerRun`/live status still come from Dagster once/if a real
-- job is wired up to it -- out of scope here.
CREATE TABLE pipeline_definition (
    id                       TEXT PRIMARY KEY,
    name                     TEXT NOT NULL,
    kind                     TEXT NOT NULL,
    status                   TEXT NOT NULL DEFAULT 'draft',
    owner                    TEXT NOT NULL DEFAULT 'Current user',
    source                   TEXT NOT NULL,
    target                   TEXT NOT NULL,
    connector_id             TEXT,
    source_asset_id          TEXT,
    target_asset_id          TEXT,
    schedule                 TEXT NOT NULL,
    last_run_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_run_at              TIMESTAMPTZ,
    sla_ok                   BOOLEAN NOT NULL DEFAULT true,
    freshness_lag_seconds    INTEGER NOT NULL DEFAULT 0,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT pipeline_definition_kind_check
        CHECK (kind IN ('batch', 'incremental', 'document', 'vector')),
    -- Same widened `EntityStatus` set as `policy.status` (Task 2.3): a
    -- freshly authored pipeline is always "draft", but pausePipeline/
    -- resumePipeline (also Task 2.5) move it between "draft"/"paused"/
    -- "ready" and there's no reason to hand-widen this constraint again
    -- when a later task adds more lifecycle transitions.
    CONSTRAINT pipeline_definition_status_check CHECK (status IN (
        'draft', 'validating', 'ready', 'scheduled', 'running', 'paused',
        'degraded', 'failed', 'completed', 'cancelled', 'blocked', 'partial',
        'archived'
    )),
    -- Every fixture in mock/pipelines.ts uses a distinct pipeline name;
    -- it's how an operator addresses a pipeline in conversation/CLI, same
    -- rationale as `policy_name_unique`.
    CONSTRAINT pipeline_definition_name_unique UNIQUE (name)
);
