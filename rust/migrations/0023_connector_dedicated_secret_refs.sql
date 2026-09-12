-- Move the seeded connectors off the API's OWN secrets and onto
-- connector-dedicated ones.
--
-- 0022 seeded `conn-pg-lakehouse` with `env:POSTGRES_PASSWORD` (the console's
-- own database password) and `conn-s3-warehouse` with
-- `env:RUSTFS_ACCESS_KEY`/`env:RUSTFS_SECRET_KEY` (the object store's root
-- keys), and `AppState::CONNECTOR_ALLOWED_SECRET_REFS` allowlisted exactly
-- those. That made the allowlist much weaker than it read: `POST
-- /api/connectors` accepts a caller-chosen `host`, and `connector_probe`'s
-- SSRF guard blocks only INTERNAL ranges — exfiltration wants an external
-- host, which is precisely what it allows. A `connector:manage` principal
-- could therefore create a connector naming `env:POSTGRES_PASSWORD`, point it
-- at infrastructure they control, and have the API authenticate to it with
-- the real password.
--
-- The refs below are dedicated to connectors. A deployment MAY set them equal
-- to the real credentials — see `.env.example`, where they default to exactly
-- that for the local stack — but that is now an explicit, visible choice in
-- the environment rather than an implicit consequence of the allowlist. The
-- API's own secrets are no longer reachable by name through a connector.
--
-- Paired with `routes::connectors::reject_allowlisted_secret_ref`, which
-- refuses a USER-created connector that names one of these: the allowlist
-- decides which refs may resolve, that check decides who may name them, and
-- only migration-seeded connectors satisfy both.
--
-- Written as a targeted UPDATE keyed on the exact old value, not a blanket
-- rewrite: a deployment that has already re-pointed these connectors by hand
-- keeps its own value rather than having it silently replaced.

UPDATE connector
SET secret_ref = 'env:CONNECTOR_PG_PASSWORD'
WHERE id = 'conn-pg-lakehouse'
  AND secret_ref = 'env:POSTGRES_PASSWORD';

UPDATE connector
SET secret_ref = 'env:CONNECTOR_S3_ACCESS_KEY'
WHERE id = 'conn-s3-warehouse'
  AND secret_ref = 'env:RUSTFS_ACCESS_KEY';

UPDATE connector
SET secret_ref_secondary = 'env:CONNECTOR_S3_SECRET_KEY'
WHERE id = 'conn-s3-warehouse'
  AND secret_ref_secondary = 'env:RUSTFS_SECRET_KEY';
