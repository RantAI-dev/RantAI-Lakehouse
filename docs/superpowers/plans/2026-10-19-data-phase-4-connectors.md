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

### Handoff: slice 2 (developer, 2026-10-19)

HubSpot, then Zendesk (V-HubSpot.1 to .4, V-Zendesk.1 to .4), one task per commit, in
that order. Base: `dc3373ae` (the slice 1 fix round).

| Task | Commit | Subject |
| --- | --- | --- |
| V-HubSpot.1 | `62c21ac3` | HubSpot verified source, copied unchanged |
| V-HubSpot.1 | `3e4fda0e` | `SaasSession` can POST a search request |
| V-HubSpot.1 | `1feaee7b` | vendored HubSpot reads only through `SaasSession` (local change 1) |
| V-HubSpot.1 | `f0f8ab7a` | HubSpot loads incrementally by last-modified time through the search API (local change 2) |
| V-HubSpot.2 | `00700ef5` | HubSpot objects, properties-API describe and reads |
| V-HubSpot.3 | `87a919bb` | HubSpot sign-in, both ways, and the checked vendor row (scopes) |
| V-HubSpot.4 | `9472fc0b` | mock HubSpot and the HubSpot steps of `g6-saas` |
| V-Zendesk.1 | `0a51a40a` | Zendesk verified source, copied unchanged |
| V-Zendesk.1 | `2d735595` | vendored Zendesk without Talk and Chat (local change 1) |
| V-Zendesk.1 | `1dabfe4b` | vendored Zendesk reads only through `SaasSession` (local change 2) |
| V-Zendesk.1 | `6bf27c37` | the duplicate of upstream #659 found and fixed (local change 3) |
| V-Zendesk.2 | `659d7979` | Zendesk objects, incremental reads and deletes |
| V-Zendesk.3 | `2510b78b` | Zendesk email + API token sign-in; browser sign-in blocked (below) |
| V-Zendesk.4 | `1a810245` | mock Zendesk and the Zendesk steps of `g6-saas` |

Both sources are copied from `dlt-hub/verified-sources` at one commit,
`3957506893a7da821dbcc6acd51c7ca4475d1f53` (2026-07-03), into
`dagster/dispar_orchestrate/saas/vendored/{hubspot,zendesk}/`, each with `UPSTREAM.md`
(repo, commit, date, every local change with its reason) and the Apache-2.0 licence.
The copy is its own commit and byte-identical. No new Python library: both sources use
only `dlt` (pinned) and the standard library, so no pin, note or licence entry was added
to `dagster/pyproject.toml`. `NOTICE` is not touched (slice 7, T7.3).

**What was found, per vendor**

- *The vendored code can be given our session* (the stop condition): both sources fetch
  through `dlt.sources.helpers.requests` in a few functions (`helpers.py`;
  `ZendeskAPIClient.get_pages`). Local change 1 of each replaces that by
  `SaasSession.get_json` (the `api_key` / credentials parameters became the session; a
  test fails if a vendored module imports an HTTP library). `SaasSession` gained
  `post_json` for HubSpot's search, with the same address, link, redirect and 429 rules.
- *HubSpot incremental* (`helpers.search_data_since`): the CRM search API, `GTE` on
  `hs_lastmodifieddate` (`lastmodifieddate` for contacts), ascending, `after` paging. PR
  #662 was read, not copied; differences, each for a limit HubSpot documents
  (developers.hubspot.com/docs/api/crm/search): the 10,000-result cap restarts from the
  newest time seen and raises `SearchOutOfBoundsException` (the module then reads the
  object in full) when 10,000 records share one instant; properties beyond the
  3,000-character body are batch-read by id; a 60 s lag before the cursor because HubSpot
  says an update can take "a few moments" to be searchable. First run lists, later runs search.
- *HubSpot deletes (decision 7)*: HubSpot's documentation says archived records "won't
  appear in any search results"; the archived listing (`archived=true`) is a separate full
  read. So no HubSpot object claims `deletes`, and a record archived at HubSpot stays in Bronze
  until a full reload. The feature page's "Limits" list (deletes only for Salesforce,
  Zendesk, SharePoint, Dynamics) is therefore right and needs no change.
- *HubSpot describe (decision 9)*: the properties API, before the load; `None` for owners and
  pipelines, which have no such API (the adapter and discovery take the after-load path).
- *Zendesk duplicate (#659), cause*: `ticket_table` read the time-based export; such a page
  ends at the `generated_timestamp` of its last item and the next page starts AT that time,
  so the tickets sharing it are sent again (Zendesk documents it). In `ticket_events`
  (`append` + primary key) the repeat is a stored row. *Fix*: tickets and users use the
  cursor-based export (no repeat within an export); ticket events and organizations have only
  the time-based one, so `get_pages(distinct_by=...)` drops an item with an already-seen
  `id` + `created_at` / `updated_at`, Zendesk's own documented rule. The test shows
  `[1, 2, 3, 3, 4, 5, 5, 6]` on the vendor's time-based endpoint and `[1, 2, 3, 4, 5, 6]` after.
- *Zendesk deletes*: a deleted ticket stays in the export with `status = "deleted"`; only
  tickets say `deletes` (the row is marked, not removed: Zendesk scrubs content after 30 days
  and drops the record 90 days after a permanent deletion).
- *Zendesk objects*: tickets, users, organizations, ticket events, ticket metric events
  incremental; the verified source's other Support objects (31 in all) re-read in full with
  the reason on the page. Custom fields are not pivoted (a pivot names columns after titles an
  admin can rename and needs the source's state); `tags` and `custom_fields` are one JSON text.
- *Vendor-table rows checked against vendor documentation (review item 5)*, URLs in the commit
  bodies of `87a919bb` (HubSpot) and `2510b78b` (Zendesk). HubSpot's scopes were incomplete
  (only `crm.objects.*.read` and `tickets`); corrected in Rust, the fixture, Python and the
  console to add `crm.objects.quotes.read`, `crm.schemas.{companies,contacts,deals,quotes}.read`,
  `e-commerce` and `oauth`. The scope names were read on HubSpot's reference pages; which
  endpoint needs which is read from those pages, not tried on an account.

**Plan mismatches found (the planner decides)**

1. **Zendesk's browser sign-in cannot work with SRC-12's token request.** `POST /oauth/tokens`
   "must be formatted as JSON" (developer.zendesk.com/api-reference/ticketing/oauth/grant_type_tokens/);
   the product's token request (Rust `connector_oauth::token_request` for the callback and the
   probe, `rest._token_request` in Python) is form-encoded, and the vendor table's `clientAuth`
   (`post` / `basic`) cannot say "JSON". Zendesk's refresh token also rotates (handled by the
   existing rotated-token report). Not built, as instructed: the fix is a third method
   (`post_json`) in three Rust call sites (`routes/connector_oauth.rs`, `connector_oauth.rs`,
   `connector_probe/saas.rs`), `rest._token_request`, the fixture, the Python and Rust tables and
   the console mirror, which this machine cannot test (no `cargo test`). The email + API token way
   is built. A strict `xfail` test (`test_the_browser_sign_in_sends_zendesks_token_request_as_json`)
   records the gap and turns red when it is fixed; the gate logs "NOT RUN" with the reason.
   Decision 3 ("extended for that vendor, not dropped") needs the planner to schedule that Rust work.
2. **The vendor hosts are fixed, so a gate cannot point a vendor at the mock.** Added
   `rebase_for_gate` (`saas/vendors/__init__.py`): a gate-only `SAAS_GATE_ORIGINS` that does nothing
   unless `CONNECTOR_OAUTH_ALLOW_HTTP=true`, names the vendor, and holds a bare `http(s)://host:port`
   (tested, falls back to the real host on anything else). It goes round "hosts come only from the
   table" (AD2), so it is commented as such at the site; the planner should confirm that is acceptable.
   The mock gives each vendor its own port and serves the vendor's real paths.
3. **Gate and unsupported types.** Nothing refused a connector of an unsupported type (the API
   lists and creates them; the SRC-9 lint reads migrations only). The vendor steps do not create a
   connector at all: they drive the vendor module and `SaasSession` in the runner (the Dagster image)
   against the mock. So the gate proves the vendor code over HTTP; it does not go through the API's
   connector routes (the Rust connection test has wiremock tests) or the Iceberg sink (pytest `loader`
   tests do that). The API step still asserts the ten types are `supported: false`.
4. **Small changes outside the task text, all in Python:** the registry now imports a vendor module
   by path on first use (`MODULE_PATHS`; the "no module" tests use NetSuite); `describe` may answer
   `None` (adapter and discovery handle it); `ObjectDecl`/adapter unchanged otherwise.
5. **HubSpot API version.** The code uses `/crm/v3/...` as the vendored source does. HubSpot's current
   documentation shows date-versioned paths (`/crm/objects/2026-09/...`) and I did not find a sunset
   date for `v3`; not verified.

**Commands run, with counts**

- `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q` (whole suite,
  loader tests included, on the final tree): **1795 passed, 1 xfailed, 0 failed** (16m23s). The one
  xfail is the strict Zendesk browser sign-in test above. New files: `test_saas_vendored_hubspot.py` (8),
  `test_saas_hubspot.py` (15), `test_saas_vendored_zendesk.py` (7), `test_saas_zendesk.py` (15 + 1 xfail);
  `test_saas_session.py` +1, `test_saas_vendors.py` +1, `test_adapters_saas.py` +1, `test_discover.py` +1.
- `python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`,
  `check_connector_gate_coverage.py`: all OK; `python3 -m unittest discover -s ops/lint -p 'test_*.py'`: 23 tests OK.
- The gate's vendor steps, run for real against the stub (`python3 ops/g6/saas_stub.py` on this machine,
  then `python g6_saas_test.py --vendors-only` with the checkout's `dispar_orchestrate` first on
  `PYTHONPATH`): 15 `[saas]` lines then `PASS (11 s)` (HubSpot 7 lines, Zendesk 8 lines, one of them the
  recorded "NOT RUN").
- The stub image built and run: `docker build -f ops/g6/Dockerfile.saas_stub` then `docker run`; `/health`
  answered `{"status": "ok", "vendors": ["hubspot", "zendesk"]}` and a HubSpot route answered 200.
- `docker compose --env-file <fake values> -f docker-compose.yml -f ops/g6/docker-compose.g6.override.yml
  --profile g6-saas config --quiet`: OK (the runner now carries the two gate-only switches).
- `bun test src/lib/connectors`: 45 pass, 1 skip, 0 fail. `bun test src/features/connectors`: 339 pass, 0 fail.
  `bun x eslint src/lib/connectors/saas-vendors.ts`: clean. `bash scripts/ci/tests/test_required_gate.sh`: 52 passed.
- `cd rust && cargo fmt --check`: clean.

**Not verified, and why**

- **No Rust was compiled or tested.** The only Rust change is the HubSpot scope list in
  `lakehouse-store/src/saas_vendors.rs` (a string array). `cargo clippy -p lakehouse-store` did not run:
  another session had `cargo test` running and `df -h /` showed 6.2 GB free (below the 8 GB rule). `cargo fmt --check`
  passed. The Rust vendor-table test that compares with `ops/fixtures/saas_vendors.json` (CI) is the check that the
  Rust and the fixture scopes agree; the Python (`test_saas_vendors.py`) and console (`saas-vendors.test.ts`)
  comparisons passed.
- **The `g6-saas` job in CI has not run** with these steps: the steps ran against the stub on this machine, not in the
  compose stack with the API, and its time against the 20-minute budget is unmeasured.
- **No real HubSpot or Zendesk account** (decision 11): every vendor shape is from the vendor's documentation, modelled in
  the mock. In particular the exact scope-to-endpoint mapping for HubSpot, that Zendesk's cursor export accepts
  `per_page`, and Zendesk's behaviour when more items share one timestamp than a page holds, are from documentation
  and not tried.
- **Zendesk browser sign-in** and the HubSpot revoke call (`DELETE /oauth/v1/refresh-tokens/{token}`, not RFC 7009) are
  not exercised; the table keeps no revocation address for either.
- The unit tests of the Rust `Dial::Saas` account rules and the probe are unchanged and were not run.
- Nothing was shown in a browser.

### Handoff: slice 2 fix round (developer, 2026-10-21)

Findings of the slice 2 review (commit `95199be0`), one commit each, in the order asked
(Python, console and compose first; Rust last). Disk was below 8 GB all round, so **no cargo
build, check, clippy or test ran**; the Rust was written and checked with `cargo fmt --check` only.

| Finding | Commit | Subject |
| --- | --- | --- |
| SHOULD-FIX 6, SHOULD-FIX 7 | `f5964d7f` | HubSpot v3 note in `UPSTREAM.md`; the connector page says a deleted Zendesk ticket stays with `status = deleted` and HubSpot reports no deletes |
| BLOCKER 3 | `f3e0668c` | Zendesk browser sign-in through `client_secret_post_json` (table value `post_json`) in Rust, Python, fixture, console; the xfail is a real test; the gate's Zendesk sign-in step runs against a mock `/oauth/tokens` that wants JSON and rotates the refresh token |
| SHOULD-FIX 5 (Rust half) | `af60d493` | `connector_oauth::gate_rebase`: the API's saas probe, save-time address check and sign-in callback use the gate-only mock origin, under the same switch, with a test |
| BLOCKER 4 (prerequisite) | `4afd8a24` | the API's connection test learns Zendesk's email + API token (Basic with `<email>/token`); without it the real `connect` step answered "not testable" |
| BLOCKER 4 | `c427ddf0` | `g6-saas` through the product, once for both vendors |

- **SHOULD-FIX 5, Python half**: already in `slice 2` (`test_the_gate_rebase_does_nothing_unless_the_gate_only_switch_and_the_vendor_are_named`:
  switch off, switch not exactly `true`, other vendor, seven odd values); the Rust twin test is
  `the_gate_rebase_is_ignored_unless_the_gate_only_switch_is_on_and_the_vendor_named`.
- **BLOCKER 3 detail**: the new method is serialised `post_json` (like `post`, `basic`); the Rust arms are the
  callback exchange, the probe's refresh and `revoke_request`; `rest._token_request(as_json=...)` is used when the
  row says `post_json`. Zendesk is the only vendor on it (a Rust test and a Python test say so). Cited:
  developer.zendesk.com/api-reference/ticketing/oauth/grant_type_tokens/.
- **BLOCKER 4 detail**: `PRODUCT_VENDORS` in `ops/g6/g6_saas_test.py` holds one entry per vendor (HubSpot, Zendesk);
  `step_product` does create (API), connection test, discovery (list and preview through the orchestrator), load
  (`ingest_job` into Bronze, read back through ClickHouse), reload (changed, new and deleted/archived records, merged on
  the key, no duplicate) and a revoked sign-in (API test and run fail, none of the mock's words). The vendor-level steps
  stay. To let the product reach the mock: `ops/g6/docker-compose.g6-saas.override.yml` (this job only) sets
  `CONNECTOR_OAUTH_ALLOW_HTTP` and `SAAS_GATE_ORIGINS` on `lakehouse-api` and `dagster-code-location`; the
  runner gets ClickHouse, Dagster and the credential volume; the CI job now runs `--profile dagster` with the four
  Dagster services and a 25-minute cut-off (as `g6-ingest-other`). `docs/CI.md` describes it.

**Commands run, with counts**
- `cd dagster && python -m pytest dispar_orchestrate -q` (whole suite, once, final tree): **1797 passed, 0 failed**
  (15m57s; the strict xfail is gone).
- `python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`, `check_connector_gate_coverage.py`: OK.
- `bun test src/lib/connectors`: 45 pass, 1 skip, 0 fail. `bun test src/features/connectors`: 339 pass, 0 fail.
- `bash scripts/ci/tests/test_required_gate.sh`: 52 passed. `python3 -c yaml.safe_load` of `ci.yml`: parses.
- `docker compose --env-file <fake values> -f docker-compose.yml -f ops/g6/docker-compose.g6.override.yml -f
  ops/g6/docker-compose.g6-saas.override.yml --profile g6-saas --profile dagster config --quiet`: OK.
- The vendor-level gate steps against the local stub (`g6_saas_test.py --vendors-only`): `PASS`, including the Zendesk
  browser sign-in with a JSON token request and a rotated refresh token.
- `cd rust && cargo fmt --check`: clean.

**Not verified, and why**
- **All Rust of this round** (`PostJson` in `saas_vendors.rs`, `token_request_json`/`post_fields`, the callback, probe and
  revoke arms, `gate_rebase`, the Zendesk Basic probe and the three new Rust tests): formatted, **not compiled, not tested**.
  CI compiles and runs them. The existing probe test for "a pair is not testable" was moved from Zendesk to ServiceNow
  because Zendesk's pair is now tested.
- **The product steps of the gate and the whole `g6-saas` job have never run** (no docker stack on this machine now);
  its time against the 20-minute budget is unmeasured. Things CI may show that I could not: whether
  `POST .../ingest/run` accepts a connector of a type that is `supported = false` (the review says creation is not
  refused; the run was not checked), the exact JSON field names ClickHouse returns for the Bronze columns, and whether the
  Zendesk `custom_fields`/`due_at` columns load. If the job cannot fit 20 minutes it will say so in the summary.
- **Browser sign-in through the product** (the API's callback exchange and refresh with the stored refresh token) is not
  exercised: it needs a browser, and a `file:` secret cannot hold a rotated refresh token. The Rust tests and the
  vendor-level steps cover the token request itself.
- Nothing was shown in a browser.

### Handoff: slice 3 (developer, 2026-10-21)

Salesforce, then Google Analytics 4 (V-Salesforce.1 to .4, V-GA4.1 to .4), one task per commit,
in that order. Base: `45bb3c42`. Disk was below 8 GB, so **no cargo build, check, clippy or test
ran and no image was built**; the Rust was written and checked with `cargo fmt --check` only.

| Task | Commit | Subject |
| --- | --- | --- |
| V-Salesforce.1 | `ec8d5a76` | Salesforce verified source, copied unchanged |
| V-Salesforce.1 | `32f6c383` | reads only through `SaasSession`; no `simple-salesforce`, no password sign-in (local change 1) |
| V-Salesforce.1 | `dc0e2617` | `Id` is the primary key of every resource (local change 2, upstream #692) |
| V-Salesforce.1 | `89623e8d` | deleted records through `queryAll`; cursor `>=` (local change 3) |
| V-Salesforce.2 | `06af7bd5` | `queried_fields` exposed (local change 4, no behaviour change) |
| V-Salesforce.2 | `dcafb5c0` | objects (custom ones too), incremental, deletes, describe |
| V-Salesforce.3 | `07cdde7f` | browser sign-in; `instance_url` adopted through `adopt_origin`; whoami moved |
| V-Salesforce.4 | `a3d2ebcd` | mock Salesforce and the Salesforce steps of `g6-saas` |
| V-GA4.1 | `59d57872` | Google Analytics verified source, copied unchanged |
| V-GA4.1 | `493a5003` | REST endpoints through `SaasSession`; no Google client (local change 1) |
| V-GA4.1 | `f9d9e110` | merge on the dimensions, lookback, no empty-range request (local change 2) |
| V-GA4.2 | `ea1fcc9a` | reports as objects, incremental by the date dimension, `ObjectDecl.lag_seconds` |
| V-GA4.3 | `2dabb179` | browser sign-in and service-account key; the Google row fixed (`access_type=offline`) |
| V-GA4.4 | `05560cfa` | mock GA4 and the GA4 steps of `g6-saas` |

Both sources are copied from `dlt-hub/verified-sources` at `3957506893a7da821dbcc6acd51c7ca4475d1f53`
into `saas/vendored/{salesforce,google_analytics}/`, each with `UPSTREAM.md` and the Apache-2.0 licence,
the copy its own commit. **No new Python library**, so no pin, note or licence entry in `pyproject.toml`
(`cryptography==50.0.2` was already pinned and signs the GA4 assertion).

**Plan section 8, verified**

- *Item 2 (`simple-salesforce`)*: true for the library, false for our purpose. In 1.12.10 (Apache-2.0) every call,
  Bulk API included, is `session.request` through `call_salesforce`, so it accepts a `requests.Session`. But that is
  a plain `requests` session (redirects followed, no address check, no pinning, no bounded 429 wait), and Bulk 1.0 is
  XML/CSV job traffic that `SaasSession` (JSON) cannot carry. **Chosen**: replace the library's HTTP with `SaasSession`
  calls (REST describe, REST `query` / `queryAll` paged by `nextRecordsUrl`, same origin); the library is not added.
- *Item 1 (`google-analytics-data`)*: 0.23.3 (Apache-2.0) has a REST transport on `google.auth.transport.requests.AuthorizedSession`,
  so its HTTP does go through `getaddrinfo` (the gRPC one does not). But it is Google's own session (redirects, no address check
  before the credential, own token refresh) and the package requires `google-api-core`, `google-auth`, `grpcio`, `proto-plus`,
  `protobuf`. **Chosen**: `runReport` and `getMetadata` through `SaasSession.post_json` / `get_json`; nothing added.
  The wheel was read, never run.

**What was found, per vendor**

- *Salesforce object model*: describe-global lists the queryable sObjects (custom `Foo__c` included, decision 5); `SystemModstamp` is
  the cursor where the object has it, `IsDeleted` makes it read through `queryAll` (marked, not removed; a purged record is not reported);
  describe answers the exact list the load selects. Objects without a modstamp are re-read in full with the reason.
  Decision 3/4: Salesforce has **no integration credential** (the page's second-way list does not name it) and the SOAP sign-in is not offered.
- *Upstream defects fixed in the copy*: no primary key (#692); `>` on the cursor lost same-millisecond records; the SOQL spliced the cursor
  text; blob fields in a batch query; Bulk API fallback (gone).
- *`instance_url` (AD3)*: `open_session` asks the first token for a vendor that names `adopt_suffixes` (Salesforce only) and adopts the
  returned host through `SaasSession.adopt_origin` and the `.salesforce.com` / `.force.com` rule before any read. A refused sign-in
  therefore surfaces at open for Salesforce, at the first read for the others. `adopt_origin(as_origin=)` lets the gate dial its mock after the
  real host passed the same rule.
- *GA4 objects*: a report is `label = dimension, dimension | metric, metric` (the dial's `objects` and the source object's name); the time
  dimension is the cursor, the dimensions the key. Columns are typed from `getMetadata` before the load.
- *GA4 upstream defects fixed*: `append` with no key (rerun stored twice); the last days (Google: "Data processing can take 24-48 hours") never
  re-read; a start after yesterday asked Google for an empty range. Now `merge` on the dimensions, a 3-day lookback moved to the start of a week/month/year
  for coarse time dimensions, and no request for an empty range.
- *Found by a loader test*: `sink._apply_cursor` replaced the resource's own `dlt.sources.incremental(...)` and so **dropped any lag**; dlt then
  filtered out the re-read rows. `ObjectDecl.lag_seconds` (new, default 0) reaches dlt's `incremental(lag=)` through `adapters/saas.py`, and
  `_apply_cursor` keeps it from `resource.cursor_lag_seconds` (absent for `sql_database`: unchanged). This is a small edit in shared code
  (`adapters/sink.py`); the whole `test_sink*.py` and `test_adapters_saas.py` suites pass with it.
- *Vendor rows checked against official documentation* (URLs in the commit bodies): **GA4**: addresses, scope, revocation and PKCE confirmed;
  **the row was wrong**: Google returns a refresh token only with `access_type=offline` (and again only with `prompt=consent`), so the authorise
  address now carries both, in Rust, fixture, Python and console. Service-account assertion (RS256, claims, one hour) confirmed.
  **Salesforce**: `developer.salesforce.com` answered 403 and `help.salesforce.com` serves an empty script shell to this machine, so the
  addresses, scopes `api refresh_token` and `client_secret_post` are **not re-verified** (kept as slice 1 wrote them). The whoami moved from
  `/services/oauth2/userinfo` (an identity-scope endpoint, scopes `id`/`openid` that the row does not request; also not verified in the docs)
  to `/services/data/v62.0/limits` (needs only `api`) in all four places. The acceptance row `SRC-3-AC1` on a real org is the check.

**Plan mismatches / decisions for the planner**

1. **Salesforce's API quota is HTTP 403 `REQUEST_LIMIT_EXCEEDED`**, as far as the developer knows (not verified: docs unreadable). `SaasSession`
   maps 403 to `SaasPermissionMissing`, so an exhausted quota would be reported as "permission missing" and not "the vendor limited the rate"
   (decision 13). The mock uses the generic 429 + `Retry-After`, as AD9 asks. Needs a decision (a Salesforce-specific body check in the session).
2. **No console field defines a GA4 report.** The dial form holds `account` and the sign-in only; `dial.objects` is where reports live, and nothing
   in the console writes it. Rust accepts the field. A GA4 connector cannot be set up from the UI until a form field exists (not in Vn.1 to Vn.4).
3. **The API's connection test cannot present a service-account key** (the probe answers `supported: false`, as for any non-token integration
   credential). A load reports whether it worked. Rust probe work was out of the constraints of this slice.
4. **A Salesforce query longer than 12,000 characters** (an sObject with very many fields) is a fixed failure: the query travels in the GET address
   (Salesforce refuses beyond 16,384 bytes) and `SaasSession` has no POST-for-query. Column choice does not shorten it (it is applied after the read).
5. **GA4's "yesterday" is UTC**, as upstream had it, while Google's `dateRanges` are in the property's time zone.
6. Salesforce API version is the constant `62.0` (`settings.API_VERSION`, also in the whoami path); not checked against Salesforce's retirement list.
7. `rest.AuthSession` gained an `instance_url` attribute (set from a token answer; a `rest` connector never reads it).

**Commands run, with counts**

- `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q` (whole suite, once, on the final tree, loader tests
  included): **1882 passed, 0 failed** (16m33s); the slice 2 baseline was 1797. New files: `test_saas_vendored_salesforce.py` (15),
  `test_saas_salesforce.py` (23 cases), `test_saas_vendored_google_analytics.py` (16), `test_saas_ga4.py` (30 cases);
  `test_saas_session.py` +1.
- `python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`, `check_connector_gate_coverage.py`: all OK;
  `python3 -m unittest discover -s ops/lint -p 'test_*.py'`: 23 tests OK.
- The gate's vendor steps against the local stub (`python3 ops/g6/saas_stub.py`, then `g6_saas_test.py --vendors-only`, the checkout's
  `dispar_orchestrate` first on `PYTHONPATH`): 32 `[saas]` lines, `PASS (22 s)` (HubSpot 7, Zendesk 8, Salesforce 8, GA4 8). The stub answers `/health` with
  hubspot, salesforce, zendesk (and ga4).
- `bun test src/lib/connectors`: 45 pass, 1 skip, 0 fail. `bun test src/features/connectors`: 339 pass, 0 fail.
  `bash scripts/ci/tests/test_required_gate.sh`: 52 passed.
- `docker compose --env-file <fake values> -f docker-compose.yml -f ops/g6/docker-compose.g6.override.yml -f ops/g6/docker-compose.g6-saas.override.yml
  --profile g6-saas --profile dagster config --quiet`: OK.
- `cd rust && cargo fmt --check`: clean.

**Not verified, and why**

- **All Rust of this slice** (two strings in `lakehouse-store/src/saas_vendors.rs`: Salesforce's whoami path and Google's authorise address): formatted,
  **not compiled, not tested**. The CI test that compares the Rust table with `ops/fixtures/saas_vendors.json` is the check; the Python
  (`test_saas_vendors.py`, plus the two vendors' row tests) and console (`saas-vendors.test.ts`) comparisons passed.
- **The product steps of `g6-saas` for Salesforce and GA4 have never run** (no docker stack here), nor has the job's time against the 20 minutes. What CI
  may show that I could not: whether the API accepts an OAuth connector whose client secret and refresh token are `file:` secrets created through the
  generic `credential` slots (`secret_key` / `token`), whether `PUT .../ingest-spec` accepts the GA4 report text as a source-object name, whether the Rust
  probe reaches the mock for the Salesforce `limits` path and GA4 `metadata` path (`gate_rebase`), and the ClickHouse column names (`id`, `name`,
  `is_deleted`, `sessions_integer`, `date`) and the JSON type of a Bool column.
- **No real Salesforce org or GA4 property** (decision 11): every shape is from the documentation I could read (GA4) or from the library and my knowledge
  (Salesforce), modelled in the mocks.
- **Salesforce's official documentation** (see above) and its quota status code.
- The GA4 gRPC/REST transport claim was read in the wheel, not run (the package is not installed).
- Nothing was shown in a browser.

### Handoff: slice 3 fix round (developer, 2026-10-21)

Findings of the slice 3 review (commit `0e04656d`), one commit each. No cargo build, check, clippy or test ran and no image
was built (disk 2.3 to 5 GB free); the Rust was written and checked with `cargo fmt --check` only.

| Finding | Commit | Subject |
| --- | --- | --- |
| BLOCKER 8 | `9ebf45e5` | Salesforce's 403 `REQUEST_LIMIT_EXCEEDED` is `rate_limited`, not retried in the run |
| SHOULD-FIX 11 | `d6242106` | GA4 reads up to `yesterday` in the property's time zone |
| SHOULD-FIX 12 | `82346962` | Salesforce and GA4 limits on the connector page |
| BLOCKER 9 | `47488264` | the GA4 report editor in the console, fields through discovery |
| BLOCKER 10 | `57b41000` | the API's connection test of GA4's service-account key; the key in the GA4 product steps |

- **BLOCKER 8**: a vendor module may name `is_quota_exhausted(body)`; `SaasSession` gives it the 403 body and raises
  `SaasQuotaExhausted` (a `SaasRateLimited`), which `ingest_errors.classify` reports as `rate_limited` with `retried=False`
  (a per-24-hours quota is not waited out in minutes). Any other 403 is still `SaasPermissionMissing`. Salesforce's limits page
  stays unreadable from this machine (HTTP 403), so the code `REQUEST_LIMIT_EXCEEDED` is the reviewer's and the developer's;
  `SRC-3-AC1` on a real org is the proof. Mock route `quota`, vendor-level gate step "salesforce quota".
- **SHOULD-FIX 11**: no Admin API call was needed. The Data API's `DateRange` reference says `yesterday`, `today` and `NdaysAgo`
  are "inferred based on the property's reporting time zone", so the request's end is the term `yesterday` (same host, same scope,
  no stored time zone). No wider lookback and no stated limit were needed; the connector page says the dates are in the property's zone.
- **SHOULD-FIX 12**: "Limits to tell a customer" states the Salesforce query-length failure, API version 62.0, the recycle-bin purge
  and the daily limit not retried the same day, and GA4's three re-read days.
- **BLOCKER 9**: the report editor is `ga4-report-editor.tsx`, shown on the connector's tables card for a GA4 connector in place of
  "Add by name". The fields come through discovery: `POST .../discovery/list` with `schema: "fields"` reaches the orchestrator, whose
  saas reader calls the vendor module's `list_fields` (GA4: `getMetadata`, names only) and answers objects of kind `dimension` and `metric`
  (two new `ObjectKind` variants in Rust and in the contract). The date dimension is always included and shown as the cursor; a chosen
  time dimension is the cursor instead. The report text (`label = dims | metrics`, `src/lib/connectors/ga4-report.ts`, the server's
  `parse_report` is the authority) is saved into `dial.objects` and ticked as an `incremental_dedup` object keyed by its dimensions.
  The GA4 product step also lists the fields through discovery.
- **BLOCKER 10**: `service_account_access` in `connector_probe/saas.rs`: the key file is resolved after the address check, the key stays a
  `SecretValue`, the RS256 assertion is signed with the workspace `jsonwebtoken` 11 (no new crate; `aud` is the vendor table's token address,
  never the key file's `token_uri`), and the exchange goes through `connector_oauth::token_request`. A file that is not a key is a fixed message.
  `g6-saas` has a second GA4 product entry, `ga4-key`, with a key made for the step and verified by the mock's RS256 check.

**Commands run, with counts**
- `cd dagster && python -m pytest dispar_orchestrate -q` (whole suite, once, final tree; `df -h /` showed 2.3 GB, above the 2 GB rule):
  **1886 passed, 0 failed** (16m38s); slice 3 baseline 1882.
- `python3 ops/lint/check_intra_package_imports.py` and `check_connector_gate_coverage.py`: OK.
- `bun test src/lib/connectors`: 49 pass; `bun test src/features/connectors`: 348 pass (new: `ga4-report-editor.test.tsx` 6,
  `ga4-ingest-panel.test.tsx` 3, `ga4-report.test.ts` 4); `bun test src/services`: 85 pass; `bun run typecheck`: no error outside
  `features/landing/scene`; `bun run lint`: 0 errors.
- Gate vendor steps against the local stub after BLOCKER 8 and SHOULD-FIX 11: `PASS (22 s)`.
- `cd rust && cargo fmt --check`: clean.

**Not verified**
- **All Rust**: the two `ObjectKind` variants and the whole of `service_account_access` with its two tests (the signed assertion decoded with the public
  half; a non-key file) are formatted, not compiled, not run. In particular the `jsonwebtoken` 11 calls (`EncodingKey::from_rsa_pem`,
  `encode`, `Validation::set_audience`, `DecodingKey::from_rsa_pem`) and the `rsa` 0.9 dev-dependency in the test follow the existing
  `routes/auth.rs` test but were not compiled.
- The new `ga4-key` and discovery-fields product steps, and the whole `g6-saas` job, did not run (no docker stack).
- The report editor was not shown in a browser.
- Salesforce's own documentation, still unreadable from this machine.

### Handoff: slice 4 (developer, 2026-10-21)

ServiceNow, then Confluence (Vn.1 to Vn.4), base `7ddb2be8`. Disk was 2.2 to 2.3 GB: **no cargo build, check,
clippy or test ran and no image was built**; Rust was checked with `cargo fmt --check` only.

| Task | Commit | Subject |
| --- | --- | --- |
| V-ServiceNow.1 | `df7544f9` | Table API through `SaasSession`, `Link` header paging, `sys_updated_on` cursor, client credentials; `SaasSession.get_json_with_next` |
| V-ServiceNow.2 | `fab36244` | objects (suggested + by name), declaration from a sample record, describe via `sys_dictionary` |
| V-ServiceNow.3 + Confluence row | `35c632f3` | vendor-table rows (fixture, Rust, Python, console), probe for ServiceNow client credentials and Confluence Basic |
| V-ServiceNow.4 | `95de67b7` | mock ServiceNow, vendor-level and product gate steps |
| V-Confluence.1 to .3 (Python) | `db3757f1` | objects, cursor paging, newest-first incremental, both sign-ins, gateway cloud id (`AccountParts.base_path`, `oauth.apiBase`) |
| V-Confluence.4 | `12f0f9cf` | mock Confluence, vendor-level and product gate steps (`max_reads`) |
| docs | `34371707` | limits on the feature page, `docs/CI.md` |

**Choice asked for (plan section 8 item 3).** Both vendors call their endpoints through `SaasSession` directly (as slice 3),
not a dlt `rest_api` source: dlt's `RESTClient` takes a `requests.Session`, but the address check and pinning before each request,
credentials only after it, no redirects, the bounded `Retry-After` and the same-origin link rule are in `SaasSession._send`. The records go to the usual dlt resource. No library added.

**Found against the vendors' documentation (URLs in the commit bodies)**
- *Confluence row was wrong in four ways*: the token endpoint takes JSON (`clientAuth` -> `post_json`); the authorise address needs `audience=api.atlassian.com&prompt=consent`; `read:blogpost:confluence` does not exist (v2 OpenAPI scopes are `read:space|page|comment:confluence`; blog posts use `read:page`); a browser token is accepted only at `api.atlassian.com/ex/confluence/{cloudId}`. The last is a new optional `oauth.apiBase` in the table (fixed host, AD2); the cloud id comes from `accessible-resources`, matched to the site address, validated as a UUID, stored in the new `AccountParts.base_path`. The rotating refresh token uses the existing report path (tested).
- *ServiceNow row*: addresses and the client-credentials grant (form body) confirmed; revocation is `GET oauth_revoke_token.do?token=`, not RFC 7009, so the table's revocation address is dropped (as Zendesk's).
- *Confluence incremental (decision 6)*: `/pages`, `/blogposts` and both comment lists sort by `-modified-date` (OpenAPI `PageSortOrder`, `BlogPostSortOrder`, `CommentSortOrder`), so the read walks newest first and stops at the first record older than the cursor; sound because the order is by the cursor's own field. Spaces have no sort/filter by time and are re-read in full.
- *ServiceNow paging*: by the `Link` header, not offset, because `sysparm_limit` is applied before the access check (a short page is not the last). 
- *Loader test finding*: dlt cannot apply a lag to a string cursor (`IncrementalCursorInvalidCoercion`), so both vendors load their change time as a timestamp.

**Plan mismatches / decisions for the planner**
1. **ServiceNow date literal time zone**: the Table API page does not say whether a date in `sysparm_query` is UTC or the user's zone; the cursor therefore lags one day (`LAG_SECONDS`) and the merge removes repeats. Confluence's 300 s lag is our margin, not a documented figure. Both on the feature page.
2. **No deletes claimed** for either (decision 7): `sys_audit_delete` was not tried (access unknown); Confluence's trashed content just drops out of the listing.
3. **ServiceNow describe** needs read access to `sys_dictionary`/`sys_db_object`; with 403 it answers `None` (after-load path). Unknown whether ordinary integration roles can read them.
4. **API connection test for Confluence browser sign-in is `supported: false`** ("not testable", nothing sent): it would need the cloud-id request in Rust. The browser way is proved at vendor level only; the product steps use email + API token. Rust work to add if the planner wants it.
5. **ServiceNow user + password** (named in the brief as an alternative) is not built: the signed page (decision 3) names client credentials for ServiceNow, and the table's integration slots are `clientId`/`clientSecret`.
6. `integration_auth` now receives `parts=` (GA4 ignores it). `AccountParts` gained `base_path`. `get_json_with_next` is new on the session.
7. Which form Atlassian sends for `_links.next` behind the gateway (with or without the `/ex/confluence/<id>` prefix) was not observed; both handled and tested, the mock models the prefix-less one.
8. Confluence bodies (storage format) are loaded for pages, blog posts and comments; attachments are not.
9. The `35c632f3` commit holds the ServiceNow row work and the Confluence row work together (same files).

**Commands run, with counts**
- `cd dagster && python -m pytest dispar_orchestrate -q` (whole suite, once, final tree, loader tests included; `df -h /` 2.3 GB before): **1942 passed, 0 failed** (16m49s; slice 3 baseline 1886). New: `test_saas_servicenow.py` 30, `test_saas_confluence.py` 17, `test_saas_session.py` +4.
- `python3 ops/lint/check_intra_package_imports.py`, `check_bare_iceberg_count.py`, `check_connector_gate_coverage.py`: OK; `python3 -m unittest discover -s ops/lint -p 'test_*.py'`: OK.
- `bun test src/lib/connectors`: 49 pass, 1 skip, 0 fail. `bun test src/features/connectors`: 348 pass, 0 fail. (`bun run typecheck`/`lint` not re-run: only `saas-vendors.ts` changed, one optional field.) `bash scripts/ci/tests/test_required_gate.sh`: 52 passed.
- Gate vendor-level steps against the local stub (`PORT=18999 python3 ops/g6/saas_stub.py`, then `g6_saas_test.py --vendors-only`, checkout on `PYTHONPATH`): `PASS (32 s)`, six vendors.
- `cd rust && cargo fmt --check`: clean.

**Not verified, and why**
- **All Rust** (the `OauthPreset.api_base` field and the table rows; the probe's ServiceNow client-credentials exchange, Confluence Basic, the "not testable" arm; four new unit tests and the changed ones): formatted, **not compiled, not tested**. The tests that compare the Rust table with the fixture (`the_table_equals_the_shared_fixture`, serde `skip_serializing_if` for `apiBase`) run in CI.
- **The product steps of `g6-saas` for ServiceNow and Confluence, and the job's time**, did not run (no docker stack). CI may show: the API's connection test reaching the mock for `/oauth_token.do` and `/wiki/api/v2/spaces` through `gate_rebase`, the Bronze column names (`sys_id`, `modified_at`), whether `incremental_dedup` accepts `modified_at`/`sys_updated_on` timestamps, and the `max_reads` count of 2.
- **No real ServiceNow instance or Atlassian site** (decision 11): every shape is from the documentation pages read, modelled in the mocks and fakes. Not tried: the dictionary tables, `sys_audit_delete`, the client-credentials grant's set-up, Atlassian's PKCE handling (the product sends PKCE; Atlassian's page does not mention it), the `_links.next` form.
- Pages not readable from here: none for the points above; ServiceNow's release pages disagree on the revoke prerequisites (noted in the commit).
- Nothing was shown in a browser; no console change beyond the table copy (the connector form for Confluence/ServiceNow comes from the table).

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

After the fix round: CI on `dc3373ae` green, 38 of 38 checks (the hand-edited
`tool_schemas.json` passes the byte-identical snapshot, so `SHOULD-FIX 2` is
closed; `0097` and its test pass).

### Review of slice 2 (planner, 2026-10-19, head `9df9103a`)

Read: the handoff, `UPSTREAM.md` of both copies, the Zendesk duplicate fix
and its test, the HubSpot search cursor, the gate override. CI on this head
runs with the push of this review.

- `BLOCKER 3` (Zendesk browser sign-in). Decision 3 says a sign-in that
  `SRC-12` cannot serve as it stands is extended, not dropped. Add a third
  client authentication, `client_secret_post_json` (client id and secret in
  a JSON body), to the vendor table, the Rust token, refresh and probe
  calls, Python `rest._token_request`, the fixture and the console mirror;
  Zendesk's row uses it; remove the strict `xfail`; the gate's sign-in step
  runs. Cite the Zendesk documentation page in the commit.
- `BLOCKER 4` (the gate proves the vendor code, not the product). AD9 and
  the SRC-9 registry ask for connect, discover, load and reload steps that
  are `real`, which means through the product: create the connector through
  the API (the type being unsupported does not refuse it, as the developer
  found), the API's connection test, discovery through the orchestrator,
  a load and an incremental reload through `ingest_job` into Bronze
  Iceberg, checked by reading the table. Build this once for both vendors
  so later slices add only their steps; keep the vendor-only steps. The job
  needs the Dagster services, so record its time; if it cannot fit the
  20-minute budget, report it rather than cutting steps.
- Gate host override (`rebase_for_gate`): accepted, because it only acts
  under the existing gate-only switch `CONNECTOR_OAUTH_ALLOW_HTTP=true`,
  names a vendor and takes a bare origin. `SHOULD-FIX 5`: a test that it is
  ignored when the switch is off, and the Rust side gets the same rule
  under the same switch (the API's probe and sign-in must reach the mock
  for `BLOCKER 4`), with the same test.
- HubSpot `/crm/v3/` paths: accepted for now; `SHOULD-FIX 6`: record in
  `UPSTREAM.md` that the date-versioned API exists and v3 has no announced
  end date as of 2026-10-19, with the page read.
- Registry by path and `describe` answering `None`: accepted.
- HubSpot deletes not claimed, Zendesk deletes marked not removed: matches
  decision 7 ("applied where the vendor reports deletes"); the connector
  page must say a deleted Zendesk ticket stays with `status = deleted`.
  `SHOULD-FIX 7` if the page text does not say so today.

Slice 2 closed (planner, 2026-10-21). PR moved to #216 (opened as the data
roadmap's account; #214 closed, same branch). CI on `901bbe3a`: 36 of 38
green. The `g6-saas` job passed in about 4 minutes, with every product step
for HubSpot and Zendesk real (create, connect, discover, load into Bronze
read back through ClickHouse, incremental reload with no duplicate, revoked
sign-in) and every vendor step. The two red checks are `cargo audit` and
`cargo deny` on RUSTSEC-2026-0381/0382/0383 (`rustls-webpki`,
`webpki-roots`, `webpki-root-certs`), published after main's last Security
run; `Cargo.lock` is identical to main's, so main is red on them too. Not
caused by this PR and not ours to fix (another team's dependency work); it
needs a backlog item from that team before this PR can merge, because a PR
touching dependencies may not merge with a dependency check red, and this
one does not touch them.

### Review of slice 3 (planner, 2026-10-21, head `a583fc96`)

Read: the handoff, both `UPSTREAM.md`, the Salesforce query and `queryAll`
path, the GA4 REST calls, the `_apply_cursor` lag change, the Google
authorise address. Plan section 8 items 1 and 2 answered by the developer:
both libraries could be given a session, but neither under `SaasSession`'s
rules (redirects, pinning, `Retry-After`); calling the vendors' REST
endpoints through `SaasSession` and adding neither library is the right
choice under AD3 and is accepted. The `instance_url` adoption goes through
`adopt_origin` only: accepted. The Google `access_type=offline` and
`prompt=consent` fix: accepted (without it no load could run). The shared
`_apply_cursor` lag (`ObjectDecl.lag_seconds`): accepted, `sql_database`
resources unchanged and their tests pass.

- `BLOCKER 8` (decision 13). Salesforce reports an exhausted API quota as
  HTTP 403 with `errorCode` `REQUEST_LIMIT_EXCEEDED`; `SaasSession` reads
  every 403 as a missing permission. Give a vendor module a way to classify
  a 403 body (Salesforce: that code → `SaasRateLimited`, not retried within
  the run since the quota is per 24 hours, run class `rate_limited`), with a
  mock route and a vendor-level gate step. Cite the Salesforce limits page
  if it can be read; otherwise say so.
- `BLOCKER 9` (spec "table and column choice", `SRC-3-AC3`). A GA4
  connector cannot be set up in the console: there is no way to define a
  report. Add the report editor to the GA4 form: name, dimensions and
  metrics chosen from the property's `getMetadata` (through discovery, not
  typed blind), the date dimension fixed as the cursor; saved into
  `dial.objects` as the API already accepts. Tests for the form.
- `BLOCKER 10` (decision 3, `SRC-3-AC1`). The API's connection test answers
  `supported: false` for GA4's service-account key, one of the two signed
  ways. Implement the JWT-bearer exchange in the probe with the workspace's
  `jsonwebtoken` (RS256, already a dependency of `lakehouse-api`; no new
  crate), the key as a `SecretValue`; add the service-account way to the
  GA4 product steps of `g6-saas`.
- `SHOULD-FIX 11`. GA4's "yesterday" is computed in UTC while GA4 dates are
  in the property's time zone: read the property's time zone (Admin API
  `properties.get`, scope already granted? check) and use it; if that needs
  a scope the row does not ask for, keep UTC, widen the lookback by one day
  and state the limit on the connector page.
- `SHOULD-FIX 12`. The connector page states the two Salesforce limits
  (an object whose query exceeds the address length fails with a fixed
  message; API version 62.0) under "Limits to tell a customer".
- Salesforce row not checked against the vendor's documentation (pages
  refused from this machine): recorded; `SRC-3-AC1` on a real org is the
  proof. The "who am I" path `/services/data/v62.0/limits` is accepted.
