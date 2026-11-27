-- SEC-76: a digital employee works in one organisation (tenant) and acts
-- as its own principal when it runs on a schedule.
--
-- WHY:
-- Before this, `agent_employee` had no tenant column, so a scheduled run
-- carried no tenant. A tenant-scoped tool refused it on any install with
-- more than one tenant, and every scheduled run of every agent acted as
-- the same `Service(nil)` principal: a row a run wrote could not be told
-- apart from another agent's row.
--
-- `tenant_id` names the organisation the agent works in. NULL means the
-- agent has no organisation yet, and a run of it is refused (409) before
-- any tool or model call. Set it with
-- `PUT /api/agents/employees/{id}/tenant`.
--
-- `ON DELETE SET NULL` is the repository's rule for a tenant column (see
-- `0099`, `0120`, `0144`). Deleting an organisation does not block on its
-- agents and does not leave an agent pointing at a tenant that is gone:
-- the agent becomes an agent with no organisation, and its runs are
-- refused (409) until an operator sets a new one.
--
-- `principal_id` is the identity a scheduled run acts as. It is a UUID
-- and not the agent's `id`, because `agent_employee.id` is a TEXT slug and
-- `PrincipalId` wraps a UUID. The column gets a value for every existing
-- row from the default, and every row inserted later takes one too.
-- The UNIQUE constraint keeps two agents from sharing one identity.
--
-- The backfill sets `tenant_id` only when the install has exactly one
-- tenant, because then there is no other organisation the agent could
-- belong to. With zero or several tenants, every agent stays NULL, and an
-- operator sets each one. The statement only fills NULLs, so running the
-- migration again changes nothing an operator has already set.
--
-- Table grants: `agent_employee` is not append-only, so the role's
-- table-level grant covers the two new columns and `db_roles.rs` needs no
-- entry.

ALTER TABLE agent_employee
    ADD COLUMN tenant_id UUID NULL REFERENCES tenant (id) ON DELETE SET NULL;

ALTER TABLE agent_employee
    ADD COLUMN principal_id UUID NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE agent_employee
    ADD CONSTRAINT agent_employee_principal_id_unique UNIQUE (principal_id);

UPDATE agent_employee
SET tenant_id = (SELECT id FROM tenant)
WHERE tenant_id IS NULL
  AND (SELECT count(*) FROM tenant) = 1;
