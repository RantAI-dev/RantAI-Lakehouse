# `SEC-18` No default credentials in compose — Implementation Plan

**Status:** ready to build. Written 2026-10-08 by the planner (Claude Opus)
for a developer agent. **Spec:** `docs/core/specs/sec-18.md`. **Feature
page:** `docs/core/features/no-default-credentials.md` (decisions 1-6).
**Base:** `main`. Branch `fix/sec-18-no-default-credentials`.

## 1. What exists today (read at `29ca545`; re-find by grep)

`docker-compose.yml` has no `${X:?}` at all. Known-value defaults:
`POSTGRES_PASSWORD:-lakehouse` (about twenty places, one inside the
`DATABASE_URL` default), `RUSTFS_ACCESS_KEY` / `RUSTFS_SECRET_KEY`
`:-rustfsadmin`, `LAKEKEEPER_ENCRYPTION_KEY:-this-is-not-a-secure-key-change-me`,
`SEAWEEDFS_ACCESS_KEY` / `SEAWEEDFS_SECRET_KEY` `:-seaweedfsadmin`, and
`CONNECTOR_PG_PASSWORD` / `CONNECTOR_S3_*` falling back to those.
`.env.example` carries the same values, and `AUTH_BOOTSTRAP_PASSWORD=change-me-now`.

## 2. Tasks (one per commit)

### N1 — Compose
Every occurrence becomes `${X:?message}` (or a fallback to a must-set
variable, decision 2). The message says what the variable is for. Check
that a variable used only by a profile's services does not stop a default
`up` that never starts them: compose interpolates the whole file, so say
what you found and how the file handles it. Overrides under `ops/` follow.
- **Accept:** `docker compose --profile '*' config --quiet` fails naming a
  missing variable when one is unset, and passes with all set.

### N2 — `.env.example` and the local helper
Decisions 4 and 5. The helper is a short script under `ops/` (or a
documented command) that writes generated values to `.env` and refuses to
overwrite an existing file.
- **Accept:** `.env.example` copied as it is does not start the stack.

### N3 — CI, gates and documents that start the stack
Every workflow step, gate script, test harness and document that runs
compose and relied on a default sets its own values or tells the reader
to. `AGENTS.md`'s verification line for `docker compose config` and
`docs/OPERATIONS.md` follow.
- **Accept:** CI green.

### N4 — Documents
`CHANGELOG.md` with the upgrade note of the feature page's limits.

## 3. Out of scope
Rotating deployed secrets; the empty-default tokens; ClickHouse's
passwordless default user (report it; it needs its own item).

## 4. Handoff (developer)

Branch `fix/sec-18-no-default-credentials`, from `main` (`97f6f1d`). Not pushed.

Commits: N1 compose (`f7faa69`), N2 `.env.example` and `ops/init-env.sh`
(`cec8f99`), N3 CI, gates and documents (`666670f`), N4 CHANGELOG and this
entry (this commit).

Must-set (`${X:?message}`): `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`,
`RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY`. `CONNECTOR_PG_PASSWORD` and
`CONNECTOR_S3_*` fall back to them. `SEAWEEDFS_ACCESS_KEY` / `_SECRET_KEY`
default to empty and `seaweedfs-iam-init` exits 1 while either is empty (every
service of that profile waits on it); `${X:?}` there would have stopped a
default `up`. Not changed: `POSTGRES_USER`, `POSTGRES_DB` (names, not secrets).

Commands run, from `/home/hv/lakehouse-sec18`:
- `docker compose --profile '*' config --quiet` with none, then some, then all
  four variables set: fails naming the first missing one with its message,
  passes with all four. Result: as described.
- The same for the 8 `.env` blocks extracted from `ci.yml`: 8 of 8 pass.
- Same with `ops/g6/*.override.yml` and `ops/backups/*.override.yml`: they
  already carried their own `${VAR:?}` (MYSQL_ROOT_PASSWORD, BACKUP_S3_*); they
  needed no change.
- `sh ops/init-env.sh` in a temporary copy: filled 9 names, mode 600, second
  run refused (exit 1); the generated `.env` passes `config --quiet`;
  `.env.example` copied as it is fails it.
- `sh -n ops/init-env.sh`; `python3 -m py_compile` on the changed gate;
  `python3 ops/lint/check_intra_package_imports.py`,
  `check_bare_iceberg_count.py`, `check_compose_init_readiness.py`: pass.
- YAML of both workflows parses.

NOT verified: a real `docker compose up` from a clean project (rule 8; not
allowed on this machine), the SeaweedFS guard in a running container, the
staging deploy step (needs the six `STAGING_*` secrets), `shellcheck`
(not installed), cargo/bun/pytest (nothing under `rust/`, `src/`, `dagster/`
changed). CI's compose jobs are the first real start.

Left alone, for the planner: `ops/g3/g3_loadgen.py` and
`dagster/.../dlt_pipeline.py` fall back to the literal `rustfsadmin` when
`CONNECTOR_S3_*` is unset (compose always sets them now); Rust code and tests
keep `postgres://lakehouse:lakehouse@localhost` as a default and fixtures
(`config.rs`, `cdc.rs`, tests); ClickHouse's passwordless `default` user
(`CH_PASSWORD` empty); empty-default tokens and `UPLOAD_S3_*`,
`TENANT_WAREHOUSE_S3_*` (empty means not configured).

## 5. Review (planner)
