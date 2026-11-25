-- SEC-68 (Option B, PO-approved): an explicit role grant on every digital
-- employee, so a run with no person behind it is narrowed to real roles
-- instead of reading every table.
--
-- WHY:
-- `GOV-7` narrows table access by the ROLES a principal holds. A scheduled
-- agent run (`x-run-token`) has no roles at all, and a service identity's
-- scopes are not roles, so `for_roles(&[])` answered `TableAccess::All`:
-- an agent could read every table. Worse, a person who may manage agents
-- could arrange for an agent to read a table their own role hides. The fix
-- is a grant stored ON THE AGENT, independent of whoever triggers it.
--
-- `read_roles` is the NAMES of the roles the agent may read as. Empty means
-- NO grant: a run of that agent reads ZERO tables (fail closed), never
-- `All`. Names (not role ids) are stored for the same reason a session
-- carries role names: OIDC-mapped roles are names too, and a role can be
-- recreated with a new id without silently widening an existing grant.
-- Unknown names grant nothing at read time (`table_access::for_roles` D4);
-- the write path (the role-grant endpoint) refuses them up front with 400.
--
-- Default '{}' is deliberate and safe here, unlike a default of 'all':
-- existing employees get NO grant and therefore read no tables until an
-- administrator assigns one. That is the fail-closed direction.
--
-- Numbered 0155, not 0153/0154: those are owned by open PRs #704 and #633
-- and this migration must not collide with either.

ALTER TABLE agent_employee
    ADD COLUMN IF NOT EXISTS read_roles TEXT[] NOT NULL DEFAULT '{}';
