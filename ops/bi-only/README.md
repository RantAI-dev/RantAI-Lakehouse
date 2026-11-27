# BI and assistant only installation

Two standalone Compose files for a deployment that reads the customer's own
database **in place**. Nothing is copied into a warehouse: the installation
is the application database, one query engine, `lakehouse-api`, the console
and the PDF renderer. This is `BI-44` slice 1 — the mode switch, the engine
choice, health/readiness reporting and the deployment files. Registering
external catalogs and running dashboards, SQL sources and the assistant
over them lands in later slices.

## Choose exactly one engine

The product owner decided (2026-11-11) that ClickHouse and Trino never run
side by side for this installation, so they do not compete for the host's
memory. Each file starts only one engine and the API refuses to start if a
second engine's URL is set. A request to Query Studio naming the engine
that was not started gets a 400 that names the configured engine; no dial
is attempted.

| File | Engine | Source coverage (see `docs/core/specs/bi-44.md`) |
| --- | --- | --- |
| `docker-compose.bi-only-clickhouse.yml` | ClickHouse external engines | PostgreSQL, MySQL, remote ClickHouse |
| `docker-compose.bi-only-trino.yml` | Trino federated catalogs | PostgreSQL, MySQL, MariaDB, SQL Server, Oracle, ClickHouse, Snowflake, BigQuery, Redshift |

## Install with `lakehousectl`

The installer takes the same choice as the two files above:

```bash
lakehousectl install --mode bi-only --engine clickhouse
lakehousectl install --mode bi-only --engine trino
```

`--mode` defaults to `full` and `--engine` to `clickhouse`, so an install
with no flags is byte-for-byte the earlier full install. In `bi-only` the
installer writes `LAKEHOUSE_INSTALL_MODE=bi-only`,
`LAKEHOUSE_QUERY_ENGINE=<clickhouse|trino>` and exactly one engine URL into
the install's `.env` (still mode 0600, never overwriting an operator's
other values): a federated install sets `TRINO_URL=http://trino:8080` and
comments out `CH_URL` / `CLICKHOUSE_URL`; a `ClickHouse` install comments
out `TRINO_URL`. An unknown mode or engine is refused by the parser, and
`--mode full --engine trino` is refused before any file is written —
`--engine` names the single engine of a `bi-only` installation, not an
optional profile of the full stack.

The installer writes the mode variables; it does not swap the release's
root Compose file. Start the matching standalone file below to deploy only
the seven named services. Once the API is up, the console's Admin >
Settings > Deployment page shows the mode and engine, hides the
warehouse-only sidebar pages and offers only that engine in Query Studio
(for a principal allowed to read the deployment route).

## Start

1. Build the two images this repository owns, from the repository root:

   ```bash
   sh ops/init-env.sh                       # writes a local .env; refuses to overwrite
   docker compose build lakehouse-api console
   ```

   Both standalone files reference the same image variables
   (`LAKEHOUSE_API_IMAGE`, `CONSOLE_IMAGE`) and the same pinned third-party
   image digests as the root file. They do not contain `build:` blocks, so
   build first (or point the variables at published images).

2. Start one variant, never both on the same project name:

   ```bash
   docker compose -f ops/bi-only/docker-compose.bi-only-clickhouse.yml \
     --env-file .env -p lakehouse-bi up -d
   ```

   or:

   ```bash
   docker compose -f ops/bi-only/docker-compose.bi-only-trino.yml \
     --env-file .env -p lakehouse-bi up -d
   ```

   `POSTGRES_PASSWORD`, `LAKEHOUSE_DB_APP_PASSWORD`,
   `LAKEHOUSE_DB_SOURCE_READER_PASSWORD` and `CONNECTOR_SECRETS_KEY` are
   must-set (no default credential is baked
   into either file). `AUTH_BOOTSTRAP_EMAIL` and
   `AUTH_BOOTSTRAP_PASSWORD` are needed to sign in for the first time.

3. Check the installation:

   ```bash
   curl -fsS http://localhost:8080/health/ready
   curl -fsS -H 'Cookie: <your session>' http://localhost:8080/api/settings/deployment
   ```

   The deployment answer carries `"installMode": "bi-only"` and
   `"queryEngine": "<the chosen engine>"`. The Services view reports the
   other engine and every warehouse-only service as `unknown` /
   `checked: false`, never as an invented outage.

## External catalogs

For the federated variant, put operator-authored `.properties` files in
`ops/bi-only/catalog/` (one per source) and set `BI_TRINO_CATALOG_DIR` to
another directory if you keep them elsewhere. The directory is mounted
read-only at `/etc/trino/catalog`; no warehouse catalog or object-store
credential is supplied by this file. Catalog registration, credential-at-
rest and schema introspection are slice 2.

The hot-analytical-engine variant uses that engine's own external table
support (PostgreSQL, MySQL, remote ClickHouse). It does not publish a
second coordinator, and the engine is reachable on its usual host ports
only when the operator leaves the defaults in place.

## Limits

- The warehouse-only services (object store, catalog, authorization
  service, orchestration, change capture) are deliberately absent. Routes
  that need them degrade honestly rather than dialing a service that was
  never deployed.
- The engine's catalog properties are the operator's responsibility; this
  repository commits no credentials.
- Both files are deployment templates. Verify with
  `python3 -m unittest discover -s ops/bi-only -v`; a real `docker compose
  up` from a clean project name is the proof that the daemon accepts the
  shape.
