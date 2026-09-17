-- WS7 plan: access requests reuse `approval_item` (grand plan §9). Two
-- additions this migration owns:
--
-- 1. `approval_item.kind` — every row inserted before this migration is
--    a WriteHigh tool-call approval (routes/ai/gate.rs's
--    create_write_high_approval); this migration backfills those to
--    'tool_call' so the CHECK below never rejects existing data, and every
--    NEW access request row is inserted with kind = 'access'.
-- 2. `approval_item.requested_by_user_id` — a tool-call approval has no
--    single human requester (the DIGITAL EMPLOYEE acted; a human only
--    decides it), but an access request always has a human requester —
--    needed so `decide_access_request` (WS7 item E3) can refuse a
--    principal approving their own request.
-- 3. `access_grant` — what an APPROVED access request actually grants:
--    one additional permission token, bounded by an expiry the requester
--    named, revocable by an admin. Not a second, parallel role-grant
--    mechanism — see `governance::PermissionSet`'s doc comment; this
--    table only ever ADDS to `role.permissions`'s existing merge logic
--    by union at Principal-load time (WS7 item E4).
--
-- Migration number: the WS7 plan text names 0038, written before Phase C's
-- own migrations (0038_alert_instance_rule_link.sql,
-- 0039_agent_metrics_recompute.sql) landed on this branch; `ls
-- rust/migrations/` at authoring time showed 0039 as the highest existing
-- number, so this migration takes 0040 instead — never renumbering an
-- already-applied file.

ALTER TABLE approval_item
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'tool_call',
    ADD COLUMN requested_by_user_id UUID REFERENCES app_user (id) ON DELETE SET NULL;

ALTER TABLE approval_item
    ADD CONSTRAINT approval_item_kind_check CHECK (kind IN ('tool_call', 'access'));

-- `employee_id`/`employee_name` are NOT NULL today (0017_agents.sql) but
-- an access request names no digital employee at all — widen both to
-- nullable, since a real tool-call approval always sets them and an
-- access request never does; the CHECK below enforces "exactly one of
-- (employee_id, requested_by_user_id) is set", matching each kind's real
-- shape rather than leaving both nullable with no cross-column guard.
ALTER TABLE approval_item
    ALTER COLUMN employee_id DROP NOT NULL,
    ALTER COLUMN employee_name DROP NOT NULL;

ALTER TABLE approval_item
    ADD CONSTRAINT approval_item_kind_shape_check CHECK (
        (kind = 'tool_call' AND employee_id IS NOT NULL AND requested_by_user_id IS NULL)
        OR (kind = 'access' AND employee_id IS NULL AND requested_by_user_id IS NOT NULL)
    );

CREATE TABLE access_grant (
    id                  TEXT PRIMARY KEY,
    approval_id         TEXT NOT NULL REFERENCES approval_item (id) ON DELETE CASCADE,
    user_id             UUID NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    permission          TEXT NOT NULL,
    granted_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ NOT NULL,
    revoked_at          TIMESTAMPTZ,
    revoked_by_user_id  UUID REFERENCES app_user (id) ON DELETE SET NULL
);

CREATE INDEX access_grant_user_id_idx ON access_grant (user_id) WHERE revoked_at IS NULL;

-- M4 (judge review, WS7 plan): deciding an access request is a
-- governance-level authority, distinct from `agent:approve` (which
-- authorizes deciding an AGENT TOOL-CALL approval — a different kind of
-- decision over a different kind of risk). Minting `access:approve` here,
-- rather than reusing `agent:approve`, keeps the two decision types
-- independently grantable. Same idempotent, regex-guarded idiom as
-- `0020_extend_role_grants.sql` (never appends the token twice on a
-- re-run):
UPDATE role
SET permissions = permissions || ', access:approve'
WHERE name = 'Governance Admin'
  AND permissions !~ '(^|,)\s*access:approve\s*(,|$)';
