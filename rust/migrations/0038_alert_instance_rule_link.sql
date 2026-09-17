-- WS5 (grand plan §7): links a fired alert occurrence back to the rule
-- that fired it, and adds silence/delivery bookkeeping `0011`'s original
-- design didn't need -- that migration's own header states `run_rules`
-- never wrote a row here at all. `rule_id` has no FK to
-- `console.alert_rule` because that table lives in ClickHouse, not
-- Postgres (same cross-database reality `0011`'s header already
-- documents for this table's existence).
--
-- `severity` loses its NOT NULL / CHECK-without-NULL shape from `0011`
-- (`0011_overview_alerts.sql:34,44-45`): a `console.alert_rule` row can
-- now carry no severity (WS5 plan review Y3 -- a rule's severity must never
-- be invented from its `kind`), and a fired instance copies that rule's
-- severity verbatim, including "none."
ALTER TABLE alert_instance
    ADD COLUMN rule_id TEXT,
    ADD COLUMN fired_at TIMESTAMPTZ,
    ADD COLUMN payload JSONB NOT NULL DEFAULT '{}',
    ADD COLUMN delivery_status TEXT,
    ADD COLUMN silenced_until TIMESTAMPTZ;

ALTER TABLE alert_instance ALTER COLUMN severity DROP NOT NULL;
ALTER TABLE alert_instance DROP CONSTRAINT alert_instance_severity_check;
ALTER TABLE alert_instance ADD CONSTRAINT alert_instance_severity_check
    CHECK (severity IS NULL OR severity IN ('critical', 'high', 'medium', 'low', 'info'));

-- The dedup window (insert_from_fired_rule, a later commit) queries "has
-- this rule fired in the last 15 minutes" -- this index is what that
-- query hits.
CREATE INDEX alert_instance_rule_fired_idx ON alert_instance (rule_id, fired_at DESC)
    WHERE rule_id IS NOT NULL;
