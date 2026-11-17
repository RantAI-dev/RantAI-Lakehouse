-- PR #106 review R1 SHOULD-FIX DATA-14 3 / D2: signed decision 3
--
-- Why: Decision 3 of DATA-14 specifies that a passing run marks its open incident
-- stale rather than auto-resolving it. Only a person resolves it.
-- We widen the status and resolved check constraints to allow 'stale' without
-- requiring a resolution, and update the partial unique index to cover both
-- 'open' and 'stale'.

ALTER TABLE quality_incident DROP CONSTRAINT quality_incident_status_check;
ALTER TABLE quality_incident ADD CONSTRAINT quality_incident_status_check
    CHECK (status IN ('open', 'stale', 'resolved'));

ALTER TABLE quality_incident DROP CONSTRAINT quality_incident_resolved_check;
ALTER TABLE quality_incident ADD CONSTRAINT quality_incident_resolved_check
    CHECK ((status IN ('open', 'stale') AND resolved_at IS NULL)
        OR (status = 'resolved' AND resolved_at IS NOT NULL));

DROP INDEX IF EXISTS quality_incident_one_open;
CREATE UNIQUE INDEX quality_incident_one_open
    ON quality_incident (asset, (COALESCE(rule_id, kind)))
    WHERE status IN ('open', 'stale');
