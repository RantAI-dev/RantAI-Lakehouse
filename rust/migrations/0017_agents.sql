-- Phase 2, Task 2.9: the `agents` domain — digital employees (agent
-- definitions), tools, workflows, runs, and approvals. Postgres backing for
-- `src/services/mock/agents.ts`.
--
-- SCOPE -- read before touching this file.
--
-- This is authored configuration plus operational state, not an execution
-- runtime: there is no agent runtime, orchestrator, or tool-invocation
-- engine anywhere in this repository (see `AI_PROJECT_INSIGHTS.md`).
-- `agent_run` stores run *records* (history), the same static shape
-- `mock/agents.ts`'s `RUNS` constant already exposes — nothing in this
-- schema or the repository layer built on top of it ever launches an
-- agent, invokes a tool, or produces a NEW run. `AgentService` has no
-- "run this agent" method, so this is a complete, honest backing for the
-- contract as written.
CREATE TABLE agent_employee (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    purpose          TEXT NOT NULL,
    owner            TEXT NOT NULL DEFAULT 'Current user',
    autonomy         TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'draft',
    budget_limit     DOUBLE PRECISION NOT NULL DEFAULT 0,
    budget_spent     DOUBLE PRECISION NOT NULL DEFAULT 0,
    budget_reserved  DOUBLE PRECISION NOT NULL DEFAULT 0,
    allowed_tools    TEXT[] NOT NULL DEFAULT '{}',
    data_scope       TEXT NOT NULL DEFAULT '',
    approval_rate    DOUBLE PRECISION NOT NULL DEFAULT 0,
    success_rate     DOUBLE PRECISION NOT NULL DEFAULT 0,
    recent_runs      BIGINT NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_employee_autonomy_check CHECK (autonomy IN ('L1', 'L2', 'L3', 'L4')),
    CONSTRAINT agent_employee_name_unique UNIQUE (name)
);

CREATE TABLE agent_tool (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    version          TEXT NOT NULL,
    publisher        TEXT NOT NULL,
    permission       TEXT NOT NULL,
    health           TEXT NOT NULL DEFAULT 'healthy',
    approval_status  TEXT NOT NULL DEFAULT 'pending',
    deprecated       BOOLEAN NOT NULL DEFAULT false,
    rate_limit       TEXT NOT NULL,
    usage_30d        BIGINT NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_tool_health_check CHECK (health IN ('healthy', 'degraded', 'unhealthy', 'unknown')),
    CONSTRAINT agent_tool_approval_status_check CHECK (approval_status IN ('pending', 'approved', 'rejected')),
    CONSTRAINT agent_tool_name_unique UNIQUE (name)
);

CREATE TABLE agent_workflow (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft',
    owner               TEXT NOT NULL DEFAULT 'Current user',
    trigger             TEXT NOT NULL,
    steps               BIGINT NOT NULL DEFAULT 0,
    last_run_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    approval_required   BOOLEAN NOT NULL DEFAULT false,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_workflow_name_unique UNIQUE (name)
);

-- Run history records. See the migration header comment: never written to
-- by a live execution path, only seeded / read.
CREATE TABLE agent_run (
    id               TEXT PRIMARY KEY,
    employee_id      TEXT NOT NULL REFERENCES agent_employee (id) ON DELETE CASCADE,
    workflow_id      TEXT REFERENCES agent_workflow (id) ON DELETE SET NULL,
    status           TEXT NOT NULL,
    trigger          TEXT NOT NULL,
    actor            TEXT NOT NULL,
    delegated_user   TEXT,
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at         TIMESTAMPTZ,
    budget_consumed  DOUBLE PRECISION NOT NULL DEFAULT 0,
    -- `AgentRun.steps`: a fixed, historical step trace. Denormalized as
    -- JSONB (no route in this domain mutates an individual step), matching
    -- the mock's plain array-of-objects shape exactly.
    steps            JSONB NOT NULL DEFAULT '[]',
    audit_event_id   TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The approval lifecycle. Also the source `AgentRun.approvals` is derived
-- from (`WHERE run_id = $1`), so a decision made through
-- `POST /api/agents/approvals/{id}/decide` is reflected in both places
-- without duplicated state.
CREATE TABLE approval_item (
    id               TEXT PRIMARY KEY,
    employee_id      TEXT NOT NULL REFERENCES agent_employee (id) ON DELETE CASCADE,
    employee_name    TEXT NOT NULL,
    run_id           TEXT REFERENCES agent_run (id) ON DELETE SET NULL,
    workflow_id      TEXT REFERENCES agent_workflow (id) ON DELETE SET NULL,
    action           TEXT NOT NULL,
    resource         TEXT,
    reason           TEXT,
    impact           TEXT,
    evidence         TEXT[],
    policy           TEXT,
    cost_estimate    DOUBLE PRECISION,
    expires_at       TIMESTAMPTZ,
    requested_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    status           TEXT NOT NULL DEFAULT 'pending',
    risk             TEXT NOT NULL DEFAULT '',
    decided_at       TIMESTAMPTZ,
    comment          TEXT,
    audit_event_id   TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT approval_item_status_check CHECK (status IN ('pending', 'approved', 'rejected'))
);
