# ADR 0001 — Dockerfile convergence and the `tenant` module

- **Status:** Accepted
- **Phase:** P0
- **Date:** 2026-08-31

## Context

A clean clone did not build. Two independent causes:

1. `lakehouse-api/src/main.rs` declared `mod tenant;` while `src/tenant.rs`
   was untracked. Five route modules (`ops`, `dashboard`, `governance`,
   `catalog`, `pipelines`) import from it.
2. Two Dockerfiles fixed the same problem in parallel: `rust/Dockerfile`
   (pinned an unbuildable `rust:1.85-slim`) and `rust/Dockerfile.api`
   (committed, and what `docker-compose.yml` actually built from).
   `docs/OPERATIONS.md` documented the split as deliberate and temporary.

## Decision

**Commit the `tenant` module.** It is load-bearing for five route modules;
removing it would mean reverting real work. Its values are read from the
environment with the previous hardcoded literals as defaults, so one image
can serve multiple deployments — a prerequisite for tenant-scoped Lakekeeper
warehouses in ADR 0003.

**Converge on `rust/Dockerfile`, carrying `Dockerfile.api`'s content.** The
`.api` variant was strictly better on three counts: it pins `rust:1.96.1-slim`
matching `rust-toolchain.toml` (rather than the MSRV floor, which only
happens to satisfy today's lockfile); it matches the runtime base to
`debian:trixie-slim`, avoiding a glibc skew that made a bookworm runtime
unable to exec the trixie-built `sqlx` binary; and it installs `sqlx-cli`
and runs migrations via `entrypoint.api.sh`, which the plain Dockerfile did
not do at all.

## Consequences

- `rust/Dockerfile.api` is deleted. `docker-compose.yml` and
  `.github/workflows/docker.yml` both target `rust/Dockerfile`.
- `lib.rs` needed `pub mod tenant;` too — the crate has a "thin bin, real
  lib" split and integration tests compile the route modules under the lib
  target. Without it, `cargo build -p lakehouse-api` fails with five
  `E0432` errors. This was missed in the original diagnosis.
- **Migrations now run twice on boot:** once in `entrypoint.api.sh` via
  `sqlx-cli`, once in `main.rs` via `lakehouse_store::migrate`. Both are
  idempotent (`_sqlx_migrations`), so this is correct but redundant. It
  costs a `cargo install sqlx-cli` in the builder stage and the `sqlx`
  binary in the runtime image. Neither path is fatal on failure, so the
  second path adds no real guarantee. **Follow-up:** collapse to the
  in-process migration and drop `sqlx-cli` from the image. Not done in P0
  because it changes boot behavior on a phase whose only job was to unblock
  the build.

## Verification

Clean clone → `docker compose up --build` → all three containers healthy →
`GET /health` returns `ok`. All 20 migrations applied. `cargo fmt --check`,
`cargo clippy --all-targets --all-features --locked -- -D warnings`, and
`cargo test --all-features` (30 test targets, 0 failures) all pass.

## Addendum (SEC-78, 2026-11-18)

### Context

The Decision and Consequences above no longer describe the image.
`entrypoint.api.sh` no longer runs `sqlx migrate run`, and `lib.rs` no
longer calls `lakehouse_store::migrate` at boot: at start the API reads
the migration bookkeeping instead and refuses to serve while a pending
migration exists. SEC-78 moved the migration out of the API process
entirely.

### Decision

**The API never migrates.** A one-shot compose service,
`lakehouse-db-migrate`, runs the same image with the binary's `migrate`
subcommand, holds the database OWNER's credentials, and runs on every
deploy before `lakehouse-api` starts. `sqlx-cli` stays in the image, but
nothing runs it any more; dropping it from the image remains the follow-up
the Consequences above call for (`rust/Dockerfile` marks the removal as
"ADR 0001's follow-up").

### Consequences

- The console connects as `lakehouse_app`, a role that owns no table and
  is not a superuser, so it cannot switch off the audit trail's
  append-only triggers (`0142_audit_event_append_only.sql`).
- The owner's role is kept, for the migrations and the runtime role's own
  setup: applying the schema is the one job that needs the owner's rights,
  and it no longer runs inside the console's process.
- The image carries a `sqlx` binary nothing runs, until the follow-up
  drops it.

### Verification

This addendum records behaviour already in the tree, not a new decision to
build: `rust/entrypoint.api.sh` ("The API NEVER migrates"), the
`sqlx-cli` comment in `rust/Dockerfile`, and the `lakehouse-db-migrate`
service in `docker-compose.yml` say the same thing. The addendum itself
changes documentation only.
