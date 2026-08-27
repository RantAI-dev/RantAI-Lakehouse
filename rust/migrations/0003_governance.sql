-- Phase 2, Task 2.3: authored governance config (policies, and the
-- quality/classification/residency rules an operator writes down).
--
-- WHAT THIS IS: the Postgres backing for the write side of
-- `GovernanceService` -- `listPolicies`/`createPolicy` and the three
-- `create*Rule` methods that were still delegating to
-- `src/services/mock/governance.ts`.
--
-- WHAT THIS IS NOT: a replacement for the ClickHouse-backed reads
-- (`GET /api/governance/quality|audit|classification|residency|lineage`).
-- Those derive from `_silver_meta.quality`, Dagster run history, and the
-- dataset catalog -- observed facts about what actually ran, not authored
-- config -- and stay exactly as Phase 1 left them. A row created through
-- this migration's tables will not appear in those lists; see
-- `lakehouse_store::governance`'s module doc comment for the full
-- reasoning.

-- ── policy ──────────────────────────────────────────────────────────────
-- Maps `Policy` (governance.ts:12-22) in full, plus `conditions` (part of
-- `CreatePolicyInput` but not read back by the contract -- stored anyway,
-- since discarding caller-supplied data a table column could trivially
-- hold would be a pointless loss).
CREATE TABLE policy (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'draft',
    kind        TEXT NOT NULL,
    subjects    TEXT NOT NULL,
    resources   TEXT NOT NULL,
    effect      TEXT NOT NULL,
    conditions  TEXT,
    version     INTEGER NOT NULL DEFAULT 1,
    owner       TEXT NOT NULL DEFAULT 'Current user',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- `EntityStatus` (status.ts) is a 13-value closed union shared across
    -- the whole product; `createPolicy` can only ever produce "draft" or
    -- "ready" (mock/governance.ts), but the column accepts the full union
    -- so a later task giving policies a lifecycle (activate/deprecate/...)
    -- doesn't need a migration just to widen this check.
    CONSTRAINT policy_status_check CHECK (status IN (
        'draft', 'validating', 'ready', 'scheduled', 'running', 'paused',
        'degraded', 'failed', 'completed', 'cancelled', 'blocked', 'partial',
        'archived'
    )),
    -- Every fixture in mock/governance.ts uses a distinct policy name, and
    -- it is how an operator addresses a policy in conversation/CLI.
    CONSTRAINT policy_name_unique UNIQUE (name)
);

-- ── quality_rule ────────────────────────────────────────────────────────
-- Maps `QualityRule` (governance.ts:34-42). List-side stays ClickHouse
-- (`_silver_meta.quality`); this table exists only so `createQualityRule`
-- has somewhere durable to write.
CREATE TABLE quality_rule (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    asset         TEXT NOT NULL,
    dimension     TEXT NOT NULL,
    threshold     TEXT NOT NULL,
    severity      TEXT NOT NULL,
    last_status   TEXT NOT NULL DEFAULT 'warning',
    last_run_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT quality_rule_severity_check
        CHECK (severity IN ('critical', 'high', 'medium', 'low', 'info')),
    CONSTRAINT quality_rule_last_status_check
        CHECK (last_status IN ('passed', 'warning', 'failed')),
    CONSTRAINT quality_rule_name_unique UNIQUE (name)
);

-- ── classification_rule ─────────────────────────────────────────────────
-- Maps `ClassificationRule` (governance.ts:24-31). `column` is a reserved
-- word in SQL generally and awkward in Postgres identifiers, so the column
-- is named `column_name`; the Rust struct still serializes it as `column`.
CREATE TABLE classification_rule (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    asset            TEXT NOT NULL,
    column_name      TEXT,
    classification   TEXT NOT NULL,
    confidence       DOUBLE PRECISION NOT NULL DEFAULT 1,
    review_status    TEXT NOT NULL DEFAULT 'needs-review',
    masking_rule     TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT classification_rule_classification_check
        CHECK (classification IN ('public', 'internal', 'confidential', 'restricted')),
    CONSTRAINT classification_rule_review_status_check
        CHECK (review_status IN ('auto', 'reviewed', 'needs-review'))
    -- No uniqueness constraint: unlike policy/quality_rule/service_identity,
    -- neither the contract nor the mock fixtures treat asset(+column) as a
    -- natural key -- a second, superseding rule for the same column is a
    -- legitimate use case (re-review, tightened masking, ...).
);

-- ── residency_rule ──────────────────────────────────────────────────────
-- Maps `ResidencyRule` (governance.ts:59-66).
CREATE TABLE residency_rule (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant               TEXT NOT NULL,
    classification       TEXT NOT NULL,
    approved_sites       TEXT[] NOT NULL DEFAULT '{}',
    cross_site_allowed   BOOLEAN NOT NULL DEFAULT false,
    allowed_output       TEXT NOT NULL DEFAULT '',
    violations_7d        INTEGER NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT residency_rule_classification_check
        CHECK (classification IN ('public', 'internal', 'confidential', 'restricted'))
);

