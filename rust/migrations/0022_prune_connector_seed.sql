-- P6: `test_connection` now genuinely dials PostgreSQL and S3-compatible
-- object-storage connectors (see `lakehouse-store::connectors`'s module
-- doc comment and `lakehouse-api`'s `connector_probe` module) instead of
-- fabricating a result from stored `health`. `0014_seed_connectors.sql`'s
-- 28-row fixture (Kafka, MQTT, MongoDB, Oracle, SAP/ERP, SFTP, Google
-- Sheets, a web crawler, a credit bureau, ...) was screenshot fixture, not
-- a claim this build could ever back up — pressing "Test" on any of those
-- 28 could only ever be a hardcoded number. Rather than edit
-- `0014_seed_connectors.sql` in place (already applied against any
-- database that ran migrations before this change — editing an applied
-- migration file is exactly what `sqlx::migrate!`'s checksum tracking
-- exists to catch and refuse), this migration removes those 28 fixture
-- rows by their fixed ids (never touching a connector a real deployment
-- created through the API afterward, since those don't share this id
-- namespace) and seeds exactly two rows this build can actually dial
-- against the compose stack:
--
--   * `conn-pg-lakehouse` — the same Postgres this service's own
--     `DATABASE_URL` points at. `host` encodes `<user>@<host>:<port>/<db>`
--     (a shape `connector_probe::probe` parses; NOT itself the DSN — no
--     password) and `secret_ref` is `env:POSTGRES_PASSWORD`, which
--     `docker-compose.yml`'s `lakehouse-api` service now passes through
--     into the container's environment for `EnvSecretResolver` to resolve.
--   * `conn-s3-warehouse` — the RustFS bucket `docker-compose.yml`'s
--     bootstrap job creates. `host` encodes `<endpoint>|<bucket>` (again,
--     parsed by `connector_probe::probe`, not a DSN) and `secret_ref`/
--     `secret_ref_secondary` are `env:RUSTFS_ACCESS_KEY`/
--     `env:RUSTFS_SECRET_KEY`, likewise now passed into the container.
--
-- Both are real "press Test, get a real result" connectors in the compose
-- stack a developer actually runs. Neither `host` value is a credential by
-- itself (same posture `0013_connectors.sql` already documents for every
-- other connector's `host`), and no `secret_ref`/`secret_ref_secondary`
-- value here is a credential value — only reference names, resolved by
-- `EnvSecretResolver` (ADR 0002) at test time, never stored.
DELETE FROM connector WHERE id IN (
    'conn-pg-oms', 'conn-mysql-pos', 'conn-mongo-catalog', 'conn-api-marketplace',
    'conn-crm-cloud', 'conn-kafka-orders', 'conn-kafka-clickstream', 'conn-kafka-fleet',
    'conn-mqtt-warehouse', 'conn-s3-landing', 'conn-sftp-bank', 'conn-gsheets-ops',
    'conn-erp-finance', 'conn-oracle-gl', 'conn-payment-gateway', 'conn-ads-platform',
    'conn-web-analytics', 'conn-esp-email', 'conn-hris', 'conn-fx-rates', 'conn-weather',
    'conn-market-data', 'conn-price-crawler', 'conn-social-listening', 'conn-credit-bureau',
    'conn-iceberg-catalog', 'conn-clickhouse-serving', 'conn-partner-share'
);

INSERT INTO connector (id, name, type, direction, health, environment, tenant, host, secret_ref, secret_ref_secondary, residency, last_test_at, last_activity_at, capabilities, owner, created_at)
VALUES
    ('conn-pg-lakehouse', 'Lakehouse OLTP (Postgres)', 'PostgreSQL', 'bidirectional', 'healthy', 'production', 'Meridian Group', 'lakehouse@postgres:5432/lakehouse', 'env:POSTGRES_PASSWORD', NULL, 'in-region', now() - interval '30 minutes', now() - interval '2 minutes', ARRAY['schema discovery'], 'Data Platform', now()),
    ('conn-s3-warehouse', 'Lakehouse warehouse (RustFS S3)', 'Object storage', 'sink', 'healthy', 'production', 'Meridian Group', 'http://rustfs:9000|lakehouse-warehouse', 'env:RUSTFS_ACCESS_KEY', 'env:RUSTFS_SECRET_KEY', 'in-region', now() - interval '30 minutes', now() - interval '5 minutes', ARRAY['list', 'read', 'parquet'], 'Data Platform', now())
ON CONFLICT DO NOTHING;
