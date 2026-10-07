-- DATA-1 (docs/core/features/gold-publish-per-mart.md): per-mart switch
-- for publishing a Gold (`serving.*`) mart to the `gold` Iceberg
-- namespace (ADR 0010). The route that flips it is
-- `PUT /api/gold/export/{mart}/publication`; the nightly/authored-pipeline
-- scheduler reads the enabled rows through
-- `GET /api/gold/publications` and exports exactly those marts.
--
-- A missing row means "off": publishing stays disabled for every mart
-- until a principal holding `gold:export` switches it on, so this table
-- starts empty and a fresh deployment publishes nothing (the feature's
-- decision table). Switching off only flips `enabled` here -- the
-- existing Iceberg table is never dropped by this feature.
--
-- Keyed by TEXT mart name, not a foreign key: the subject is a
-- ClickHouse `serving.*` MergeTree table (asset id `serving.<mart>`
-- minus the `serving.` prefix), which has no row anywhere in Postgres to
-- reference. One row per mart, no joins, idempotent upserts from the PUT
-- route -- the same shape `0050_pipeline_sla.sql` chose for pipeline
-- settings.
CREATE TABLE gold_publication (
    mart       TEXT PRIMARY KEY,
    enabled    BOOLEAN NOT NULL,
    updated_by UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
