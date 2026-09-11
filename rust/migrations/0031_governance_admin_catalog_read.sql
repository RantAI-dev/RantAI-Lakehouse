-- WS2 §4: a Governance Admin may write a table maintenance policy
-- (`governance:write`, granted in 0030_table_maintenance_policy.sql), but
-- every `/api/lakehouse/*` GET route — including the table-detail read the
-- maintenance policy applies to — requires `catalog:read`, which the seeded
-- Governance Admin role does not hold. Without this grant, the role that
-- writes a maintenance policy cannot read back the table it configured.
--
-- Following 0020_extend_role_grants.sql's exact idiom: idempotent and
-- regex-guarded, so re-applying this migration (or a hand-run copy of it)
-- never appends the token twice. The grant is folded onto the existing
-- Governance Admin role rather than inventing a new one.
UPDATE role
SET permissions = permissions || ', catalog:read'
WHERE name = 'Governance Admin'
  AND permissions !~ '(^|,)\s*catalog:read\s*(,|$)';
