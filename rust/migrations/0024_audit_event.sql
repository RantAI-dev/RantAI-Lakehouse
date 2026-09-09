-- Task T0.3 (copilot operations handover, plan section 3.3): the
-- append-only audit sink.
--
-- WHY this table exists: every gate decision the copilot makes (allowed,
-- refused, needs_confirmation, needs_approval) and every tool call it
-- actually executes needs a durable, queryable record independent of
-- `agent_run.steps` (which is scoped to one run and denormalized JSONB) and
-- independent of `approval_item` (which only exists for WriteHigh actions
-- and is mutated in place as a decision is made). `audit_event` is the one
-- place that records ALL of it — copilot and, optionally, console
-- identity actions (login/logout) — as flat, uniformly-shaped rows a
-- governance view can list, filter, and paginate without reconstructing
-- history from three different tables.
--
-- INVARIANT: this table is append-only by convention. Nothing in this
-- schema or the repository layer built on top of it (`lakehouse-store`'s
-- `audit` module) ever updates or deletes a row — there is deliberately no
-- UPDATE/DELETE helper in that module. A completed action does not "amend"
-- an earlier audit row; it writes a NEW row with its own `outcome`
-- (e.g. `needs_approval` now, `approved` and `executed` later as separate
-- rows once a human decides). This mirrors how `approval_item` already
-- keeps `requested_at`/`decided_at` as two points in time rather than
-- overwriting a single status in place, just taken further: every
-- transition is its own row, not just the terminal one.
--
-- `args` is JSONB so a tool call's arguments can be attached structurally,
-- but nothing in this table redacts them: `lakehouse-store::audit::insert`
-- documents that redaction (never writing a `secretRef` value's plaintext,
-- never writing raw SQL result rows) is entirely the CALLER's
-- responsibility before it builds the `NewAuditEvent` it passes in.
CREATE TABLE audit_event (
    id               TEXT PRIMARY KEY,
    at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    principal_id     TEXT,
    principal_kind   TEXT,
    actor_label      TEXT,
    action           TEXT NOT NULL,
    resource_kind    TEXT,
    resource_id      TEXT,
    args             JSONB NOT NULL DEFAULT '{}',
    outcome          TEXT NOT NULL,
    detail           TEXT,
    run_id           TEXT REFERENCES agent_run (id) ON DELETE SET NULL,
    approval_id      TEXT REFERENCES approval_item (id) ON DELETE SET NULL,
    session_id       TEXT,
    CONSTRAINT audit_event_principal_kind_check
        CHECK (principal_kind IS NULL OR principal_kind IN ('user', 'service', 'copilot', 'schedule')),
    CONSTRAINT audit_event_outcome_check
        CHECK (outcome IN (
            'allowed', 'executed', 'refused', 'needs_confirmation',
            'needs_approval', 'approved', 'rejected', 'failed'
        ))
);

-- The audit/governance view lists newest-first; this is the index that
-- query hits.
CREATE INDEX audit_event_at_idx ON audit_event (at DESC);

-- Looking up "everything that happened to this resource" (e.g. one
-- connector, one alert rule) is the other primary access pattern.
CREATE INDEX audit_event_resource_idx ON audit_event (resource_kind, resource_id);
