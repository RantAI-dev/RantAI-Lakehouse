# `SEC-15` Connection tests cannot reach internal addresses — Implementation Plan

**Status:** ready to build. Written 2026-10-08 by the planner (Claude Opus)
for a developer agent, under the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/sec-15.md`. **Feature page:**
`docs/core/features/connection-tests-internal-addresses.md` (decisions 1-7).
**Base:** `main`. Branch `fix/sec-15-probe-internal-addresses`.

## 1. What exists today (verified by reading at `29ca545`; re-find by symbol)

| # | Spec row | Code today |
| --- | --- | --- |
| F1 | Default in compose | `docker-compose.yml` sets `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS: ${…:-true}`; `config.rs` treats unset as blocked; `.env.example` sets `true`. Compose's own comment says "blocked by default". |
| F2 | REST redirects | `connector_probe.rs` `probe_rest` builds `reqwest::Client::new()`: up to ten redirects, only the first host checked. |
| F3 | Pinning | `resolve_checked` returns `Result<(), String>`; every dialer then connects by name (PostgreSQL, MySQL, SQL Server, REST, object storage, and the same builders in `connector_discover.rs`). |
| F4 | CDC delete | `routes/connectors.rs` `delete` → `deprovision_postgres_connector` → `connector_deprovision::drop_slot_and_publication` dials with no check. |
| F5 | Messages | `resolve_checked` formats the resolved address and the resolver's error into the message, which reaches the test result, the probe history, a 400 on ingest-spec, a 422 on discover and on credential save. |

## 2. Tasks (one per commit)

### K1 — The check returns what it approved, and says nothing it should not
- `resolve_checked` returns the approved address(es) for the host and
  port. Its refusals are fixed sentences (feature page decision 6); the
  detail goes to the log, not the caller.
- **Accept:** unit tests: a blocked name, a mixed answer (one public, one
  internal: refused), an allow-listed network, allow-all, an unresolvable
  name; no test output contains an address.

### K2 — Every dial uses the approved address
- PostgreSQL, MySQL/MariaDB, SQL Server, REST, object storage, in the
  probe and in discovery. Where a library cannot dial a chosen address,
  decision 5.
- **Accept:** per dialer, a test that the connection goes to the approved
  address even when the name would now resolve elsewhere (use what the
  existing probe tests use to stand in for a server and a resolver).

### K3 — REST redirects are not followed
- **Accept:** a test server answering 302 to an internal address: the
  test is reported as a redirect and the second request is never made.

### K4 — Delete checks before it dials
- Decision 7.
- **Accept:** route tests: refused target → fixed refusal and the row
  stays; `force=true` → row removed and no dial.

### K5 — The default
- `docker-compose.yml`: unset means blocked; the comment matches.
  `.env.example`: the variable empty, with the opt-in and the allow-list
  explained. Gates and CI that need internal hosts set it themselves:
  find every compose file, override and workflow step that relied on the
  old default and set it there explicitly, with the reason.
- **Accept:** `docker compose --profile '*' config --quiet`; CI green on
  the gates.

### K6 — Documents
- `CHANGELOG.md`, `docs/OPERATIONS.md` (the setting, the allow-list, what
  a fresh install sees), the feature page's limits if anything changed.

## 3. Out of scope
TLS modes and what is sent to a server (`SEC-14`); the orchestrator's own
guard; tenant scoping (`SEC-16`).

## 4. Handoff (developer)

## 5. Review (planner)
