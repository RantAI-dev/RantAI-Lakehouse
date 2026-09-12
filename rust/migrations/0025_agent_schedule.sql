-- Task T3.1 (copilot operations handover, plan section 3.4): make a
-- digital employee something that can actually be RUN — headlessly, on a
-- schedule, or via an interactive "Run now" — rather than just a
-- configuration/history record (see the `0017_agents.sql` header comment:
-- until now this domain had no execution runtime at all).
--
-- WHY these four columns, and why nullable/defaulted the way they are:
--
--   prompt         TEXT, nullable. The instruction a headless run sends to
--                  the copilot. NULL means "this employee is not runnable"
--                  — T3.2's run route must refuse a run with no prompt
--                  rather than send the copilot an empty instruction.
--
--   schedule_cron  TEXT, nullable. NULL means "manual only": no Dagster
--                  schedule exists for this employee. A non-NULL value is
--                  what T3.3's schedule factory reads to build one
--                  `ScheduleDefinition` per employee — see
--                  `list_scheduled_employees` in `lakehouse-store::agents`.
--
--   mode           TEXT NOT NULL DEFAULT 'build', CHECK'd to the copilot's
--                  two modes (`ai.rs`'s `SYSTEM_ASK_SUFFIX` /
--                  `SYSTEM_BUILD_SUFFIX`). Defaulting to 'build' matches
--                  what a scheduled/headless run needs to be useful (Ask
--                  mode can never write anything, so a headless Ask-mode
--                  employee could only ever produce read-only steps).
--
--   permissions    TEXT NOT NULL DEFAULT ''. The CEILING on what this
--                  employee's runs may do, in EXACTLY the free-text
--                  `resource:action[, resource:action...]` format
--                  `role.permissions` already uses (see
--                  `0002_seed_identity.sql`'s role seed and
--                  `lakehouse-auth::permissions::PermissionSet::parse`).
--                  The default '' means "authenticated only, no
--                  permissioned tools" — the same fail-closed default
--                  `PermissionSet::parse("")` already produces (an empty
--                  set satisfies nothing). A headless run's synthetic
--                  principal is built from this column, never from the
--                  triggering human's own permissions.
ALTER TABLE agent_employee
    ADD COLUMN prompt         TEXT,
    ADD COLUMN schedule_cron  TEXT,
    ADD COLUMN mode           TEXT NOT NULL DEFAULT 'build',
    ADD COLUMN permissions    TEXT NOT NULL DEFAULT '',
    ADD CONSTRAINT agent_employee_mode_check CHECK (mode IN ('ask', 'build'));

-- The reserved interactive-copilot employee row. Interactive chat
-- (`POST /api/ai/chat`) attributes every approval it creates and every
-- audit event it writes to THIS row's id (`emp-copilot`), the same way a
-- scheduled run attributes its own approvals/audit events to the employee
-- that owns the schedule. Without a reserved row, an interactive chat
-- session would have no `employee_id` to put in `approval_item` /
-- `agent_run`, and those tables' `employee_id` columns are `NOT NULL
-- REFERENCES agent_employee (id)` (`0017_agents.sql`) — there is nowhere
-- else to point.
--
-- `schedule_cron = NULL` and `prompt = NULL`: this row is never run
-- headlessly by T3.2/T3.3 — it exists purely as an attribution target for
-- work a live human is doing in chat right now, so "run this employee on a
-- schedule" makes no sense for it. `permissions = ''` for the same reason:
-- a headless run derived from this row would otherwise be meaningless to
-- scope, since interactive chat already scopes every tool call to the
-- LOGGED-IN USER's own permissions (T0.2), never to this row's.
--
-- DO NOT DELETE THIS ROW. Deleting it would either orphan every
-- interactive-chat `approval_item`/`agent_run` (impossible — both columns
-- are `NOT NULL`) or, in practice, break interactive chat's ability to
-- record an approval/run at all. `ON CONFLICT DO NOTHING` on the fixed id
-- makes inserting it idempotent, matching every other seed migration in
-- this repository.
INSERT INTO agent_employee
    (id, name, purpose, owner, autonomy, status, budget_limit, budget_spent,
     budget_reserved, allowed_tools, data_scope, approval_rate, success_rate,
     recent_runs, prompt, schedule_cron, mode, permissions, created_at)
VALUES
    ('emp-copilot', 'Copilot (interactive)',
     'Attributes the interactive AI copilot''s chat-driven approvals and runs to a stable identity.',
     'Platform', 'L1', 'ready', 0, 0, 0, '{}', '', 0, 0, 0,
     NULL, NULL, 'build', '', now())
ON CONFLICT DO NOTHING;
