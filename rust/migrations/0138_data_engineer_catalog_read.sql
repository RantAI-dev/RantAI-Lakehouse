-- GOV-11 (`DEC-20`, product owner 2026-11-12); QA `LH-DATA-12-07` finding F2.
--
-- WHY: the seeded Data Engineer role holds `catalog:write` since
-- 0002_seed_identity.sql, but not `catalog:read`. So the role that edits
-- catalog entries cannot open the Catalog list or a table's page, which is
-- exactly the shape 0031_governance_admin_catalog_read.sql fixed for
-- Governance Admin. Folding `catalog:read` onto the same role keeps one
-- built-in role doing the job it is named for rather than adding a second.
--
-- IDEMPOTENT: keyed on the token's absence (0020_extend_role_grants.sql's
-- idiom, as 0031 and 0033 use), so re-running this migration adds nothing a
-- second time; and it touches only the role NAMED `Data Engineer` that does
-- not already hold `catalog:read` or a wildcard that covers it, so a renamed
-- or hand-authored custom role is left alone. The `CASE` covers a role whose
-- permissions were emptied by hand (0133_pages_per_role.sql's shape), where
-- appending `', catalog:read'` would otherwise leave a leading comma.
UPDATE role
SET permissions = CASE
    WHEN trim(permissions) = '' THEN 'catalog:read'
    ELSE permissions || ', catalog:read'
END
WHERE name = 'Data Engineer'
  AND permissions !~ '(^|,)\s*catalog:read\s*(,|$)'
  AND permissions !~ '(^|,)\s*(\*:\*|catalog:\*)\s*(,|$)';
