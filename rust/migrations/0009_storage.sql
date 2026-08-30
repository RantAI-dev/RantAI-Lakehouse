-- Phase 2, Task 2.6: storage lifecycle policies + tiering operations.
--
-- WHAT THIS IS: the Postgres backing for the write side of
-- `StorageService` -- `listPolicies`/`createLifecyclePolicy` and
-- `listOperations`/`restoreAsset`, which were still delegating to
-- `src/services/mock/storage.ts`. `getOverview` (byte/asset counts per
-- tier) stays exactly as Phase 1 left it, reading `ClickHouse`
-- (`system.parts` + the Bronze registry) -- that is observed fact about
-- what's actually stored, not authored/operational config, same
-- distinction Task 2.3's governance module drew.

-- ── lifecycle_policy ────────────────────────────────────────────────────
-- Maps `LifecyclePolicy` (storage.ts:11-20).
CREATE TABLE lifecycle_policy (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    scope              TEXT NOT NULL,
    hot_days           INTEGER NOT NULL,
    warm_days          INTEGER NOT NULL,
    cold_after_days    INTEGER NOT NULL,
    status             TEXT NOT NULL DEFAULT 'draft',
    estimated_savings  TEXT NOT NULL DEFAULT 'Pending estimate',
    last_applied_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- `LifecyclePolicy["status"]` is narrowed to `"ready" | "draft" |
    -- "paused"` in the contract (unlike `Policy`/`Pipeline`, which take the
    -- full `EntityStatus` union) -- constrained to match exactly.
    CONSTRAINT lifecycle_policy_status_check CHECK (status IN ('ready', 'draft', 'paused')),
    CONSTRAINT lifecycle_policy_name_unique UNIQUE (name)
);

-- ── tiering_op ──────────────────────────────────────────────────────────
-- Maps `TieringOp` (storage.ts:22-30). Both `listOperations` (a fixed
-- operational log) and `restoreAsset` (which appends a new "running"
-- restore op) read/write this one table.
CREATE TABLE tiering_op (
    id          TEXT PRIMARY KEY,
    asset       TEXT NOT NULL,
    asset_id    TEXT,
    from_tier   TEXT NOT NULL,
    to_tier     TEXT NOT NULL,
    status      TEXT NOT NULL,
    at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    detail      TEXT NOT NULL DEFAULT '',
    CONSTRAINT tiering_op_tier_check CHECK (
        from_tier IN ('hot', 'warm', 'cold', 'ai') AND to_tier IN ('hot', 'warm', 'cold', 'ai')
    ),
    CONSTRAINT tiering_op_status_check
        CHECK (status IN ('running', 'completed', 'failed', 'cancelled'))
);
