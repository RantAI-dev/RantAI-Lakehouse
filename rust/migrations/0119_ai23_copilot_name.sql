-- AI-23: one name for the assistant — "AI Copilot" (product owner,
-- 2026-11-02).
--
-- The reserved employee row `emp-copilot` was seeded by `0025_agent_schedule.sql`
-- as "Copilot (interactive)". Every chat approval copies that row's name into
-- `approval_item.employee_name` (a denormalized label, never re-derived from
-- `agent_employee`), so the old name is shown in Approvals and on the digital
-- employees pages for every approval the chat created. The console meanwhile
-- called the same assistant "AI Copilot", "Copilot" and "the assistant" in
-- different places. The owner decided on one name, "AI Copilot", so the row
-- is renamed here rather than only in the console.
--
-- `agent_employee.name` is UNIQUE (`0017_agents.sql`), so a customer who
-- already named an employee of their own "AI Copilot" would make a plain
-- rename fail and take the API down at start-up. The guard below therefore
-- renames the reserved row ONLY when no other row already holds the new
-- name; in that case the reserved row keeps its old name and the console's
-- header says which name it carries. The update is also idempotent: it only
-- matches the old name, so a re-run changes nothing.
--
-- `approval_item.employee_name` is a label, not evidence, so it is renamed
-- unconditionally: Approvals shows one name for old and new rows. (The
-- label can then name a row that kept its old name in the clash case above;
-- the id, `emp-copilot`, is what identifies the row.)
--
-- `audit_event` is immutable and is NOT touched: it already recorded what
-- happened, under the name that was true at the time.
--
-- Migration number: `0119` is the next free number after `0118`
-- (`0118_sec57_approval_requester.sql` is SEC-57's).
UPDATE agent_employee
SET name = 'AI Copilot'
WHERE id = 'emp-copilot'
  AND name = 'Copilot (interactive)'
  AND NOT EXISTS (SELECT 1 FROM agent_employee WHERE name = 'AI Copilot');

UPDATE approval_item
SET employee_name = 'AI Copilot'
WHERE employee_id = 'emp-copilot'
  AND employee_name = 'Copilot (interactive)';
