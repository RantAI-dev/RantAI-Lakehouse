-- SRC-40 (`TC-1695`): the latest delivery attempt of a "run succeeded" rule,
-- kept on the rule instead of as an incident.
--
-- ── Why a table of its own ────────────────────────────────────────────
--
-- A success alert is delivered ("your run finished") but the console had no
-- place to show that it was sent: the only store of a delivery's outcome is
-- `alert_instance.delivery_status` (`0068`), written by
-- `lakehouse-api/src/routes/load_alerts.rs::persist_instances`, and the success
-- event passes `None` as its source, so nothing was written.
--
-- Writing an `alert_instance` row for a success will not do: that table is the
-- incident list ("Active Incidents"), a row is `open` at birth, and every open
-- row lights the bell, the home page and the system status page
-- (`routes/notifications.rs`). A connector on a 15-minute schedule would add
-- 96 "incidents" a day. The other per-rule table, `alert_rule_state` (`0113`),
-- is the threshold alerts' state machine and its `state` CHECK admits three
-- words; a delivery outcome is not one of them.
--
-- So: one row per rule, holding the rule's **latest** delivery attempt. That is
-- the narrowest record that makes "sent at … / not delivered: …" true for a
-- success rule, and it opens no incident. Older attempts are replaced, and a
-- row whose `at` is newer than the incoming one is never overwritten (the
-- upsert in `lakehouse-store/src/alert_delivery.rs` carries the `WHERE`).
--
-- `rule_id` names a `ClickHouse` row in `console.alert_rule`, so it has no
-- foreign key here; a deleted rule leaves a row nothing reads, and the API
-- removes it when the rule is deleted or when an edit changes what the rule
-- watches (`routes/alerts.rs::forget_state`).
--
-- `delivery_reason` holds one of the fixed sentences of
-- `lakehouse_notify::DeliveryFailure::message`, only on a `not_delivered` row.
-- It is never a webhook's status line or body, an SMTP reply or a driver's
-- error text (principle 4); the second CHECK keeps a reason from sitting on a
-- delivered row.
CREATE TABLE alert_rule_delivery (
    rule_id         TEXT PRIMARY KEY,
    at              TIMESTAMPTZ NOT NULL,
    delivery_status TEXT NOT NULL,
    delivery_reason TEXT,
    CONSTRAINT alert_rule_delivery_status_check
        CHECK (delivery_status IN ('delivered', 'not_delivered')),
    CONSTRAINT alert_rule_delivery_reason_check
        CHECK (delivery_reason IS NULL OR delivery_status = 'not_delivered')
);
