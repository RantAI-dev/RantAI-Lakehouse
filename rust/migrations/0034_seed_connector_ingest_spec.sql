-- WS3: 0022_prune_connector_seed.sql already reduced the seed fixture to
-- the two connectors this compose stack can actually dial
-- (conn-pg-lakehouse, conn-s3-warehouse). 0034's job is seeding
-- adapter/ingest_mode/dial/source_objects on those two rows so they carry
-- an ingest spec that lakehouse_store::ingest_spec::Dial::parse actually
-- accepts.
--
-- dial.driver is "postgres", not "postgresql": SqlDriver
-- (lakehouse-store/src/ingest_spec.rs) is
-- #[serde(rename_all = "snake_case")] over Mysql | Postgres | Mssql, so
-- "postgres" is the only value that name would ever serialize to. This
-- migration writes straight to the database, bypassing set_ingest_spec's
-- Dial::parse guard, so getting this value right here matters: a wrong
-- value would sit in the row looking healthy until some later reader
-- (the deprovision path, or the ingest factory) choked on it. See the
-- accompanying store test in
-- rust/crates/lakehouse-store/tests/connector_ingest_spec_seed.rs, which
-- parses this seeded row's dial through Dial::parse so the seed can never
-- drift from the enum again.
--
-- dial.user is a LITERAL username, not "env:CONNECTOR_PG_USER" or any
-- other indirection -- the seeded connector's `host` column already
-- carries this exact username ("lakehouse@postgres:5432/lakehouse"), and
-- a bare username is not credential-shaped.
UPDATE connector
SET adapter = 'sql',
    ingest_mode = 'batch',
    dial = '{"driver":"postgres","host":"postgres","port":5432,"database":"lakehouse","user":"lakehouse","sslMode":"disable"}'::jsonb,
    source_objects = '[{"name":"ingest_demo.orders","incrementalKey":"created_at","target":"orders"}]'::jsonb,
    schedule_cron = NULL
WHERE id = 'conn-pg-lakehouse'
  AND adapter IS NULL;

-- source_objects is seeded EMPTY here -- no tracked file creates a
-- "landing/" prefix or CSV fixture on the warehouse bucket to describe.
-- This row stays adapter='files' with an empty object list until a real
-- object is chosen through the wizard's discover step, or a later gate
-- task creates a real fixture and seeds its own connector's
-- source_objects.
UPDATE connector
SET adapter = 'files',
    ingest_mode = 'batch',
    dial = '{"protocol":"s3","endpoint":"http://rustfs:9000","bucket":"lakehouse-warehouse","format":"csv"}'::jsonb,
    source_objects = '[]'::jsonb,
    schedule_cron = NULL
WHERE id = 'conn-s3-warehouse'
  AND adapter IS NULL;
