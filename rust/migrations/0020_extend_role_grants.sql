-- Extends the seeded role grants for four route groups that were left at
-- `Policy::RequiresAuth` (any authenticated principal, including a
-- read-only Analyst) instead of a real permission check:
--
--   1. `/api/agents/employees/{id}/{suspend,resume,revoke}` (digital
--      employee lifecycle control)      -> `agent:manage`
--   2. `/api/ops/workloads/{id}/cancel` (kills a running ClickHouse query)
--                                        -> `workload:cancel`
--   3. `/api/storage/restore` (destructive restore)
--                                        -> `storage:restore`
--   4. `/api/alerts` POST/PUT/DELETE (alert rule CRUD; fires real webhooks
--      and emails)                      -> `alert:write`
--
-- Token naming follows the existing `resource:action` taxonomy
-- (`lakehouse_auth::permissions`) rather than inventing a parallel one:
-- `agent:manage` sits in the same `agent` resource family Approver's
-- `agent:approve` already uses; `workload`/`storage`/`alert` are new
-- resource families named after the domain they gate, matching how
-- `dashboard:read`/`connector:manage`/`audit:read` were named.
--
-- Only TWO of the four get a seed grant here:
--
-- * Data Engineer -> `workload:cancel`: Data Engineer already operates
--   pipelines end-to-end (`pipeline:*`) and is the role that would notice
--   and need to kill a runaway query it (or a pipeline it owns) launched.
-- * Data Engineer -> `alert:write`: alert rules are operational tooling
--   over the same pipelines/connectors Data Engineer already manages
--   (`pipeline:*`, `connector:manage`); the natural owner of "define a
--   threshold alert on a data pipeline" is the same role that builds the
--   pipeline.
--
-- `agent:manage` and `storage:restore` deliberately get NO grant here —
-- see the module doc comment in `lakehouse-api/src/policy.rs` for why they
-- stay reachable only via Platform Admin's `*:*`. In short: digital-employee
-- lifecycle control (a kill switch on an identity, not a single decision)
-- and storage restore (destructive/irreversible) both lack a seeded role
-- that plausibly owns them today — Approver's `agent:approve` is a
-- narrow per-action decision, not blanket employee administration, and no
-- seeded role represents storage/backup operations at all. Guessing an
-- owner for either would be exactly the guess Task 3.4's identity work
-- declined to make.
--
-- IDEMPOTENT: guarded so re-running this migration (or applying it by hand
-- against a database that already has the grant) never appends the same
-- token twice.
UPDATE role
SET permissions = permissions || ', workload:cancel'
WHERE name = 'Data Engineer'
  AND permissions !~ '(^|,)\s*workload:cancel\s*(,|$)';

UPDATE role
SET permissions = permissions || ', alert:write'
WHERE name = 'Data Engineer'
  AND permissions !~ '(^|,)\s*alert:write\s*(,|$)';
