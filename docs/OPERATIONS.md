# Operations

This document covers running the backend stack locally with `docker
compose`, backing up/restoring Postgres, and the operational traps in this
codebase worth knowing about before you run it against real data.

## Local stack: what's in `docker-compose.yml`

```
docker compose up --build
```

brings up three services:

| Service | Image | Purpose |
| --- | --- | --- |
| `postgres` | `postgres:16` | OLTP store (identity, governance, pipelines, connectors, alerts, ...) |
| `clickhouse` | `clickhouse/clickhouse-server:24.8` | Analytics store (catalog, overview, governance, dashboards, ...) |
| `lakehouse-api` | built from `rust/Dockerfile` | the axum API, port 8080 |

Both data stores have healthchecks; `lakehouse-api` waits for both to
report healthy (`depends_on: condition: service_healthy`) before starting,
so first-boot migrations and the bootstrap-admin seed (both of which need
Postgres) get a real chance to run instead of racing container startup.
This is a start-order convenience, not a hard runtime dependency — see
"Postgres-down is quiet," below.

Copy `.env.example` to `.env` first (`docker compose` auto-loads `.env`
from the project root) and set at minimum `AUTH_BOOTSTRAP_EMAIL` /
`AUTH_BOOTSTRAP_PASSWORD`. Every other variable has a safe local default.

### What's deliberately NOT in the stack

- **The Next.js frontend.** Its Dockerfile is untracked, ad hoc work in
  progress on this branch and does not exist in a clean clone — building a
  competing one here would fork that effort. Run it directly instead:
  ```bash
  bun install
  RUST_API_URL=http://localhost:8080 bun --bun next dev
  ```
  against the backend stack `docker compose up` brought up.
- **Dagster.** Heavy (webserver + daemon + a code-location container) and
  out of scope for a local dev loop. Pipeline trigger/run-status routes
  return `503` without it.
- **A real LLM.** Needs a paid API key. AI chat / agent / text-to-SQL
  routes return `503` without `LLM_KEY` (or `MINIMAX_API_KEY`) set to a
  working key.

See "Features unavailable locally," below, for the full list and how to
turn each one on.

## Healthcheck: what `GET /health` actually checks

`GET /health` is a **plain liveness check** — it returns `200 ok`
unconditionally, with no dependency on Postgres, ClickHouse, Dagster, or
the LLM (`rust/crates/lakehouse-api/src/routes/mod.rs`, `async fn
health()`). It answers "is the process up and serving," not "are its
dependencies reachable." That's sufficient for compose's own
`depends_on`/orchestration needs (which target `postgres` and
`clickhouse`'s own healthchecks directly, not this endpoint), and it's
what the existing `#[tokio::test] health_returns_200_ok` locks in.

**We did not add a `GET /health/ready` in this pass.** It would be
genuinely useful (a real k8s/compose readiness probe should reflect
dependency health, not just liveness), but every route file that would
need touching to add one — `main.rs`, `routes/mod.rs` is fine, but wiring
in a new handler that calls out to ClickHouse/Postgres crosses into
territory this phase was told not to touch casually — plus the DoS
consideration (a readiness probe must do a *bounded*, cheap check —
e.g. `SELECT 1` / `ChClient::query("SELECT 1")` with a short timeout, not
a full dependency traversal, and definitely not hitting Dagster or the LLM
on every probe) needs its own review, not a drive-by addition. Proposed
shape, for whoever picks this up:

```
GET /health/ready
  -> 200 {"postgres": "ok"|"unavailable", "clickhouse": "ok"|"unavailable"}
     if Postgres is configured (state.pg.is_some()), a bounded SELECT 1
     against it (short timeout, e.g. 1s) — degrade to "unavailable" on
     error rather than 500
     same for ClickHouse: a cheap `SELECT 1` / `/ping`, bounded timeout
     never hit Dagster or the LLM here — they're already excluded from
     "core" health by design (see Config's doc comments) and calling out
     to them on every probe is exactly the unbounded-upstream-call
     DoS shape to avoid
  -> always 200 (a probe endpoint shouldn't itself flap the process's
     perceived liveness); readiness is communicated in the body, not the
     status code, unless the orchestrator specifically wants a non-200 to
     pull the pod from a load balancer — pick one and document it
```

## Bootstrap admin: there is no default credential

`AUTH_BOOTSTRAP_EMAIL` / `AUTH_BOOTSTRAP_PASSWORD` seed exactly one admin
account, idempotently, on every boot (`main::bootstrap_admin`). **If you
don't set both, no account is created and there is no way to log in** —
this is deliberate (see the doc comment on `bootstrap_admin`): a
hardcoded fallback credential would be a standing backdoor. If you forget
to set them before first boot, set them now and restart the container —
the seed re-runs and succeeds (it only no-ops if that *specific* email is
already taken).

The created identity has `must_change_password = true`; the login
response includes `mustChangePassword: true` and the client is expected to
force a password change before continuing (see
`routes::auth::change_password`, which enforces this server-side, not
just as a UI hint).

## Postgres-down is quiet — this is the biggest operational trap here

**The service does not refuse to boot when Postgres is unreachable.**
`lakehouse_store::connect_lazy` never does network I/O at startup; a
misconfigured or dead `DATABASE_URL` is discovered lazily, at first use.
Dependent (Phase 2: identity, auth, governance-writes, pipelines,
connectors, alerts, ...) routes degrade to `503` one request at a time
instead of the process failing to start or a healthcheck failing loudly.

**In practice this means:** a broken `DATABASE_URL` looks, from the
outside, like "the process is up and `/health` is green" while every
login attempt and every Phase 2 route silently 503s. Watch the `503` rate
and the structured logs (`tracing::error!` on migration/bootstrap
failures), not just process uptime or the liveness check, when diagnosing
"nothing works" reports. The same applies to ClickHouse-backed routes,
which 503 the same way if ClickHouse is down — the difference is only
that in the local compose stack, ClickHouse *is* wired into
`depends_on: condition: service_healthy`, so this specific failure mode is
less likely to bite you locally than in a deployment that skips that
check.

## Features unavailable in the local stack

| Feature | Needs | Symptom without it | To enable |
| --- | --- | --- | --- |
| Pipeline trigger / run status | Dagster | `503` from `/api/pipelines/*` | Point `DAGSTER_URL`/`DAGSTER_REPO`/`DAGSTER_LOCATION` at a real Dagster instance |
| AI chat / agent / text-to-SQL | LLM API key | `503` from `/api/ai/*`, `/api/agent/*` | Set `LLM_URL`/`LLM_MODEL`/`LLM_KEY` (or `MINIMAX_API_KEY`) to a real OpenAI-compatible provider |
| Alert digests / threshold emails | SMTP | Alerts still evaluate; email delivery silently no-ops | Set `SMTP_HOST` (and friends) to a real SMTP relay |
| Signed dashboard embeds | `EMBED_SECRET` | Embed routes unavailable | Set `EMBED_SECRET` |
| SSO / OIDC login | An OIDC provider | Local password auth only | Set `OIDC_ISSUER` + `OIDC_CLIENT_ID` (see `rust/crates/lakehouse-auth/README.md`) |

## Proposal: `GET /api/auth/providers` (not built)

**SSO is currently gated by a build-time flag, not a runtime one.** The
frontend can't read the Rust process's environment directly, so whether
the SSO login button shows up is controlled by `NEXT_PUBLIC_SSO_ENABLED`
at *Next.js build time* — it can't react to whether `OIDC_ISSUER` /
`OIDC_CLIENT_ID` are actually configured on the backend at runtime. That
means a deployment can build with SSO UI enabled but the backend
unconfigured (dead button), or the reverse (backend ready, but the UI
never shows it without a rebuild).

**Proposal:** add `GET /api/auth/providers`, unauthenticated, returning:

```json
{ "local": true, "oidc": { "enabled": true, "providerName": "okta" } }
```

derived directly from `AppState::auth.oidc.is_some()` (already computed at
startup from `OIDC_ISSUER`/`OIDC_CLIENT_ID` — see `state.rs`) and
`Config::oidc_provider_name`. The frontend would call this once (e.g. on
the login page mount, or server-side in the login route) instead of
reading a build-time env var, and show/hide the SSO button based on the
live response.

**Rationale:**
- Removes the "backend truth, frontend build flag" split — one source of
  truth, read at request time.
- Zero new attack surface: this endpoint answers "is SSO configured," not
  "here are the secrets" — no issuer secrets, client secrets, or JWKS
  contents belong in the response, only booleans/labels already meant to
  be public UI copy (the provider name shows up on the login button
  either way).
- Lets ops flip OIDC on/off (e.g. during an incident, or a provider
  migration) by restarting the Rust process with new env vars, without a
  frontend rebuild+redeploy.

**Not built in this phase** because it touches `routes/auth.rs` and
`routes/mod.rs` router wiring, which is out of scope here — left as a
proposal for whoever owns the auth surface next.

## Postgres backup / restore

Scripts: `scripts/backup-postgres.sh`, `scripts/restore-postgres.sh`. Both
assume the `docker-compose.yml` `postgres` service is running and shell
out to it via `docker compose exec`.

### Backup

```bash
scripts/backup-postgres.sh [output-dir]   # default: ./backups
```

Runs `pg_dump -Fc` (custom format — compressed, supports selective
`pg_restore`) inside the `postgres` container and writes
`<db>-<UTC-timestamp>.dump` into the output directory (git-ignored;
`backups/` is not meant to be committed). Prunes dumps for that database
older than `RETENTION_DAYS` (default 14) after each run.

### Restore

```bash
scripts/restore-postgres.sh <dump-file> [target-db]
```

Restores via `pg_restore --clean --if-exists --no-owner`. **Always
restore into a scratch database first** (`[target-db]`) to verify a dump
before trusting it — restoring into the live database name is destructive
(`--clean` drops existing objects before recreating them). The script
creates `[target-db]` if it doesn't already exist.

### Retention and location

Backups land in `./backups/` by default (override with `BACKUP_DIR` or
the first positional argument). This directory is local to whatever host
runs the script — for anything beyond local dev, point `BACKUP_DIR` at
durable, off-host storage (a mounted volume synced elsewhere, object
storage, etc.) and run the backup script on a schedule (cron/systemd
timer/CI job), not just ad hoc. `RETENTION_DAYS` (default 14) controls how
long local dumps are kept before the backup script prunes them on its own
next run — it does not proactively delete on a timer by itself.

This procedure was tested end-to-end as part of this phase: a backup was
taken, a scratch database was restored from it, and the restore was
verified by querying the restored data. See the phase report for the
actual command transcript.
