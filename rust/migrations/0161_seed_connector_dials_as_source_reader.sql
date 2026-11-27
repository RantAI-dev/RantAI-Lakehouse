-- ops-145 (SEC-84): the seeded `conn-pg-lakehouse` dials the SOURCE
-- READER role, `lakehouse_source_reader`, not the console's role.
--
-- Before this, 0149 pointed the row at `lakehouse_app`, the console's
-- role. The Dagster code location reads its source tables through this
-- connector, and it held the console's password to do so. `lakehouse_app`
-- can INSERT into `public.audit_event`, so pipeline code could forge audit
-- rows. The source reader role has read access to the source schemas only
-- (`rust/crates/lakehouse-api/src/db_roles.rs`, `setup_source_reader_role`),
-- so the connector now dials it. The password this connector resolves,
-- `env:CONNECTOR_PG_PASSWORD`, is the reader's since the same change: the
-- compose file fills it from `LAKEHOUSE_DB_SOURCE_READER_PASSWORD`. The
-- `secret_ref` itself does not change.
--
-- `host` is changed too. The `<user>@<host>:<port>/<db>` shape of `host` is
-- parsed by `connector_probe::parse_postgres_host`, and the user it returns
-- is the login the probe and the deprovision dial use. Left at
-- `lakehouse_app`, those paths would still log in as the console's role.
--
-- Each UPDATE is guarded by its old value, so it is idempotent: a rerun, or
-- a database an operator already repointed by hand, matches no row. A
-- connector an operator created by hand is not in this WHERE and keeps its
-- own dial.
--
-- 0149 is not edited: it ran on every database that has it, and this
-- migration only moves the row from its value after 0149 to the reader.
UPDATE connector
SET host = 'lakehouse_source_reader@postgres:5432/lakehouse'
WHERE id = 'conn-pg-lakehouse'
  AND host = 'lakehouse_app@postgres:5432/lakehouse';

-- `jsonb_set`, not a whole-`dial` literal: only the `user` key changes, so
-- the other keys an operator or a later migration set keep their values.
UPDATE connector
SET dial = jsonb_set(dial, '{user}', to_jsonb('lakehouse_source_reader'::text))
WHERE id = 'conn-pg-lakehouse'
  AND dial->>'user' = 'lakehouse_app';
