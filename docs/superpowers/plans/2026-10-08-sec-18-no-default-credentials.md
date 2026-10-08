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

## 5. Review (planner)
