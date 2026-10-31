-- DATA-5 (docs/core/specs/data-5.md, ADR 0016): the daily read counter for
-- outside tools.
--
-- ADR 0016 decision 5: a successful table load through the API's door
-- (`/api/iceberg/v1/...`) adds one read to a daily counter for that tool
-- and that table, and a load whose count cannot be written is REFUSED
-- (503) rather than served uncounted -- an uncounted read would make the
-- count a lie (principle 5). This table is that counter; the store half
-- is `lakehouse_store::outside_reads`, and the route that writes it is
-- `lakehouse_api::routes::iceberg_door`.
--
-- # Why the default schema and not a `console` one
--
-- The plan's sketch of this table is written `console.outside_table_read_day`,
-- but `console` is the CLICKHOUSE database this stack's other
-- `console.*` tables live in (`console.alert_rule`,
-- `console.gold_export_run`), not a Postgres schema: no migration in this
-- directory creates one, and every Postgres table here is created
-- unqualified in the default schema. The count lives in Postgres on
-- purpose -- it is written on the request path and must fail closed
-- (a failed write refuses the load), which is a transaction this store
-- already owns, not something an analytical store should answer. The
-- table's name is the plan's; only the (nonexistent) schema prefix is
-- dropped.
--
-- # Why `identity_id` has no foreign key
--
-- The counter stores the tool's NAME at the time of the read
-- (`identity_name`), and ADR 0016 decision 5 requires a deleted tool's
-- past reads to stay readable. A foreign key to `service_identity` would
-- delete those rows with the identity; the id is kept as a plain UUID so
-- the history outlives the tool.
--
-- # Why one row per day
--
-- The page shows the last 7 days and the history is kept for 90, so the
-- counter is a daily row rather than a running total: a daily row can be
-- pruned on its own (rows older than 90 days are deleted) without
-- rewriting a total that has to survive the pruning. `reads` is the
-- number of loads counted that day; the store's reader SUMS it over the
-- window it is asked about.
--
-- `last_read_at` is the newest read counted in that row, so the page can
-- say "last read" without a second query.
CREATE TABLE IF NOT EXISTS outside_table_read_day (
    day           DATE        NOT NULL,
    identity_id   UUID        NOT NULL,
    identity_name TEXT        NOT NULL,
    namespace     TEXT        NOT NULL,
    table_name    TEXT        NOT NULL,
    reads         BIGINT      NOT NULL,
    last_read_at  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (day, identity_id, namespace, table_name)
);

-- The one read this table is asked for: "who read this table since this
-- day, and how often". The index leads with the table so that read never
-- scans the whole history; the leading `day` of the primary key serves
-- the 90-day prune.
CREATE INDEX IF NOT EXISTS outside_table_read_day_table_idx
    ON outside_table_read_day (namespace, table_name, day);
