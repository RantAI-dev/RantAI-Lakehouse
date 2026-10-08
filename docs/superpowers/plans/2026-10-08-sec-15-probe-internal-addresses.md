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

Branch `fix/sec-15-probe-internal-addresses`, off `main` at `a013e9d`, not
pushed. One commit per task (hashes: `git log --oneline a013e9d..`).

### Commits

| Task | Commit subject | What |
| --- | --- | --- |
| K1 | the connector address check returns what it approved and names no address | `Approved`, fixed refusals, wider block list, `internal_hosts::never_listed` gains `0/8` |
| K2 | every connector dial connects to the address that was checked | `pg_connect_options`, `mysql_connect_options`, `connect_pinned`, `pinned_http_client`, `PinnedHttpConnector`, `s3_client`; `reqwest` 0.13 as a direct dependency |
| K3 | the REST and S3 connection tests do not follow redirects | `Policy::none()` on both clients; fixed redirect message |
| K4 | deleting a change-capture connector checks its target before it dials | check in `deprovision_with_names`; `DeprovisionError::summary`; fixed 409 text |
| K5 | connection tests block internal addresses unless the operator opts in | `docker-compose.yml`, `.env.example`, the two `ops/g6` overrides |
| K6 | documents | `CHANGELOG.md`, `docs/OPERATIONS.md`, `docs/CODE-STANDARD.md`, feature page limits, this section |

### Block list, before and after

Before (`is_blocked_ip`): RFC1918, loopback, link-local, unspecified; IPv6
loopback, unspecified, unique-local `fc00::/7`, link-local `fe80::/10`;
IPv4-mapped IPv6 carrying any of the IPv4 ones.

After (added by `SEC-15`): multicast `224.0.0.0/4` and `ff00::/8`;
carrier-grade NAT `100.64.0.0/10`; the rest of `0.0.0.0/8`; reserved
`240.0.0.0/4` (includes `255.255.255.255`); IPv6 site-local `fec0::/10`; IPv4
embedded in an IPv4-compatible address (`::a.b.c.d`) or the NAT64 well-known
prefix (`64:ff9b::/96`), next to the existing mapped form. The allow-list
(`internal_hosts::never_listed`) additionally refuses to open all of `0/8`.
Multicast was already in the orchestrator's `ssrf_guard.py` and in
`never_listed`; the orchestrator's list was not changed.

### Pinning, per dialer

| Dialer | How | Decision 5 |
| --- | --- | --- |
| `PostgreSQL` (`sqlx`) | approved IP and port passed as host and port | not needed |
| `MySQL`/`MariaDB` (`sqlx`) | same | not needed |
| SQL Server (`tiberius`) | `TcpStream` connected to the approved socket address; the name stays in the client config | not needed |
| REST (`reqwest` 0.12) | `resolve_to_addrs(host, approved)`, no redirects, no system proxy, URL re-parsed by the `url` crate and refused unless it names the checked host and port | not needed |
| S3 (`object_store` 0.14) | `ClientOptions::with_dns_resolver` can pin a name but `object_store` follows redirects and cannot be told not to, so an `HttpConnector` builds a pinned `reqwest` 0.13 client (no redirects, no proxy) | not needed |
| Delete clean-up (`sqlx`) | approved IP and port in `PgTarget` | not needed |

Every dialer could be pinned, so the "refuse while the block is on" fallback
of decision 5 is not used anywhere. A name with several addresses is refused
if any is blocked; the single-address clients then dial the first approved
address and do not fall back to the others (so a dual-stack name whose first
address is unreachable now fails where the client used to try the next).

### Deviations from the plan, for the reviewer

- `reqwest` 0.13 is a new direct dependency of `lakehouse-api`
  (`reqwest013`, `default-features = false`, feature `rustls`). The plan did
  not foresee it: `object_store` gives no other way to stop redirects. One
  line added to `rust/Cargo.lock`; no new crate version.
- The two redirect and pinning changes share `pinned_http_client` and
  `PinnedHttpConnector`, so K2 and K3 touch the same functions; K3 adds the
  `Policy::none()` lines, the redirect message and the redirect tests.
- `DELETE` with `?force=true` still attempts the clean-up when the target is
  allowed (as before) and ignores its failure. "Removes the row without
  dialling" holds for a refused target, which is never dialled.
- `ApiError::Conflict` carries the refusal from `deprovision_with_names`
  (the other clean-up failures stay `Internal`); `delete` turns either into
  the same 409.
- Messages that changed meaning for the console: none matched by `src/`
  (grep for the old texts found only a mocked 409 string).

### Messages a caller can now receive from these paths

`{host}` is the host string the caller typed; `{class}` is one of the
existing fixed classes ("connection refused", "timed out", "authentication
failed", "TLS error", "permission denied", "database rejected the
connection", "connection failed").

- Refused address: `refusing to dial "{host}": it resolves to a private/internal address, which this installation does not allow connection tests to reach (an operator can list its network in CONNECTOR_PROBE_ALLOWED_CIDRS, or set CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true to allow every internal address for a trusted internal deployment)`
- Unresolvable: `could not resolve host "{host}"`; no address: `host "{host}" did not resolve to any address`.
- Test result, probe history, credential save (`the credential was NOT saved: ...`), ingest-spec PUT (400) and discovery (422) carry the three texts above unchanged.
- REST: `REST request was answered with a redirect (HTTP {n}); connection tests do not follow redirects`; `REST request rejected: HTTP {n}`; `REST request failed: {class}`; `REST request timed out after 5s`; `could not build the HTTP client for this test`; `connector is misconfigured: a rest adapter's baseUrl must be a plain http(s):// URL with a host and an optional port, and no user name or password`.
- S3: `S3 connection failed: {class}`; `S3 connection timed out after 5s`; `failed to build the S3 client for this connector`; `connector is misconfigured: S3 endpoint must be a plain http(s):// URL with a host and an optional port, and no user name or password`.
- SQL: `{PostgreSQL|MySQL|SQL Server} connection failed: {class}` or `... connection timed out after 5s`; discovery: `could not discover the connector's schema: {class}`, `discovery timed out after 5s`.
- Delete (409): `connector {id} was NOT deleted: dropping {slot/publication} did not complete ({detail}); the registry row is kept deliberately ... pass ?force=true ...`, where `{detail}` is `the source database was not contacted: {one of the refusals above}` or `deprovisioning connector {id}'s CDC replication slot ("{slot}") and publication ("{pub}") failed: {summary}`, with `{summary}` one of `could not connect to the source database ({class})`, `dropping the publication failed`, `ending the session that holds the replication slot failed`, `checking the replication slot failed`, `dropping the replication slot failed`, `the source database did not answer within 5s`, `{field} is not a valid identifier`; or `could not resolve connector {id}'s credential to deprovision its CDC slot`.
- Still interpolated, unchanged and not network text: `could not resolve the connector's credential: {err}` (the allow-listed resolver's own refusal) and `connector's dial is invalid: {err}` (the saved dial's own validation).

### Compose: what relied on the old default

Only the two `g6` gates (they save ingest specs aimed at fixtures in the
compose network, and `PUT .../ingest-spec` runs the API's address check).
Both now set `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS: "true"` on
`lakehouse-api` in their own override, commented gate-only. Nothing else
under `ops/` or `.github/` calls a connector test, discovery, credential,
ingest-spec or delete route.

### Commands run (this machine; test binaries are not allowed here)

- `cd rust && cargo fmt --check`: clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  (shared `CARGO_TARGET_DIR`, 2 jobs): clean, on the final tree of K1-K5
  (K6 changes no Rust).
- `docker compose --profile '*' config --quiet`: exit 0; also with each g6
  override and dummy values for its must-set variables: exit 0.
- `python3 ops/lint/check_intra_package_imports.py` and
  `check_bare_iceberg_count.py`: OK.

### NOT verified

- `cargo test` of any kind: **none of the new or changed Rust tests was run**
  (they type-check under clippy `--all-targets`). First run is CI.
- `docker compose up` of the default stack or of either g6 project; the two
  g6 gates are untested after the override change.
- Nothing was run against a real `PostgreSQL`, `MySQL`, SQL Server or S3
  server; pinning of the `sqlx` and `tiberius` clients is proved only by
  what the option objects and the socket helper carry.
- `bun typecheck/lint/test`: not run, no `src/` change.

### Known residual (not in the Target table)

"Connection refused" versus "timed out" still tells a caller who may test
connectors whether a port is open, on an address that is allowed.

## 5. Review (planner)
