# Data roadmap Phase 4: connector breadth (master plan)

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-19 |
| Roadmap | "Data Build Roadmap", phase 4 "Connector breadth": `SRC-3`, `SRC-4` |
| Base | `origin/main` at `ee8589c3`. Every anchor below was read there |
| Branch | `feat/data-phase-4-connectors`, worktree `/home/hv/lakehouse-phase4-connectors`, draft PR #214. One branch, one PR for the phase |
| Spec | `docs/core/specs/src-3.md` |
| Feature page | `docs/core/features/business-app-connectors.md`: fifteen decisions, signed 2026-10-19 (every default; decision 3 corrected to the spec's "OAuth where offered" before signing) |
| `SRC-4` | **Not in this plan.** Blocked by `SEC-14` (part B waits for `SEC-41`, which waits for `REL-9`). It gets its own plan on this branch once `SEC-14` merges (product owner, 2026-10-19) |
| Migrations | `0095`, `0096` and `0097` (added at the slice 1 review) reserved by this plan (free on main and in every open branch's plan on 2026-10-19: #106 holds 0078–0083, BI phases 3–4 0085–0088, data phase 3 0090–0094) |

## 1. Scope

`SRC-3`: Databricks' ten generally available SaaS connectors (Salesforce,
ServiceNow, Workday Reports, Google Analytics 4, HubSpot, Zendesk,
Confluence, Dynamics 365, NetSuite, SharePoint), each with sign-in, object
listing, object and column choice, incremental loads, schema-change
handling, failure alerts and a gate (spec "Each connector" row). `DEC-9`:
dlt verified sources for Salesforce, HubSpot, Zendesk, Google Analytics;
dlt's `rest_api` source for the other six.

Spec targets not met, reported on the feature page: the second wave, Workday
incremental, and real-account proof for Workday and NetSuite.

## 2. What exists (read at `ee8589c3`, not run)

`A` = `rust/crates/lakehouse-api/src`, `S` = `rust/crates/lakehouse-store/src`,
`D` = `dagster/dispar_orchestrate`.

- **Types are rows, adapters are strings.** `connector_type`
  (`rust/migrations/0035_connector_type.sql:10-37`, widened in
  `0043_ingest_tier2_adapters.sql:69-87`) with CHECKs on `adapter` in both
  `connector_type` and `connector` (`0033:15`). The adapter string is
  switched on in about fifteen places: Rust `Dial` (`A/ingest_spec.rs:473-557`),
  `secret_field_names` (`:1791-1822`), `target_identity` (`:621-756`),
  load modes (`:1134-1405`, incremental refused for non-SQL at `:1390`),
  column choice (`:1461`), probe dispatch (`A/connector_probe.rs:288-312`),
  listing dispatch (`A/connector_listing.rs:1082-1136`), OAuth `app_of`
  (`A/routes/connector_oauth.rs:89-110`, REST only), SSRF save checks
  (`A/routes/connectors.rs:3341-3376`, `:3394+`), the assistant tool enum
  (`A/routes/ai/registry.rs:1189,1200`); Python `_ADAPTERS` and the
  per-adapter arms (`D/ingest_factory.py:290-297`, `:795-845`), `_load_plan`
  (`:399`), `secret_map.py`, `discover.py:_reader_for` (`:703-711`); console
  `ADAPTER_META` (`src/features/connectors/connector-type-picker.tsx:40-58`),
  `credentialSlotsFor` (`connector-form-parts.tsx:113-150`), dial forms
  (`:255`, `dial-forms/*`), `table-finder.tsx:36,54`,
  `connector-review.tsx:72`, `hostFromDial` (`connector-form-parts.tsx:49`),
  contracts (`src/services/contracts/connectors.ts:685-694`, `:761-768`).
- **REST adapter.** `D/adapters/rest.py`, hand-written, five auth kinds, three
  paging shapes, rule Z5 at `:10-21`: no URL from a response body or header
  is dialled. `AuthSession` refreshes OAuth tokens and reports a rotated
  refresh token (`:387`).
- **Load path.** `run_ingest` → `_run_one_object` (`D/ingest_factory.py:664-877`):
  secrets resolve (`:728`), the load runs inside
  `ssrf_guard.pinned_resolution(host, …)` (`:851-856`), sink
  `D/adapters/sink.py:load_via_sink` (`:823`); `_apply_cursor` (`:690-722`)
  accepts only an `IncrementalResourceWrapper`. Non-SQL schema changes are
  observed after the load (`D/ingest_factory.py:406-447`). Retries
  `_fail_or_retry` (`:1131-1180`).
- **OAuth sign-in** (`SRC-12`): per-connector `RestAuth::Oauth2AuthorizationCode`
  (`A/ingest_spec.rs:199-217`), PKCE, `client_secret_post` only
  (`A/routes/connector_oauth.rs:31-34`), migration `0080`. Gate `g6-oauth`
  mocks the provider (`ops/g6/src12_oauth_test.py:103-168`).
- **SSRF.** Rust `resolve_checked` (`A/connector_probe.rs:1200-1213`),
  `pinned_http_client` (`:915-937`); Python `D/ssrf_guard.py`
  (`resolve_checked` `:137`, `pinned_resolution` `:187`, `checking_resolver`
  `:244`).
- **Gates.** SRC-9 lint `ops/lint/check_connector_gate_coverage.py` replays
  `connector_type` statements against `ops/gates/coverage.json`; its parser
  accepts only `INSERT … (name, adapter, supported, docs_url) VALUES (…)`
  and `UPDATE … SET supported/adapter/unsupported_reason WHERE name = '…'`.
  `g6-ingest-other` measured 14m53s–18m36s against the signed 20-minute
  budget (`docs/plans/SRC-9-RESULT.md:40`): no room, so a new job.
- **Python deps.** `dagster/pyproject.toml`: exact pins, `dlt[...]==1.30.0`;
  no vendored code; no pip audit; product licence AGPL-3.0-or-later.

## 3. Facts from outside the repo (2026-10-19)

Sources: github.com/dlt-hub/verified-sources (sources/salesforce, hubspot,
zendesk, google_analytics; issues and PRs #285, #659, #662, #692);
github.com/dlt-hub/dlt at tag 1.30.0 (`dlt/sources/rest_api/config_setup.py`,
`dlt/sources/helpers/rest_client/auth.py`, `paginators.py`); Databricks
Lakeflow Connect SaaS overview and release notes 2025–2026; the vendors' API
documentation (ServiceNow Table API, Confluence REST v2, Dataverse Web API
paging and change tracking, NetSuite REST/SuiteQL, Microsoft Graph delta,
Salesforce SOAP API end of life). Anything marked *verify* is checked by the
developer in the task named, before code depends on it (section 8).

- Verified sources install by copying (`dlt init`); Apache-2.0; no package.
  Salesforce: `simple-salesforce`, no `primary_key` (upstream PR #692),
  standard objects only (#285); HubSpot: full re-read (#662), private-app
  token only; Zendesk: duplicate report (#659); Google Analytics: GA4 Data
  API (`BetaAnalyticsDataClient`), service account or OAuth.
- dlt 1.30.0 `rest_api`: paginators `json_link`, `header_link`,
  `header_cursor`, `cursor`, `offset`, `page_number`, `single_page`; auth
  `bearer`, `api_key`, `http_basic`, `oauth2_client_credentials`; custom auth
  subclasses `AuthConfigBase`. No authorisation-code/refresh auth, no
  delta-link state.
- Vendor shapes: ServiceNow `Link` header + `sys_updated_on`; Workday RaaS
  one response, no change filter; Confluence v2 cursor in `_links.next`;
  Dataverse `@odata.nextLink`, `modifiedon`, change tracking; NetSuite
  SuiteQL POST with `offset`/`hasMore`; Graph `@odata.nextLink` and delta.
- Salesforce SOAP `login()` retired Summer '27 (decision 4).

## 4. Architecture decisions (planner)

- **AD1. One new adapter, `saas`, with a `vendor` field.** Not ten adapters:
  the fifteen switch sites gain one arm each, and the vendor is dispatched
  inside the SaaS code. `Dial::Saas { vendor, account, objects, auth }`,
  where `account` holds the vendor's per-customer host part (Zendesk
  subdomain, ServiceNow instance, Dataverse org URL, NetSuite account id,
  Workday host and tenant, Confluence site) and is validated per vendor
  against the vendor's host pattern. The connector type rows are ten
  `connector_type` rows with `adapter = 'saas'`.
- **AD2. Vendor hosts are built, never typed.** The API and token hosts
  come from a per-vendor table in code (`saas_vendors` in Rust and Python,
  one list each, compared by a test in each language against a shared JSON
  fixture). The user supplies only the `account` part. `target_identity`
  for `saas` is the vendor plus `account` (SEC-14 part A: changing either
  needs every credential again).
- **AD3. The link rule replaces Z5 for `saas` only** (decision 8). A
  `SaasSession` (Python, on `requests`) dials a URL from a response only if
  its scheme, host and port equal the request's (or, for Salesforce's
  `instance_url`, the host ends in `.salesforce.com` or `.force.com`, and
  then becomes the session's host), and every host goes through
  `ssrf_guard.resolve_checked` and is pinned. No redirects. `rest.py` keeps
  Z5 unchanged.
- **AD4. Verified-source code is vendored** (decision 1) under
  `D/saas/vendored/<source>/` from one pinned upstream commit, with
  `UPSTREAM.md` naming the commit and listing each local change with its
  reason, and the Apache-2.0 licence file. Our fixes are separate commits.
  Their HTTP goes through `SaasSession` (Salesforce: `simple-salesforce`
  accepts a `session`; HubSpot and Zendesk use dlt's client, given our
  session; GA4: the client's REST transport, *verify*, because gRPC resolves
  outside `getaddrinfo` and so outside the SSRF guard).
- **AD5. The six others are `rest_api` configs** in `D/saas/vendors/<vendor>.py`,
  each with its own custom `AuthConfigBase` where dlt has none (OAuth
  refresh, reused from `rest.py`'s `AuthSession`; NetSuite certificate JWT),
  and its own delta-link state where the vendor uses one (Graph, Dataverse
  change tracking: dlt resource state).
- **AD6. Incremental and column choice widen to `saas`.** Rust load-mode
  validation and `supports_column_choice`, Python `_load_plan` and
  `sink._apply_cursor` accept `saas` objects that declare a cursor; each
  vendor module declares per object `cursor`, `primary_key`, `deletes`
  (decision 7) and `full_reason` when no cursor exists (decision 6; the
  connector page shows it).
- **AD7. Schema changes before the load** (decision 9) for vendors that
  describe fields: each such vendor module implements `describe(object)`,
  called in the same place SQL sources are observed before the load. The
  others keep the after-load path.
- **AD8. OAuth sign-in for `saas`** reuses `SRC-12`'s routes and storage;
  the authorise and token URLs come from the vendor table, not the user.
  The token request gains `client_secret_basic` for vendors that need it
  (decision 3: extend, not drop).
- **AD9. One gate job, `g6-saas`,** with one mock server (`ops/g6/saas_stub.py`,
  stdlib only, as `rest_stub.py`) serving every vendor's routes on its own
  path prefix: sign-in, listing, paging by the vendor's own shape, a cursor
  filter, a delete, a 429 with `Retry-After`, a changed field. Budget: the
  signed 20 minutes.
- **AD10. Types stay unsupported until the end.** Migration `0095` adds the
  ten rows with `supported = false` and a reason; migration `0096`, in the
  last slice, flips the eight with a passing gate and a real-account row
  possible, and adds their `coverage.json` entries in the same commit.
  Workday and NetSuite stay unsupported (decision 10).

## 5. Slices (one branch, one PR)

| Slice | Content | Migration |
| --- | --- | --- |
| 1 | The `saas` adapter end to end with no vendor: types, dial, identity, secrets, probe arm, OAuth presets, listing dispatch, load path, incremental and column choice widened, console picker and form shell, gate job skeleton | `0095` |
| 2 | HubSpot, Zendesk (vendored; HubSpot made incremental, Zendesk duplicates fixed) | none |
| 3 | Salesforce (vendored; primary key, custom objects, deletes), GA4 (vendored; REST transport) | none |
| 4 | ServiceNow, Confluence (`rest_api`) | none |
| 5 | SharePoint, Dynamics 365 (`rest_api`, Entra, delta state) | none |
| 6 | Workday, NetSuite (`rest_api`; stay unsupported) | none |
| 7 | Flip to supported, docs, measurement | `0096` |

Rust work sits in slice 1 and 7 ("Keep build time down"); slices 2–6 are
Python, mocks and console, plus at most a vendor-table row in Rust each.

## 6. Tasks

One task per commit. Scoped checks per commit; the full block once per
slice before its handoff. Machine limits: no `cargo test` on this machine
(CI is the first run); `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target`,
`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`; run bun tests per directory.

### Slice 1: the `saas` adapter

- **T1.1 Migration `0095_saas_connector_types.sql`.** Widen both adapter
  CHECKs to include `saas`; insert the ten types, `supported = false`, with
  `unsupported_reason` "Being built (SRC-3)". Statements in the shapes the
  SRC-9 lint parses; if a CHECK change is outside its grammar, extend the
  lint in the same commit, with a test. *Check:* `python3
  ops/lint/check_connector_gate_coverage.py` passes; the lint test passes.
- **T1.2 Rust vendor table and dial.** `A/saas_vendors.rs`: per vendor the
  host pattern for `account`, API and token URL builders, OAuth authorise
  and revocation URLs, scopes, client authentication method, secret slots.
  `Dial::Saas` parse and validation, `secret_field_names`, `target_identity`
  (AD2), `check_dial_ssrf` arm. Shared fixture
  `ops/fixtures/saas_vendors.json` compared by a unit test. *Check:* unit
  tests: each vendor's `account` accepts a good value and refuses a host
  outside the pattern, userinfo, a port, `http`; identity changes with
  `account`; secrets are `SecretValue`.
- **T1.3 Load modes and column choice.** Widen `:1390` and `:1461` to `saas`
  objects with a declared cursor. *Check:* unit tests both ways (a `saas`
  object without a cursor still refuses incremental, with the message).
- **T1.4 Probe arm.** `connector_probe.rs` `saas` arm: the vendor's "who am
  I" call through `pinned_http_client` after `resolve_checked`, credentials
  read only after the address check; fixed-class messages; an
  authorisation-code connector with no refresh token answers
  `not_testable_with` as today. *Check:* wiremock tests for ok, 401, 403,
  429, redirect refused; `tests/sec11_guard.rs` passes.
- **T1.5 OAuth presets.** `connector_oauth.rs` accepts `saas`, taking URLs
  and scopes from the vendor table; add `client_secret_basic`. *Check:*
  route tests for start and callback with a `saas` connector; a vendor URL
  cannot be overridden from the dial.
- **T1.6 Listing dispatch.** `connector_listing.rs`: `saas` →
  `Dispatch::Orchestrator`. *Check:* unit test.
- **T1.7 Routes and policy.** Every new or widened route in `POLICY_TABLE`
  and both ways in `tests/route_auth.rs` (none expected beyond widening).
  The assistant tool enum gains `saas`; regenerate
  `tests/fixtures/tool_schemas.json`. *Check:* `route_auth`, tool schema
  test.
- **T1.8 Python SaaS core.** `D/saas/session.py` (`SaasSession`, AD3:
  same-origin links, the Salesforce host rule, `resolve_checked` + pinning,
  no redirects, `Retry-After` up to a bound then a classified failure),
  `D/saas/vendors/__init__.py` (vendor registry mirroring the fixture),
  `D/saas/objects.py` (per-object `cursor`, `primary_key`, `deletes`,
  `full_reason`), `secret_map.py` arm. *Check:* pytest, no network: a link
  to another host stops the run; a Salesforce host outside the suffixes
  stops it; a blocked address stops it before any credential is sent;
  429 then 200 succeeds; 429 past the bound fails as `rate_limited`.
- **T1.9 Load path.** `ingest_factory.py`: `saas` arm in `_run_one_object`,
  `_load_plan` and `_ADAPTERS`; `sink._apply_cursor` accepts a `saas`
  resource; before-load `describe` hook (AD7); `ingest_errors.classify`
  gains `sign_in_refused`, `permission_missing`, `rate_limited` for `saas`.
  *Check:* pytest with a fake vendor: incremental second run reads only the
  newer rows; merge on the primary key leaves no duplicate; a left-out
  column is absent; a describe change triggers the connector's rule.
- **T1.10 Discovery.** `discover.py`: `saas` reader calling the vendor
  module's `list_objects`. *Check:* pytest.
- **T1.11 Console.** Picker category "Business apps"; a dial form per vendor
  driven by the vendor fixture (only `account` and the sign-in choice);
  `credentialSlotsFor`; the connector page lists objects with
  "incremental" or "re-read in full: <reason>". Brand marks only where the
  current Simple Icons release publishes them (rule in `brand-marks.ts:21-29`).
  *Check:* `bun run typecheck`, lint, tests in `src/features/connectors`.
- **T1.12 Gate skeleton.** `ops/g6/saas_stub.py`, `Dockerfile.saas_stub`,
  compose profile `g6-saas`, `ops/g6/g6_saas_test.py`, CI job `g6-saas` in
  `ci.yml`, `ci-required`, `scripts/ci/required_gate.sh` and its test,
  `docs/CI.md`. With no vendor it asserts the ten types are listed as not
  supported. *Check:* the job passes in CI; time recorded.

### Slices 2–6: one vendor per task group

For each vendor, in decision 12's order, these tasks (vendored or
`rest_api` as AD4/AD5 say):

- **Vn.1 Code.** Vendored: copy at the pinned commit (`UPSTREAM.md`,
  licence), one commit unchanged; then each fix its own commit (HubSpot
  incremental by `hs_lastmodifieddate` through the search API; Zendesk
  duplicates; Salesforce `primary_key = Id`, custom objects via `describe`,
  `IsDeleted` through `queryAll`; GA4 REST transport). `rest_api`: the
  config, auth class, paging, cursor, delta state. Exact pins added to
  `dagster/pyproject.toml` with a dated note and the licence of each new
  library (MIT, Apache-2.0, BSD only; anything else stops and is reported).
- **Vn.2 Objects.** `list_objects`, `describe` where AD7 applies, per-object
  declarations (decisions 5, 6, 7).
- **Vn.3 Sign-in.** Vendor table row (Rust and Python, fixture), both ways
  of decision 3, probe "who am I".
- **Vn.4 Mock and gate step.** The vendor's routes in `saas_stub.py`
  behaving as the vendor (its paging shape, its error bodies, a 429, a
  changed field, a delete where it reports deletes); `g6_saas_test.py`
  steps connect, discover, load, reload (incremental), schema change,
  revoked sign-in.
- *Check per vendor:* pytest for the vendor module with recorded responses
  (no network); the gate's steps for the vendor pass in CI.

Slice 6 note: Workday's object is a report URL the user names (owner and
report), read with `single_page`; the page says it is re-read in full.

### Slice 7: supported, docs, measurement

- **T7.1 Migration `0096_saas_connectors_supported.sql`** flips the eight
  (not Workday, NetSuite) to `supported = true` with the guarded `UPDATE`
  shape; `coverage.json` entries with `gate: "g6-saas"` and all steps
  `real`, in the same commit. *Check:* SRC-9 lint.
- **T7.2 Measure.** `docs/plans/SRC-3-RESULT.md`: the `g6-saas` job time
  from three CI runs against the 20-minute budget; per vendor the rows and
  pages the gate loaded; what was not measured (real accounts).
- **T7.3 Docs.** `PRODUCT.md` (`:113`, `:115`, `:216`, `:351`),
  `competitive-comparison.md` (`:431`, `:433`, `:440`), feature pages
  `connector-gates.md`, `connectors-that-work.md`,
  `table-discovery-every-source.md`, `credentials-done-properly.md`
  (OAuth limits), `load-modes.md`, `source-schema-changes.md`;
  `user-guide.md:59`; README connection-test list; `coverage.json` `_about`;
  `NOTICE` (vendored Apache-2.0 code); `CHANGELOG.md`; BACKLOG `SRC-3`
  Building with #214 and the Dates row; new backlog items for the second
  wave by demand and anything found.

## 7. PR

Draft #214 from the first commit. CI on every push. Browser check by the
product owner once at the end on a QA instance (screens, the mock vendors
cannot be signed into there; real accounts per decision 11 for the
acceptance checklist). Merging is others' job.

## 8. Verify, do not assume

Each before the code that depends on it; a false one stops the task and is
reported here:

1. GA4's `BetaAnalyticsDataClient` runs with the REST transport and its
   HTTP goes through `getaddrinfo` (before Vn.1 of GA4).
2. `simple-salesforce` accepts our `requests.Session` for every call it
   makes, including Bulk API 1.0 (slice 3).
3. dlt 1.30.0's `rest_api` `header_link` and `json_link` hand the next URL
   to a session we control, so AD3 applies (slice 1, T1.8).
4. dlt's Iceberg merge strategy used by our sink (`upsert`) works with the
   vendor primary keys, and `delete-insert` is not needed (T1.9).
5. The SRC-9 lint grammar covers the CHECK widening (T1.1).
6. dlt resource state survives a failed run for delta links (slice 5).
7. Each new library's licence (Vn.1).

## 9. Out of scope

`SRC-4`; the second wave; writing back; SharePoint and Confluence files;
GA4 raw events; a no-code builder (decision 14); changing `rest.py`'s
rule Z5.

## 10. Rules for the whole phase

`AGENTS.md` in full. In particular: secrets only as `SecretValue` and
through the allowlisted resolver; no vendor error text in a response;
fixed vendor hosts in code, never from the user; no real account names,
hosts or tokens in code, fixtures or docs (rule 12); cite this plan's
decisions (`SRC-3 decision N`, `AD-N`) at the code that carries them.

## 11. Handoff

(Developer, per slice: commits, the exact commands with their counts, what
was not verified and why.)

### Handoff: slice 1 (developer, 2026-10-19)

Commits, in order (branch `feat/data-phase-4-connectors`, base `2ea0f377`):

| Task | Commit | Subject |
| --- | --- | --- |
| T1.1 | `832a5ed9` | SRC-3 feat(migrations): saas adapter and the ten business-app connector types, unsupported |
| T1.2 | `d9a7d72f` | SRC-3 feat(store): saas vendor table and Dial::Saas |
| T1.3 | `52d2a4a2` | SRC-3 feat(store): incremental loads and column choice for saas objects |
| T1.4 | `c0ea41d0` | SRC-3 feat(api): connection test for saas connectors |
| T1.5 | `2443e156` | SRC-3 feat(api): OAuth sign-in for saas connectors, with client_secret_basic |
| T1.6 | `e3b9d2b4` | SRC-3 test(api): saas listing and column preview go to the orchestrator |
| T1.7 | `fabbe751` | SRC-3 feat(ai): the assistant's connector tools know the saas adapter |
| T1.8 | `2e97023c` | SRC-3 feat(dagster): SaasSession, vendor registry and object declarations |
| fix (PR #214 CI) | `1988a26f` | SRC-3 test(store): connector_type tests know the ten saas types |
| T1.9 | `34120026` | SRC-3 feat(dagster): the saas load path, with a describe hook and incremental loads |
| T1.10 (Rust half) | `8d9d601d` | SRC-3 feat(store): discovery lists a business-app object with how it loads |
| T1.10 | `1e19ae75` | SRC-3 feat(dagster): discovery reads a saas connector through its vendor module |
| T1.11 | `00393bd2` | SRC-3 feat(frontend): business apps in the connector picker and forms |
| T1.12 | `c2e7a263` | SRC-3 ci(gate): the g6-saas job skeleton and the mock vendor server |

Commands run on the final tree, with counts:

- `cd rust && cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets --all-features -- -D warnings` (shared `CARGO_TARGET_DIR`, `-j 2`, crate sources touched first): finished, 0 warnings. (Only these two crates changed Rust.)
- `cd dagster && python -m pytest dispar_orchestrate -q` (whole suite, loader tests included, venv `~/.cache/rantai-dagster-venv`, dlt 1.30.0): **1746 passed, 0 failed** (16m08s).
- `python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`, `check_connector_gate_coverage.py`: all OK; `python3 -m unittest discover -s ops/lint -p 'test_*.py'`: OK.
- `bun run typecheck`: 8 errors, all `Cannot find module 'three'` / its follow-on under `src/features/landing/scene` (the checkout has no `node_modules` of its own; it uses the main checkout's through a symlink that is not committed, and that install lacks `three`); 0 errors anywhere else. `bun run lint`: 0 errors, 6 warnings, none in files of this slice.
- `bun test src/features/connectors`: 338 pass, 0 fail. `bun test src/lib/connectors`: 45 pass, 1 skip, 0 fail. `bun test src/services`: 85 pass, 0 fail.
- `bash scripts/ci/tests/test_required_gate.sh`: 52 passed, 0 failed.
- `docker compose --profile g6-saas config --quiet` (with the g6 override) and `--profile '*' config --quiet`: OK (fake `.env` values).

**Not verified, and why**

- **No Rust test was run** (`cargo test` is forbidden on this machine). Every Rust unit and integration test added or changed (`saas_vendors.rs`, `ingest_spec.rs` saas tests, `connector_probe/saas.rs`, `routes/connector_oauth.rs`, `connector_oauth.rs`, `tests/connector_oauth_flow.rs`, `tests/connector_discovery.rs`, `tests/connector_type.rs`, `tool_schemas_snapshot_is_byte_identical`) compiled under clippy `--all-targets` and was read, but did not run. `tests/fixtures/tool_schemas.json` was edited by hand to what the registry should print (two strings, one enum value), not regenerated. PR #214 CI's first run of T1.1 to T1.4 found the three `connector_type.rs` failures fixed in `1988a26f`; nothing of T1.5 onward has run in CI yet.
- **The `g6-saas` job and `g6_saas_test.py` have not run.** The job is copied from `g6-discovery-scale` and `g6-oauth`; the stub was started locally and answers `/health` and 404; the gate script was not executed against an API.
- **The Rust and Python account rules are separate code**, compared only through the vendor table fixture and each side's own tests; the shared fixture covers the table, not the account-validation functions. The near-miss lists in both languages' tests are the same on purpose.
- **Brand marks were not added** (T1.11): the current Simple Icons release could not be checked from this machine, which `brand-marks.ts:21-29` requires. All ten types show the generic icon.
- Nothing was shown in a browser.

**Plan items decided or found false (section 8)**

- Item 3 (dlt `rest_api` links): true by reading dlt 1.30.0; `RESTClient` takes a caller-supplied session and paginators set `request.url` to the next link, so a `Session.send` override can apply AD3. Not run with a vendor (slice 4).
- Item 4 (upsert merge on vendor keys, `_apply_cursor`): verified by running, in `test_adapters_saas.py` against a local Iceberg catalog with a real dlt resource: the second incremental run read only the newer rows, the merge left no duplicate, and `sink._apply_cursor` accepted the saas resource, so `sink.py` is unchanged.
- Item 5 (SRC-9 lint grammar): true; `0095` is in the shapes the lint replays and the lint needed no change.
- Items 1, 2, 6, 7: not reached in slice 1.

**Plan mismatches found (the planner decides)**

1. **Paths.** `ingest_spec.rs` is in `rust/crates/lakehouse-store/src`, not `lakehouse-api`. Because `Dial::parse` validates the account against the vendor table and the API crate depends on the store, `saas_vendors.rs` is in the store too (T1.2 said `A/saas_vendors.rs`).
2. **T1.9 asks `ingest_errors.classify` to gain `permission_missing` and `rate_limited`.** The run-report classes are a closed list enforced by `connector_run.rs` and by `CHECK connector_run_attempt_class_check` (`0072`); two new classes need a migration, and `0095`/`0096` are taken. Provisional mapping, in the code and the commit: `SaasSignInRefused` and `SaasPermissionMissing` classify as `sign_in_refused` (not retried), `SaasRateLimited` as `source_busy` (retried). Decision 7 of the feature page ("a permission is missing", "the vendor limits the rate" in plain words) is therefore NOT met for run reports until the classes exist. Discovery already has `no_permission`, and uses it.
3. **T1.10 and T1.11 assume the discovery result can list a business-app object and say how it loads.** `ObjectKind` is closed and `ListedObject` denies unknown fields, so a saas listing would have been refused. Added `ObjectKind::Object` and an optional `load` text (`8d9d601d`, additive, validated). This is Rust work outside the task text; revert it if the planner wants another carrier.
4. **`connector_type.rs` tests (`1988a26f`).** `unsupported_rows_have_no_adapter` guards "an unsupported type without a `Dial::parse` shape has no adapter"; the exceptions are named (`0062` Sheets, `0066` the two CDC types) because those types keep their adapter so existing connectors open in the edit page. The ten saas types are the same case, so they were added to the named list (not a wildcard), the count became 26, and a new test pins adapter `saas` plus the reason for each. AD10 is unchanged. If the planner prefers `adapter = NULL` until `0096`, `0095` must change instead (not applied, so no deployed database has it).
5. **Confluence.** With browser sign-in the Atlassian API host is `api.atlassian.com/ex/confluence/<cloud id>`, not `<site>.atlassian.net`. The table row uses the site host (right for an API token); slice 4 must add the cloud-id step.
6. **Dynamics and SharePoint client credentials need a Microsoft tenant id**, which no slot or dial field carries yet; slice 5.
7. **The vendor scopes and revocation addresses in the table are from the developer's knowledge of each vendor's documentation, not checked against it from this machine** (no network). Slices 2 to 6 check each when the vendor is built.

**Other**

- `rest.py` changed only to add `client_secret_basic` to the token request (AD8); a `rest` connector's request is unchanged (a `rest` test failed while `headers=` was always passed; fixed to pass it only when asked).
- Not changed: `rest.py`'s rule Z5, `sink.py`, `POLICY_TABLE` and `tests/route_auth.rs` (no route was added or widened).
- `src/features/connectors/connector-ingest-panel.test.tsx` has one expected sentence updated with the copy it checks ("SQL sources and business apps only").


### Handoff: slice 1 fix round (developer, 2026-10-19)

- **BLOCKER 1** fixed in `5f811fa6` (SRC-3 fix(runs): permission_missing and rate_limited are run-report classes): migration `0097_connector_run_attempt_saas_classes.sql` (DROP and ADD of `connector_run_attempt_class_check`, a statement shape no lint reads); `connector_run::ErrorClass::{PermissionMissing, RateLimited}` with fixed texts, `RateLimited` transient and `PermissionMissing` not; `ingest_errors` (`PERMISSION_MISSING`, `RATE_LIMITED`, `RETRIED_CLASSES` includes the second; `SaasPermissionMissing` and `SaasRateLimited` classify to them, `SaasSignInRefused` stays `sign_in_refused`); console contract and wording (`permission_missing`, `rate_limited`, and "Retries are used up" for a rate limit). The retry loop reads `classify().retried`, so the run's own delays apply to `rate_limited` with no change to `_fail_or_retry`. Discovery is unchanged (it has `no_permission` already).
  - Run: `cargo fmt --check` clean; `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` 0 warnings; pytest `test_ingest_errors.py`, `test_adapters_saas.py`, `test_ingest_run_reports.py`, `test_ingest_retries.py`, `test_discover.py`, `test_saas_session.py`: 214 passed; `bun test src/features/connectors/connector-ingest-panel.test.tsx`: 85 pass, 0 fail; `bun run lint` 0 errors; typecheck 0 errors outside `features/landing/scene`; `check_connector_gate_coverage.py` OK; no duplicate migration numbers.
  - **Not verified:** the new Rust test `the_two_saas_classes_are_stored_worded_and_retried_only_for_the_rate_limit` (DB round trip, and that a made-up word is still refused by the CHECK) compiled but did not run; nor did `0097` itself against a database.
- **SHOULD-FIX 2** not changed, and *not verified*: the repository's generator is the ignored Rust test `routes::ai::registry::tests::write_tool_schema_fixture` (`cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`), a `cargo test` run that this machine may not do. The hand edit stays (one enum value, one description string, both in `registry.rs` and in the fixture); `tool_schemas_snapshot_is_byte_identical` in CI is the check. If it fails, run the generator on a machine that can and commit the difference.

## 12. Review

### Review of slice 1 (planner, 2026-10-19, head `dcd07aa4`)

Read: the migration, the vendor table, `SaasSession`, the probe's order
(address check before any credential), the developer's five reported
mismatches. CI on `c0ea41d0` (T1.1–T1.4): 35 of 37 checks green; the three
`connector_type.rs` failures are fixed in `1988a26f`. The rest of the slice
has its first CI run on the push of this review.

Decisions on the reported mismatches:

1. **Run-report classes (T1.9).** `BLOCKER 1`: decision 13 asks for "the
   vendor limited the rate" and the page's item 7 for a plain message when
   a permission is missing; mapping them to `source_busy` and
   `sign_in_refused` does not say that. Add migration `0097` (reserved
   here) widening the run-report class CHECK of `0072` with
   `permission_missing` (not retried) and `rate_limited` (retried as
   `source_busy` is), the Rust enum and its fixed texts, the Python classes,
   and the console's wording; tests both ways.
2. **`ObjectKind::Object` and the `load` text** (`8d9d601d`): accepted, an
   additive, validated carrier; the plan's T1.10 is read to include it.
3. **`connector_type.rs`:** accepted. The ten types join the named
   exceptions as Sheets and the CDC types did, for the same reason (an
   existing connector opens in its edit page); no wildcard; AD10 unchanged.
4. **`saas_vendors.rs` in `lakehouse-store`:** accepted (that is where
   `ingest_spec.rs` lives).
5. **Confluence cloud id, Microsoft tenant id, scopes and revocation
   addresses:** each vendor's slice checks its row against the vendor's
   documentation and records the source in the commit (Vn.3).

`SHOULD-FIX 2`: `tests/fixtures/tool_schemas.json` was edited by hand;
regenerate it with the repository's generator and commit any difference.

Not verified by the developer and still open: every Rust test (CI),
the `g6-saas` job (CI), a browser look (end of phase), brand marks
(slice 7, against the current Simple Icons release).
