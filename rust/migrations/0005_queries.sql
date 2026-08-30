-- Phase 2, Task 2.4: query-studio persistence (saved queries, run history,
-- collaboration projects).
--
-- WHAT THIS IS: the Postgres backing for the write side of `QueryService`
-- -- `listSaved`/`listHistory`/`listCollaboration`/
-- `createCollaborationProject`, which were still delegating to
-- `src/services/mock/queries.ts`. `run`/`estimate` (ClickHouse) and
-- `generateSql` (LLM, `/api/agent/text-to-sql`) are untouched by this
-- migration -- they were already real in Phase 1.
--
-- `query_history` is written by `routes::query::run` after a successful
-- ClickHouse execution (see `lakehouse_store::queries::record_history`),
-- so `listHistory` reflects genuine past executions from the moment this
-- migration lands, growing forward from an empty table rather than
-- needing seed rows the way the other tables here do.

-- ── saved_query ─────────────────────────────────────────────────────────
-- Maps `SavedQuery` (queries.ts:3-9). Read-only from the console's point of
-- view: `QueryService` has no create/update/delete for it, only
-- `listSaved()`.
CREATE TABLE saved_query (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title       TEXT NOT NULL,
    sql         TEXT NOT NULL,
    owner       TEXT NOT NULL,
    tags        TEXT[] NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── query_history ───────────────────────────────────────────────────────
-- Maps `QueryHistoryItem` (queries.ts:11-22). `id` is TEXT, not UUID: it is
-- the same `q-<epoch_ms>` id `routes::query::run` already mints for
-- `QueryResult.id` (see `routes::query::epoch_ms`), so a history row and
-- the result that produced it share one identifier rather than minting a
-- second, unrelated one. `user_name`, not `user`: `user` is a reserved
-- word / built-in identity function in Postgres.
CREATE TABLE query_history (
    id               TEXT PRIMARY KEY,
    sql              TEXT NOT NULL,
    user_name        TEXT NOT NULL,
    at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    status           TEXT NOT NULL,
    duration_ms      BIGINT NOT NULL,
    scanned_bytes    BIGINT NOT NULL,
    cost_units       DOUBLE PRECISION NOT NULL,
    workload_class   TEXT NOT NULL,
    engine           TEXT NOT NULL,
    cache_assisted   BOOLEAN NOT NULL DEFAULT false,
    audit_event_id   TEXT,
    CONSTRAINT query_history_status_check
        CHECK (status IN ('completed', 'failed', 'cancelled', 'blocked'))
);

-- Every `listHistory` read is `ORDER BY at DESC LIMIT ...`; this is the
-- index that query actually uses once the table has real traffic in it.
CREATE INDEX query_history_at_idx ON query_history (at DESC);

-- ── collaboration_project ───────────────────────────────────────────────
-- Maps `CollaborationProject` (queries.ts:56-62). `members` is a plain
-- stored count, not a derived `COUNT(*)` subquery like
-- `identity.tenant.users`/`identity.role.members`: the contract has no
-- method that adds or removes a member from an existing project, so there
-- is no join table for a subquery to reconcile against -- the count is set
-- once, at creation, from `CreateCollaborationProjectInput.collaborators`.
CREATE TABLE collaboration_project (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL,
    members      INTEGER NOT NULL DEFAULT 0,
    description  TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

