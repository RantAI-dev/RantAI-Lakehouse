-- SEC-78: the seeded `conn-pg-lakehouse` row named the
-- OWNER `lakehouse` as a LITERAL in two places: in `host`
-- (`0022_prune_connector_seed.sql` seeds
-- `lakehouse@postgres:5432/lakehouse`, the `<user>@<host>:<port>/<db>`
-- shape `connector_probe::probe` parses) and in `dial->>'user'`
-- (`0034_seed_connector_ingest_spec.sql`). The password this connector
-- resolves — `env:CONNECTOR_PG_PASSWORD`, which compose fills with the
-- runtime role's password since SEC-78 — belongs to `lakehouse_app`, so
-- a literal naming the owner asks Postgres for the owner's password
-- with the role's password: the dial fails, and if it ever succeeded it
-- would run as the owner, the very thing SEC-78 removes. This
-- migration repoints both literals at `lakehouse_app`, so the seeded
-- connector dials the console's own database as the role that owns no
-- table. The `secret_ref` needs no change: it has been
-- `env:CONNECTOR_PG_PASSWORD` since
-- `0023_connector_dedicated_secret_refs.sql`.
--
-- Each UPDATE is guarded by its old value, so it is idempotent: a rerun
-- against an already-migrated database — or one an operator already
-- fixed by hand — matches no row and changes nothing.
UPDATE connector
SET host = 'lakehouse_app@postgres:5432/lakehouse'
WHERE id = 'conn-pg-lakehouse'
  AND host = 'lakehouse@postgres:5432/lakehouse';

-- `jsonb_set`, not a whole-`dial` literal: only the `user` key changes,
-- so a dial whose other keys a later migration or an operator changed
-- keeps them.
UPDATE connector
SET dial = jsonb_set(dial, '{user}', to_jsonb('lakehouse_app'::text))
WHERE id = 'conn-pg-lakehouse'
  AND dial->>'user' = 'lakehouse';
