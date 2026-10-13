-- SRC-17 F1 (delivery review 2026-10-12): an alert instance records whether
-- its webhook or e-mail actually arrived.
--
-- Until now the alerts crate discarded the delivery result and an instance
-- was saved either way, so a webhook that answered 500 left an alert that
-- read as sent (feature page `load-failure-alerts`, checklist row 5: "the
-- failed delivery is shown with an honest message and no server text").
--
-- `0038` already added `delivery_status TEXT` and nothing ever wrote it; it
-- is used here rather than adding a second column for the same fact. NULL
-- means "not known": every row written before this migration, the seeded
-- rows, and instances whose delivery was skipped. It is never filled in
-- for them (principle 2: no invented outcome).
--
-- `delivery_reason` holds one of the fixed sentences of
-- `lakehouse_notify::DeliveryFailure::message`, only when the status is
-- `not_delivered`. It is never a webhook's status line or body, an SMTP
-- reply or a driver's error text (principle 4); the CHECK keeps a reason
-- from sitting on a delivered row.
ALTER TABLE alert_instance
    ADD COLUMN delivery_reason TEXT;

ALTER TABLE alert_instance
    ADD CONSTRAINT alert_instance_delivery_status_check
        CHECK (delivery_status IS NULL OR delivery_status IN ('delivered', 'not_delivered')),
    ADD CONSTRAINT alert_instance_delivery_reason_check
        CHECK (delivery_reason IS NULL OR delivery_status = 'not_delivered');
