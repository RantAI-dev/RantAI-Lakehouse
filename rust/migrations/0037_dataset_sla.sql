-- WS5 (grand plan §7): per-table freshness expectations, read by the
-- Overview "Freshness" strip (actual lastUpdatedMs, from WS2's Iceberg
-- REST table stats, vs this expectation) and by lakehouse-alerts's
-- Freshness rule kind. Postgres, not ClickHouse: this is
-- operator-authored config, written rarely and read on every Overview
-- load — the same OLTP shape `0011`'s header already argues for
-- alert_instance, not a new argument.
CREATE TABLE dataset_sla (
    table_name TEXT PRIMARY KEY,
    -- WS5 plan review U12: a zero or negative expectation is not a
    -- legitimate SLA (an "expected within 0 minutes" table is always
    -- late by construction) and is rejected at the database, not just
    -- at the route — the route-level check (Task E1 Step 4) is defense
    -- in depth, this CHECK is the actual guarantee.
    expected_interval_minutes INT NOT NULL CHECK (expected_interval_minutes > 0),
    owner TEXT
);
