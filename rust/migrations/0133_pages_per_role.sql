-- ADM-13 review round 1, B1, S1
-- ADM-13: each role sees only the pages it may use (`docs/core/specs/adm-13.md`,
-- `docs/core/features/pages-per-role.md`, plan `docs/superpowers/plans/2026-11-09-adm-13-pages-per-role.md`).
--
-- WHY:
-- Every signed-in person previously had access to the assistant (`/api/ai/*`)
-- and operational lists (System status `/api/ops/*`, Alerts `/api/alerts`,
-- Data Quality / Audit Log `/api/governance/*`), meaning a "dashboards only"
-- or "assistant only" role could not exist.
--
-- This migration (D2, D3):
-- 1. Appends `,assistant:use,ops:read,alert:read,governance:read` to every role
--    that holds none of them and is neither `*:*` nor named `Dashboard Viewer`.
--    Existing custom roles and built-in roles (Analyst, Data Engineer, Data
--    Scientist, Approver, Governance Admin) retain all their previous capabilities
--    without losing page access.
-- 2. Inserts the built-in `Assistant only` role (`assistant:ask,dashboard:read`)
--    with UUID `22222222-2222-4222-8222-000000000008` (ON CONFLICT DO NOTHING).
-- 3. Leaves `Dashboard Viewer` at `dashboard:read`, removing assistant access
--    (D2 on the feature page).
--
-- IDEMPOTENT:
-- The UPDATE guards on the absence of all four permissions, excludes `*:*`,
-- `Dashboard Viewer`, and `Assistant only`, so re-running this migration adds
-- nothing twice.

UPDATE role
SET permissions = CASE
    WHEN trim(permissions) = '' THEN 'assistant:use,ops:read,alert:read,governance:read'
    ELSE permissions || ',assistant:use,ops:read,alert:read,governance:read'
END
WHERE name NOT IN ('Dashboard Viewer', 'Assistant only')
  AND permissions != '*:*'
  AND permissions !~ '(^|,)\s*\*:\*\s*(,|$)'
  AND permissions !~ '(^|,)\s*assistant:use\s*(,|$)'
  AND permissions !~ '(^|,)\s*ops:read\s*(,|$)'
  AND permissions !~ '(^|,)\s*alert:read\s*(,|$)'
  AND permissions !~ '(^|,)\s*governance:read\s*(,|$)';

INSERT INTO role (id, name, permissions, description)
VALUES (
    '22222222-2222-4222-8222-000000000008',
    'Assistant only',
    'assistant:ask,dashboard:read',
    'Ask questions and read published dashboards; cannot make changes.'
)
ON CONFLICT (name) DO NOTHING;
