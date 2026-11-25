# Known limitations

Moved here from the README on 2026-11-05, unchanged. Some entries predate
later work and have not been re-checked since; the current list of what the
product does not do is [core/PRODUCT.md](core/PRODUCT.md), section 3.

This is a young, honestly-scoped project. Please read this before filing an
issue about any of the following — they're known, not bugs:

- **There is no streaming surface.** There is no stream-processing engine
  (Redpanda, Pulsar, Flink) in this project; Kafka appears only as a
  connector source that the orchestrator reads in batches
  (`dagster/dispar_orchestrate/adapters/kafka.py`). The console previously had a mocked
  `streaming` domain fabricating lag/throughput/checkpoint numbers; it has
  been removed rather than kept as a mock. **CDC (Debezium, P5) is not a
  streaming engine** and is not relabeled as one — it's a
  change-data-capture pipe from Postgres into Bronze Iceberg, surfaced
  instead under Governance → "Ingestion (CDC)".
- **Uploading a file: every column is loaded as text.**
  The console accepts a CSV or TSV (UTF-8 or UTF-16; comma, semicolon, tab or
  pipe), an `.xls`/`.xlsx` workbook, or a Parquet file, converted to text
  columns on the way in (`routes/uploads.rs`, ADR 0014), of up to 50 MB and
  2,000,000 data rows. It refuses any other format (JSON among them), or
  anything over a limit, with the reason, never cutting a file short. The table it becomes stays in the raw layer: the pipeline builder
  cannot read raw tables yet and a dashboard reads Gold tables only. The
  original file is kept until someone deletes the upload, and uploading needs
  the object-storage settings and the orchestrator (see `docs/OPERATIONS.md`,
  "Uploaded files"). It has not been run end to end on a deployed stack.
- **There is no knowledge search.** The console's knowledge pages and the
  mocked search behind them were removed; there is no vector store or
  embeddings API. The API's knowledge source and vector-job records remain
  (Postgres, `lakehouse-store::knowledge`) with no search route and no page.
- **Connector "Test connection" dials PostgreSQL, MySQL/MariaDB, SQL Server,
  S3-compatible object storage, REST APIs and the business-app vendors
  (Salesforce, ServiceNow, Google Analytics 4, HubSpot, Zendesk, Confluence,
  Dynamics 365, SharePoint; a fixed vendor host, never one typed by the user).** `POST /api/connectors/{id}/test`
  (`lakehouse-api`'s `connector_probe` module) opens a real, bounded
  connection to those and measures real latency, resolving the connector's
  `secretRef` through the allowlisted resolver (ADR 0002). Oracle, MongoDB,
  Kafka and SFTP connectors are read only by the orchestrator, so the
  console reports them as not testable from here; any other type returns
  `{ supported: false }` with a message saying so — never a fabricated
  latency or success. The seed (`0022_prune_connector_seed.sql`) was
  shrunk to match: two connectors, `conn-pg-lakehouse` and
  `conn-s3-warehouse`, pointing at the compose stack's own Postgres and
  RustFS.
- **ClickHouse cannot `CREATE TABLE` through the catalog, on 26.3 or
  26.8.** `CREATE TABLE` inside a `DataLakeCatalog` database never reaches
  Lakekeeper — it falls back to `MergeTree` and fails with `Code: 79` on
  both versions. `INSERT` is the part that changed: on 26.3 a partitioned
  catalog-registered table **segfaulted the server** and an unpartitioned
  one failed with `Code: 1000`; on the pinned 26.8 both now succeed. Gold
  export still goes through Rust (ADR 0010), because `CREATE TABLE` —
  the half that has to work for ClickHouse to own a table's lifecycle —
  still does not. See `docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md` for the
  re-measurement and `docs/plans/G1-RESULT.md` for the original 26.3 run.
- **`OPTIMIZE` runs but does not bin-pack, and `expire_snapshots` is now
  refused.** On the pinned 26.8, `remove_orphan_files` works (it did not
  exist on 26.3) and `OPTIMIZE` returns OK but leaves the file count
  unchanged — 7 files in, 7 files out — so it is not compaction. Moving
  the other way, `expire_snapshots` worked on 26.3 and is **unsupported
  for transactional catalogs** on 26.8. Small-file compaction on Bronze
  therefore still runs out-of-band via a Trino-as-cron container (`trino`
  compose profile, ADR 0009); a deployment that never enables that profile
  accumulates small Bronze files unbounded.
- **Bronze snapshot history has a default 7-day retention floor (OPS-1), but requires the `trino` profile to actually run expiry.**
  Tables without an explicit governance policy or with `snapshotsToKeep: None`
  expire snapshots older than 7 days when `bronze_maintenance_job` runs.
  Because ClickHouse 26.8 refuses `expire_snapshots` for catalog-backed
  tables, expiry is dispatched via Trino (`ALTER TABLE iceberg.<ns>."<name>" EXECUTE expire_snapshots(retention_threshold => '7d')`).
  Deployments that do not enable the `trino` compose profile skip expiry
  with a logged warning, leaving snapshot growth unbounded until Trino is
  started or Lakekeeper gains native retention support. See
  `docs/adr/0004-bronze-naming-partitioning-retention.md`.
- **A bare `count()` overcounts on CDC-fed Bronze tables — measured on
  26.3, not re-verified on the pinned 26.8.** Reproducing it needs a
  CDC-fed table carrying merge-on-read equality deletes, i.e. the full
  Debezium path rather than a hand-built table, so the 26.8 re-measurement
  did not cover it. Treat it as open, not fixed: the lint that guards it
  (R11) is version-independent and stays either way. On ClickHouse
  26.3, `SELECT count()` / `count(*)` / `count(<col>)` against a Bronze
  Iceberg table with merge-on-read equality deletes (i.e. any
  Debezium-fed table) takes a metadata-only fast path that does not
  subtract deleted rows — measured returning 6 where 4 was correct. Any
  `WHERE` or `GROUP BY` forces the correct row-scan path. This is a
  silent wrong answer, not an error; no code added in this repository
  emits a bare `count()` against a Bronze Iceberg table (R11). See
  `docs/plans/P5-RESULT.md`.
- **Lakekeeper authorization is enabled by default — `authz-backend:
  "openfga"`, not `"allow-all"`.** `docker compose up` now brings up
  OpenFGA and a mock OIDC issuer (`ops/oidc-mock`) alongside Lakekeeper,
  and every writer this repo's own tests exercise (the Rust `g1-lakekeeper`
  test path, the dlt pipeline) authenticates as a
  granted principal — see `docs/adr/0011-lakekeeper-authorization.md` for
  the model, the grants, and the default-posture decision. **What is still
  open:** ClickHouse's `CREATE TABLE` through the catalog does not work on
  this ClickHouse version (`INSERT` does on 26.8; a ClickHouse defect, independent
  of authorization — see above and `docs/plans/G1-RESULT.md`), so R1's
  original framing ("ClickHouse catalog-registered
  writes fail against Lakekeeper's authz on metadata updates") is still
  untestable. The `trino` compose profile (ADR 0009's small-file-
  compaction escape hatch) is granted its own principal too — `EXECUTE
  optimize` measured working under enforcement (3 data files -> 1 on a
  live Bronze table; see ADR 0011 and `docs/plans/G3-RESULT.md`).
- **Login throttling covers password login only.** Failed `POST
  /api/auth/login` attempts are throttled per email; after a configurable
  number of failures within a window, the email is locked out for a
  configurable duration. OIDC/SSO login is not throttled. See
  `LOGIN_MAX_FAILURES`, `LOGIN_FAILURE_WINDOW_SECS`, and
  `LOGIN_LOCKOUT_SECS` in [CONFIGURATION.md](CONFIGURATION.md).
- **SSO is gated by a build-time flag, not a runtime one.** The frontend
  can't read the Rust process's environment variables directly, so whether
  the SSO login UI shows up is controlled by `NEXT_PUBLIC_SSO_ENABLED` at
  *build* time — it cannot react to whether `OIDC_ISSUER`/`OIDC_CLIENT_ID`
  are actually set on the backend at runtime. `GET /api/auth/providers`
  exists and the admin single sign-on page uses it to show what the backend
  has configured; the login page's button still follows the build-time
  flag. Single sign-on has not been tested against a real identity provider
  (backlog `SEC-3`).
- **The service does not refuse to boot when Postgres is down.**
  `lakehouse-store::connect_lazy` never does network I/O at startup;
  Postgres connectivity is only discovered lazily, at first use. Dependent
  (Phase 2) routes answer `500` with a fixed "database error" (a
  `DATABASE_URL` that does not parse gives `503`) instead of the process
  failing to start; making this loud is backlog `OPS-4`.
  This is deliberate (see [docs/ARCHITECTURE.md](ARCHITECTURE.md)),
  but it means a misconfigured `DATABASE_URL` degrades quietly rather than
  loudly — watch logs and the 503 rate, not just process uptime.
- **Session/credential cleanup is best-effort, not guaranteed.** A
  background job purges expired sessions, revoked credentials, and stale
  throttle rows hourly. It runs in-band and may skip a tick under load;
  nothing guarantees a session or credential is removed within a
  particular bound. See `AUTH_RETENTION_DAYS` in [CONFIGURATION.md](CONFIGURATION.md).
- **Active service tokens never expire.** The cleanup job deletes only
  what is already expired or revoked; a non-revoked service token is
  valid indefinitely no matter how old, so rotation is a manual,
  operator-driven act (see `docs/OPERATIONS.md`).
- **A previously-internal API key and internal LAN hostnames are present
  in git history** (2 and ~10 commits reachable from `main`,
  respectively), predating this repo going public. The key must be, and
  has been treated as, compromised. See the "Known exposure" section of
  [SECURITY.md](../SECURITY.md) and [docs/CI.md](CI.md) (the
  `gitleaks (full git history)` job is intentionally red because of this).
- **The backend was ported from TypeScript to Rust by an AI agent
  workflow**, reviewed by AI reviewers, task by task, with a parity harness
  comparing responses against the original TypeScript backend along the
  way. The commit history has not had a full human security/architecture
  review end to end — treat it accordingly, especially before production
  use.
