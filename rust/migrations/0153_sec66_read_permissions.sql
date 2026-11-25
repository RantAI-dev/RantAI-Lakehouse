-- SEC-66: dedicated read permissions for Gold exports and agent runs.
--
-- WHY: before SEC-66, `GET /api/gold/*` and `GET /api/agents/runs*` were
-- `RequiresAuth`, so any signed-in person — a Dashboard Viewer included —
-- could open `/gold-exports` and `/agents/runs/[runId]` by address. Those
-- reads now take `gold:read` and `agent:read`. Following the upgrade rule
-- ADM-13 set in `0133_pages_per_role.sql` (D2/D4), every built-in role that
-- could reach the pages keeps its reach: both grants are appended to every
-- role except the two people whose remit is dashboards and the assistant
-- (Dashboard Viewer, Assistant only). Platform Admin's `*:*` already
-- satisfies both and is left untouched; `gold:export` holders keep their
-- read through `POLICY_TABLE`'s accepted-grant list.
--
-- THE SCHEDULER: `gold_export.py` calls `GET /api/gold/publications` with
-- the same `GOLD_EXPORT_RUN_TOKEN` it uses for the export. Its
-- `gold-export-scheduler` identity was deliberately unscoped because the
-- write route checks no permission; the GET now does, so on upgrade that
-- ONE named identity gets `gold:read` and never `gold:export` (which still
-- guards `PUT /api/gold/export/{mart}/publication`). Fresh installs get the
-- same scope from `lib.rs::bootstrap_gold_export_service`.
--
-- IDEMPOTENT: each UPDATE guards on the token's absence (and excludes
-- wildcard grants already covering it), so re-running the file appends
-- nothing twice and an operator's hand-edited permissions survive.

UPDATE role
SET permissions = CASE
    WHEN trim(permissions) = '' THEN 'gold:read'
    ELSE permissions || ',gold:read'
END
WHERE name NOT IN ('Dashboard Viewer', 'Assistant only')
  AND permissions != '*:*'
  AND permissions !~ '(^|,)\s*\*:\*\s*(,|$)'
  AND permissions !~ '(^|,)\s*gold:\*\s*(,|$)'
  AND permissions !~ '(^|,)\s*gold:read\s*(,|$)';

UPDATE role
SET permissions = CASE
    WHEN trim(permissions) = '' THEN 'agent:read'
    ELSE permissions || ',agent:read'
END
WHERE name NOT IN ('Dashboard Viewer', 'Assistant only')
  AND permissions != '*:*'
  AND permissions !~ '(^|,)\s*\*:\*\s*(,|$)'
  AND permissions !~ '(^|,)\s*agent:\*\s*(,|$)'
  AND permissions !~ '(^|,)\s*agent:read\s*(,|$)';

-- `SEC-66`: the scheduler's token clears `auth_gate` only as a real
-- principal; without this scope its nightly `GET /api/gold/publications`
-- would be a 403 at the gate even though `check_export_token` would accept
-- the token. `&&` appends to the array only when the scope is absent.
UPDATE service_identity
SET scopes = scopes || ARRAY['gold:read']
WHERE name = 'gold-export-scheduler'
  AND NOT ('gold:read' = ANY(scopes));
