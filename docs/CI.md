# CI/CD

This repo's CI is split into workflows under `.github/workflows/`:

- **`ci.yml`** — change-scoped fast feedback and acceptance testing. Begins with
  a change scope detector (`changes`) and unconditional repo lints (`repo-lints`),
  runs selective frontend/Dagster/Rust/acceptance jobs based on modified paths,
  builds the `lakehouse-api` and Dagster images once (`images`) for the jobs
  that need them, and gates merges with a single summary gate (`ci-required`).
- **`security.yml`** — `cargo audit`, `cargo deny check all`, a working-tree
  `gitleaks` scan, GitHub's `dependency-review-action` on PRs, and a
  full-git-history `history-scan` job, which reports only findings outside
  the `.gitleaksignore` baseline (see below) and is deliberately excluded from
  required checks.
- **`docker.yml`** — runs on `v*` tag pushes only: builds `rust/Dockerfile`
  to check that a release tag still builds. A GHCR push job exists but is
  gated on tag pushes and is currently inert (no registry login
  configured). The `/health` smoke test that used to live here is the
  `image-smoke` job in `ci.yml`.
- **`coverage.yml`** — `cargo llvm-cov` (lcov, uploaded as a workflow
  artifact) and a CycloneDX SBOM per crate (also uploaded as an artifact,
  for Phase 6's release attachment). Runs on `push` to `main` and manual dispatch
  only to avoid spending ~30 runner minutes per PR run when no PR checks consume
  the artifact.
- **`security-all-branches.yml`** — daily scheduled (`cron: '17 3 * * *'`) and manual
  dispatch scan of all git branches using `gitleaks` (`--log-opts="--all"`).
  Deliberately separate from pull-request checks so findings on one branch do
  not block unrelated PRs (SEC-36, D2).

## Change Scopes and the Single Gate (`ci-required`)

To reduce CI feedback latency on targeted changes (such as docs-only or frontend-only PRs),
`ci.yml` classifies changes in the `changes` job into six scopes:

| Scope | Triggered By |
|---|---|
| `rust` | `rust/**`, `src/lib/dashboard-specs.ts`, `docker-compose.yml` |
| `frontend` | `src/**`, `public/**`, `package.json`, `bun.lock`, `bunfig.toml`, `tsconfig.json`, `next.config.ts`, `eslint.config.mjs`, `postcss.config.mjs`, `components.json`, `Dockerfile.frontend` |
| `dagster` | `dagster/**` |
| `stack` | `docker-compose.yml`, `ops/**` (except `ops/fixtures/**`), `scripts/**` (except `scripts/ci/**`), `.env.example` |
| `docs_only` | True when every changed file is under `docs/**`, `GTM/**`, or is a root `*.md`, `LICENSE` or `NOTICE` |
| `all` | `.github/**`, `ops/fixtures/**`, `scripts/ci/**`, an unreadable base SHA, or any file unclassified by the rules above (fails safe) |

On a `push` to `main`, all jobs run unconditionally.

Downstream jobs execute according to scope:
- **`repo-lints`**: Runs unconditionally on all pushes and PRs. Folds the static lints (parity corpus secrets, R11 bare iceberg count, compose init readiness, Dagster intra-package imports, connector gate coverage (`SRC-9`: every supported connector type has a gate in `ops/gates/coverage.json`, or an explicit waiver that the output lists)) and the unit tests of the connector gate tooling into one fast job.
- **`verify`**: Runs if `frontend` or `all` is true.
- **`dagster-unit-tests`**: Runs if `dagster` or `all` is true. Runs the tests not marked `loader` (`-m "not loader"`).
- **`dagster-loader-tests`**: Same condition. Runs the tests marked `loader` (`-m loader`): a real `dlt` pipeline against a local Iceberg table. Split from the job above because together they took 13m52s of a 15-minute limit (PR #115).
- **`g6-credentials`** (`SRC-12` part A, `ops/g6/src12_credentials_test.py`): checks that `docker compose config` refuses an empty `CONNECTOR_SECRETS_KEY` and names it; starts the `g6-ops` stack with `ops/g6/docker-compose.src12.override.yml` (the writable credentials volume the other g6 gates replace); types a MySQL credential through the API and checks the stored file is a seal (version byte, not the plain value, owner-only) and that the connector tests and loads; runs `lakehouse-api rotate-secrets-key` with a new key (old key as `CONNECTOR_SECRETS_KEY_PREVIOUS`), drops the old key, recreates the API and the code location, and checks the file changed and the connector still tests and loads. Its own job so the slowest connector gate stays under 20 minutes; the first CI runs are its measurement. Not run against a real stack before it reached CI.
- **`g6-secret-managers`** (`SRC-12` part B, `ops/g6/src12_managers_test.py`): starts the `g6-ops` stack with `ops/g6/docker-compose.src12.override.yml` and `ops/g6/docker-compose.src12-managers.override.yml`, which adds Vault's official development server (`vault-g6`) and a `moto` server (`moto-g6`), both pinned and CI-only. The gate runs twice, once per manager, because an installation has one: with `SECRET_MANAGER=vault`, then (after `.env` is switched and the API and the code location recreated) with `SECRET_MANAGER=aws`. Each run stores a MySQL source password in the manager, refuses a reference to nothing (422, fixed words, none of the manager's) and another manager's scheme (400), saves a reference through `PUT .../credential` (read once, connection tested), checks the credentials volume holds no file and no copy, tests and loads through the reference, changes the secret in the manager to a wrong value (the next test and run fail), restores it (the next run succeeds, appends). Google Secret Manager and Azure Key Vault have no local stand-in: they are covered by unit tests against recorded responses (`secret_manager/gcp.rs`, `azure.rs`, `test_secret_manager_gcp.py`, `..._azure.py`) and were not run against the real services. Its own job: the bring-up of `g6-credentials` is 12 to 14 minutes and the two together would pass its 25; the first CI runs are its measurement. Not run against a real stack before it reached CI; the image tags (`hashicorp/vault:1.18.5`, `motoserver/moto:5.0.9`) were not pulled where this was written.
- **`g6-oauth`** (`SRC-12` part C, `ops/g6/src12_oauth_test.py`): starts the same stack as `g6-ops` with `ops/g6/docker-compose.src12.override.yml` (the writable credentials volume and the runner's read-only view of it), and the CI-only `.env` sets the gate-only `CONNECTOR_OAUTH_ALLOW_HTTP=true` and `CONNECTOR_OAUTH_REDIRECT_URI=http://lakehouse-api:8080/api/connectors/oauth/callback`. The provider is a thread of the gate's runner: an authorize endpoint that approves at once and redirects with a code, a token endpoint that checks the PKCE verifier, issues 5-second access tokens and rotates the refresh token (honouring only the latest), a revocation endpoint, and a REST API whose pages answer 2 seconds late and require the bearer token. Steps: the client secret is saved before any sign-in (unverified, sealed); the gate signs in as a browser would (S256, random state, redirect to the product's callback with its own session); a load of 5 rows outlives one access token (at least two refresh grants are counted at the provider); the rotated refresh token is sealed in place (the file changed, still no clear text) and a second load after the first access token expired succeeds (the run record says so, the provider counted a refresh-token grant for it, the sealed token changed again; the table is loaded by `replace`, so rows are not the proof); the connection test refreshes and passes twice; a wrong and a replayed `state` are refused with the fixed page and never reach the provider; a refresh token the provider no longer honours fails one attempt as `sign_in_refused` with none of the provider's words in the log, and signing in again restores loading; signing out and deleting the connector each call the revocation address with the stored token and remove the files. Its own job, 25 minutes; the first CI runs are its measurement. Not run against a real stack before it reached CI.
- **`g6-saas-a`** and **`g6-saas-b`** (`SRC-3` AD9 amended before slice 6, `ops/g6/g6_saas_test.py`): the gate of the business-app (`saas`) connectors is two jobs that run in parallel on the same scripts, each given the vendors it owns by `SAAS_GATE_VENDORS` (comma separated vendor ids, or `--vendors=`; empty means every vendor, as in a local run). `g6-saas-a` owns HubSpot, Zendesk, Salesforce, Google Analytics 4 and ServiceNow; `g6-saas-b` owns Confluence, SharePoint, Dynamics 365, Workday and NetSuite. One job with eight vendors took 16m17s of the signed 20 minutes (`7a8af090`), so ten would not fit; no step was removed to make them fit, and each job has its own compose project (`saasagci`, `saasbgci`). Both are in `ci-required` and `scripts/ci/required_gate.sh` (`G6_SAAS_A_RESULT`, `G6_SAAS_B_RESULT`), and `coverage.json` (slice 7) names the job each type is proven in. The rest of this entry describes what either job runs for its vendors. It is a gate in jobs of their own so that no existing connector gate grows (`g6-ingest-other` measured 14m53s to 18m36s of its 20 minutes, `docs/plans/SRC-9-RESULT.md`). One mock vendor server (`ops/g6/saas_stub.py`, stdlib only, service `saas-stub-g6`, profile `g6-saas`) answers for every vendor on the vendor's own port (the vendor's real paths, `VENDOR_PORTS` in the stub; `GET /health` and the `/__admin/<vendor>` state controls on 8080). **Through the product (slice 2 review BLOCKER 4):** the job runs the Dagster stack as `g6-ingest-other` does (`--profile dagster`: API, `dagster-code-location`, `dagster-webserver`, `dagster-daemon`, ClickHouse and the catalog as dependencies) with `ops/g6/docker-compose.g6-saas.override.yml` layered on `docker-compose.g6.override.yml`; that file turns on, for the API and the code location only, the gate-only `CONNECTOR_OAUTH_ALLOW_HTTP` and `SAAS_GATE_ORIGINS` (vendor id to the mock's origin; a no-op without the switch, in Rust and in Python, both tested) so that the product dials the mock. For every vendor in `PRODUCT_VENDORS` the gate: creates the connector through the API (a type that is `supported = false` is not refused) and saves its ingest spec; runs the API's connection test (passes, names the account, repeats none of the mock's words); lists the objects through the orchestrator (the object says it loads incrementally) and previews it (HubSpot's properties API, Zendesk honestly `not_supported`); loads with `ingest_job` into Bronze Iceberg and reads the rows back through ClickHouse; changes the vendor, reloads, and reads back the merged table (no key twice; Zendesk's deleted ticket says `status = deleted`; HubSpot's archived deal stays because HubSpot reports no deletes); revokes the sign-in and checks the API's test and a run fail without the vendor's words. The browser sign-in has no browser here, so it stays a vendor-level step, and the vendor-level steps (the vendor module and `SaasSession` run in the runner against the mock) stay as well: HubSpot connect with the private-app token and the browser sign-in, discover, load, reload through search, a new property in describe, a 429 with `Retry-After` waited out and one that never lifts, a revoked token and refresh token; Zendesk connect with an email and API token and with the browser sign-in (the token request is JSON and the refresh token rotates), the mock's time-based export repeating tickets at page boundaries (upstream #659) and the connector reading each once, a cursor reload with a changed, a new and a deleted ticket, a new field, rate limits, a forbidden and a revoked sign-in. `--vendors-only` runs the vendor-level steps with no API against a local `python ops/g6/saas_stub.py` (set `PORT` if 8080 is taken). The API steps first assert the ten types are `supported: false`. ServiceNow (client credentials; Table API paged by the `Link` header; the dictionary describes the table) and Confluence (email + API token for the product steps; the browser sign-in behind Atlassian's gateway, with its cloud id, at vendor level; a reload that must stop near the cursor, `max_reads`) joined in slice 4. SharePoint (Graph delta feed, its link kept in dlt state; a delete marked; a 410 Gone resync at vendor level and in the dlt-state pytest) and Dynamics 365 (an untracked table filtered by `modifiedon`, a tracked table read by its change-tracking delta link) joined in slice 5; each has a product entry for client credentials and one for the browser sign-in's connection test and load, with Entra's token endpoint in the mock at the customer's tenant. Workday (a report read once and replaced; Basic for the integration user, the refresh grant with the client in the Basic header) and NetSuite (SuiteQL pages by `offset` and `hasMore`; the certificate way, whose PS256 assertion the mock verifies with the public key the gate registers, and the browser sign-in) joined in slice 6, each with a vendor-level run and two product entries in `g6-saas-b`; both are models of documentation that could not be run against a real account. Each later vendor adds an entry to `PRODUCT_VENDORS`, its mock and its vendor-level steps in its own slice, and the types stay `supported = false` until slice 7. Budget: the signed 20 minutes per job (each job's cut-off is 25, as `g6-ingest-other`'s); the first CI runs are its measurement and the job summary has each vendor's product steps time.
- **`g6-discovery-scale`** (`SRC-13` decisions D9 and D12, `ops/g6/src13_discovery_scale_test.py`): creates a PostgreSQL schema of 10,000 tables in the stack's own database, lists it through `POST /api/connectors/conn-pg-lakehouse/discovery/list`, searches one table by part of its name, previews it, and prints each call's seconds on `[src13]` lines that are copied to the job summary. **Fails if the listing takes more than 120 seconds** (the feature page's "10,000 tables in 2 minutes", D9) or does not return exactly the 10,000 names. The stack is the API and its PostgreSQL only (with ClickHouse and `oidc-mock`, which the API depends on): no Dagster, because the API lists PostgreSQL itself. Its own job, so no existing connector gate grows past its 20 minutes (the type-by-type `discover` steps of `g6-ingest`, `g6-ingest-other`, `g4-cdc` and `g6-oracle` are quick calls, and the ones the orchestrator answers run after the loads). Estimated 8 to 12 minutes; the first CI runs are its measurement. The measured seconds are for a CI runner against a local PostgreSQL, not a customer's database. Not run against a real stack before it reached CI.
- **Rust fmt / clippy / build / test / msrv**: Runs if `rust` or `all` is true.
- **Acceptance jobs (`g1-rustfs`, `g2-seaweedfs`, `g3a-dagster`, `g3-maintenance`, `g4-cdc`, `g6-ingest`, `g6-ingest-other`, `g6-oracle`, `g9-upload`, `g6-alerts`, `g6-ops`, `g6-credentials`, `g6-secret-managers`, `g6-oauth`, `g6-discovery-scale`, `g6-saas-a`, `g6-saas-b`, `gold-export`, `g8-governance`, `g8-time-travel`)**: Run if `rust`, `dagster`, `stack`, or `all` is true.
- **`images` and `image-smoke`**: Same condition as the acceptance jobs; see "Images: built once per run" below.

`ci.yml` runs with `permissions: contents: read` for the whole workflow, which is the
token a fork PR is given. Nothing in it needs more: image artifacts and the layer cache use
the Actions runtime credentials, which every run gets.

At the end of the pipeline, **`ci-required`** runs with `if: always()`, checks every job result against the detected scope, and fails if any required job failed, cancelled, or was skipped when the scope indicated it should have run. Both the scope detection script (`scripts/ci/detect_change_scope.sh`) and the gate script (`scripts/ci/required_gate.sh`) have accompanying self-tests that run in CI before the scripts are executed.

## Images: built once per run

Before CI-SPEED-PLAN PR 2, the acceptance jobs that start the stack (six when it was written;
eleven with the gates `main` has added since) and the Docker workflow each built the
`lakehouse-api` image from source (a release build of about 515 crates), and most of those
jobs built the Dagster image as well. Now:

- The **`images`** job builds both images once and uploads each as a workflow artifact
  (`image-lakehouse-api`, `image-lakehouse-dagster`): `docker buildx` with
  `outputs: type=docker`, compressed with `zstd`, one-day retention, no second
  compression by the artifact service. It prints each archive's size in the log.
- Each consumer (`g3a-dagster`, `g3-maintenance`, `g4-cdc`, `g6-ingest`, `g6-ingest-other`,
  `g6-oracle`, `g9-upload`, `g6-alerts`, `g6-ops`, `g6-credentials`, `g6-secret-managers`, `g6-oauth`, `g6-discovery-scale`, `g6-saas-a`, `g6-saas-b`, `gold-export`, `g8-governance`, `g8-time-travel`,
  `image-smoke`) `needs: [changes, images]`, downloads the artifact(s) it
  uses, `docker load`s them, and starts the stack with `up -d --no-build`. `gold-export`,
  `g8-governance` and `image-smoke` only use the API image and download only that one.
  A missing image therefore fails the job, it never triggers a silent rebuild. The test
  runner commands are unchanged.
- `docker-compose.yml` names the images: `lakehouse-api` is
  `${LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}` and every service built from
  `./dagster` is `${LAKEHOUSE_DAGSTER_IMAGE:-rantai-lakehouse-dagster:local}`. CI sets both
  variables to `…:ci` tags in the job `env:`. The `build:` blocks stay, so local
  `docker compose up --build` is unchanged. The Dagster-built services (the five in
  `docker-compose.yml` plus `g6-oracle-test-runner`, `g6-alerts-test-runner`, `g6-ops-test-runner`, `src13-scale-test-runner` and `g6-saas-test-runner` (both SaaS jobs) in the
  `ops/g6` overrides) must keep identical `build:` blocks (same `args:`), because a build of
  any one retags the image they all use. A new service built from `./dagster` needs the same
  `image:` key, or `docker compose run` builds the whole Dagster image again.
- Two small fixture images are **not** shared: `oidc-mock` (every stack that starts
  `lakehouse-api`) and `rest-stub-g6` (only `g6-ingest-other`, profile `g6-other`) and `saas-stub-g6` (only the two `g6-saas-*` jobs, profile `g6-saas`). Each
  acceptance job builds the ones its stack needs with an explicit `docker compose … build` (same `-p`, `-f` and
  `--profile` flags as its `up`) just before `up --no-build`, which would otherwise fail on
  a service that has a `build:` block and no local image.
- `g1-rustfs` and `g2-seaweedfs` do not use these artifacts: they never built the API image.
- `GIT_SHA` is left unset in CI, so both images are built with `GIT_SHA=unknown` as before.
  `ARG GIT_SHA` sits near the top of `dagster/Dockerfile`; a per-commit value would invalidate
  every layer below it.
- **`image-smoke`** is the `/health` smoke test that used to be `docker.yml`'s
  `build-and-smoke-test`: it runs the shared API image standalone (no Postgres or
  ClickHouse) and asserts `/health` returns 200. `ci-required` covers it, so it is no
  longer a separate required check (see branch protection below).
- If `images` fails, every job that needs it is skipped. `ci-required` treats `images`
  and each consumer as required whenever the acceptance jobs are, so that still turns the
  gate red (covered by `scripts/ci/tests/test_required_gate.sh`).

### `rust/Dockerfile` and `CARGO_TARGET_DIR`

The Dockerfile has a tools base (`sqlx-cli` and `cargo-chef`, both pinned and installed above
every `COPY` of repo files), a planner (`cargo chef prepare`), and a builder that compiles the
dependencies from `recipe.json` alone (`cargo chef cook --release --locked -p lakehouse-api`)
before copying the sources. The dependency layer changes only when `Cargo.lock` or a
`Cargo.toml` does.

One build argument serves two audiences. `CARGO_TARGET_DIR` defaults to `/build/target`, the
BuildKit cache mount: a local rebuild keeps compiled dependencies in the mount, exactly as
before. A cache mount is not stored by the GitHub Actions layer cache, so CI passes
`CARGO_TARGET_DIR=/build/target-layer`, a path outside every mount: `cook`'s output then lands
in the image layer and can be restored from the cache. The mount stays at `/build/target` on
both steps either way.

## Caches

The repository's Actions cache is capped at 10 GB and was at 9.9 GB when PR 2 was written
(2026-10-09). Two rules keep PRs from filling it:

- **Image layer cache** (`type=gha`, scopes `api-image` and `dagster-image`): read on every
  run, **written only on `push` to `main`** (`mode=max`). A cache written by a PR can only be
  read by that PR, so letting PRs write only evicts `main`'s entries. `docker.yml`'s tag build
  reads `api-image` and writes nothing.
- **`Swatinem/rust-cache`**: every step in `ci.yml` has `save-if: github.ref ==
  'refs/heads/main'`. PRs restore `main`'s cache and do not store their own (PR #88 stored a
  980 MB copy of `build`'s). The `fmt` job has no cache step at all: `cargo fmt --check`
  compiles nothing.

If `images` is cold on `main` more often than on a Cargo.lock change, the cache is being
evicted: reduce what `rust-cache` stores before anything else (CI-SPEED-PLAN §9).

## MSRV

`rust/Cargo.toml`'s `rust-version` is `1.99` (REL-9, product-owner decision
D1, 2026-10-17), the same release as the `rust-toolchain.toml` pin. The
floor is deliberately not a dependency's minimum: `sqlx` 0.9 — which
`SEC-41` needs — requires rustc 1.94, so a held-over lower promise could
not be kept, and the owner moved pin and floor to the latest stable
together. The MSRV stays proven, not assumed: the CI `msrv` job runs
`cargo +1.99.0 check --workspace --all-targets --locked` in `rust/`, one
patch below the pin.

The `+1.99.0` is load-bearing. `rust-toolchain.toml` beats anything a
checkout action installs as the *default* toolchain (rustup precedence:
command-line `+` spec > `RUSTUP_TOOLCHAIN` env > `rust-toolchain.toml` >
default), so a plain `cargo check` in a job that installs `1.99.0` would
compile the workspace on the pin instead of the floor — which is what the
old job did. The same precedence means CI's `fmt`/`clippy`/`build`/`test`
jobs, which install `stable`, actually build with the pin; local dev does
too, so one compiler everywhere.

## `history-scan`: baselined, red only on a new finding

A Phase 1 audit found a real internal LLM API key leaked in git history, on 2 commits
reachable from both `main` and `feat/rust-backend`, together with two
internal LAN hostnames. The exact values are deliberately not reproduced
here — see the rule definitions in `.gitleaks.toml`.

**gitleaks' default ruleset does not match either pattern.** Scanned with
defaults, `history-scan` came back green — a false negative. That is worse
than no scan at all: it tells a reviewer history is clean when it is not.
`.gitleaks.toml` therefore adds two custom rules (`rantai-llm-node-api-key`,
`rantai-internal-lan-host`) matching the key prefix and host range.

With those rules and the default ones, the scan of `main` at `a13e5ea`
(CI job 115776111835) reported 15 findings; a scan that reaches the root
commit `6c3a7ff` reports four more hits for the same addresses, so 19
entries are baselined. The product owner confirmed on
2026-10-14 that every real secret among them is rotated, and a history
rewrite would break every open branch, so they are **baselined, not
rewritten** (SEC-1, plan
[`2026-10-14-sec-1-gitleaks-baseline.md`](superpowers/plans/2026-10-14-sec-1-gitleaks-baseline.md)).
`.gitleaksignore` at the repository root lists each one by its exact
fingerprint (commit, file, rule, line):

| Baselined | What | Why it may be ignored |
| --- | --- | --- |
| 7 lines of `.env.staging` in `1d2c45c` | Staging tokens and secret | Rotated; SEC-13 (#130) moves them to repository secrets `STAGING_*` |
| `llm.ts:9` in `8ac6ee2` | Internal LLM node key | Rotated |
| 5 `rantai-internal-lan-host` hits in `8ac6ee2`, `c5954ca`, `7075682` (2), `27d1e2b`, and the same 4 files in the root commit `6c3a7ff` (an ancestor of `main`) | Private LAN addresses in old commits | Not secrets; none of those files holds them today |
| `test_schema_observer.py:26` in `31ec837` | Test token literal | False positive; the literal is gone since #129 |
| `src11_connector_operations_test.py:576` in `4ae1bcd` | `password=matrix.CONNECTOR_MYSQL_PASSWORD` on `feat/phase-1-rest` (#144) | False positive: a reference to a CI fixture constant (surfaced when scanning all branches) |

Because the entries are full fingerprints, they match nothing else: the same
secret in a new commit, or any new secret, is **not** baselined and turns
`history-scan` red. A red run is therefore a new finding. Rotate the secret
first; only then decide whether it may be added to `.gitleaksignore` (with a
comment saying why, the date, and the item ID).

`.gitleaks.toml` also allowlists, by exact literal, the three non-secret
`CONNECTOR_SECRETS_KEY` values `SRC-12` needs: the CI `.env` key (base64 of the
32 characters `ci-connector-key-not-real-000000`), the shared test vector's
key and the API integration tests' key. Every CI job that starts the stack
writes that CI key into its `.env`, because `docker-compose.yml` refuses to be
read without `CONNECTOR_SECRETS_KEY`; a new job that runs `docker compose`
needs the same line.

`.gitleaksignore` is read from `--source` (and the working directory);
blank lines and full-line `#` comments are skipped (gitleaks v8.30.1,
`detect/detect.go`, `AddGitleaksIgnore`).

The working-tree `gitleaks` job uses the same config and is a separate
check. `.env.local` does contain live values and is flagged on a local scan,
but it is gitignored and never checked out in CI.

`history-scan` stays a **separate job**, not folded into the working-tree
`gitleaks` job and not wrapped in `continue-on-error`, precisely so that
whichever way it goes (red or green) is independently visible in the
Actions UI on every run, rather than being averaged into another job's
status. It is **excluded from required status checks** (see below) so that
a red run — a real regression, a rule update, or new evidence — cannot
silently block a merge; a human needs to look at it.

Removing the leaked values from history itself would still require:

1. Rotating the exposed key at the provider (done for the findings above).
2. Rewriting history (`git filter-repo` or BFG) to purge the blob from every
   affected commit.
3. Force-pushing every affected ref, which invalidates every existing clone
   and any open PR based on the old history.

That is a destructive, cross-cutting, and irreversible-for-clones operation,
deliberately not undertaken as a side effect of a CI-hardening pass — it
needs a human to explicitly decide to take it on and coordinate with anyone
holding a local clone. The baseline makes it unnecessary for keeping the
scan useful.

### Scan scope on PRs and `main` vs. the daily all-branch scan (`SEC-36`)

Historically, `history-scan` fetched all remote branches and invoked `gitleaks`
with no `--log-opts`, causing `gitleaks` to pass `--all` to `git log` and
scan every ref in the repository. A test value or secret pushed on an
unrelated feature branch (such as #144) turned the check red on `main` and
on every open PR.

Under `SEC-36` (decisions D1–D2):

- **In `security.yml` (`history-scan`)**: on pushes to `main` and on pull
  requests, `gitleaks detect` runs with `--log-opts="HEAD"` (D1). On a push
  to `main`, this scans `main`'s history; on a pull request, `actions/checkout`
  checks out `refs/pull/<n>/merge`, so it scans `main` plus the pull request's
  commits. It does not scan unrelated branches, making the scan independent
  of what other refs exist or are fetched.
- **In `security-all-branches.yml`**: a dedicated daily workflow (`cron: '17 3 * * *'`
  and `workflow_dispatch`) fetches all remote branches and runs
  `gitleaks detect --log-opts="--all"` (D2). This runs as a scheduled job,
  deliberately not as a pull-request check, so a finding on one branch
  never blocks an unrelated PR.
- **When the daily all-branch run turns red**: it means at least one branch
  in the repository carries an un-baselined finding. The owner of that branch
  must fix it: by rotating the secret, replacing the literal with a synthetic
  value that does not match the rule, or baselining it in `.gitleaksignore`
  with an exact fingerprint and justification.

Note on merge commits: `gitleaks` uses `git log -p -U0` under the hood. Git
by default skips diffs for merge commits in `git log -p` unless `-c`, `--cc`,
or `-m` is specified. Consequently, conflict resolutions introduced in merge
commits are not diffed by `gitleaks` (this was true before `SEC-36` and
remains true).


## Docker

`docker.yml` runs on `v*` tag pushes only and builds `rust/Dockerfile` with the same build
arguments as the `images` job, reading (never writing) its `api-image` layer cache. It no
longer runs on pushes to `main` or on pull requests: the API image is built, shared and smoke
tested by `ci.yml` (`images`, `image-smoke`).

`image-smoke` runs the image standalone (no Postgres/ClickHouse containers), because
`lakehouse-api` is designed to boot and serve its DB-independent routes — including
`/health` — without either dependency (`entrypoint.api.sh` skips migrations when
`DATABASE_URL` is unset; see `lakehouse_store::connect_lazy`'s doc comment). That's a real
smoke test of the container's own boot path, not a substitute for `docker compose up`
against the full stack.

The `push-ghcr` job is gated on `startsWith(github.ref, 'refs/tags/')` and,
even when that condition is met, does not actually push anywhere — there is
no `docker/login-action` step and no registry credential configured. Wiring
up real publishing is an explicit, separate decision for a later phase.

## Dependency review: now works, verified on a real PR

This job previously failed unconditionally: `dependency-review-action`
requires GitHub Advanced Security for *private* repositories, and this org
was on GitHub's free plan. Now that the repository is public, **GHAS
dependency review is free for public repositories**, so the job was
expected to start passing. This was verified directly, not assumed: a
throwaway PR (#23, "dependency-review probe", closed without merging, branch
deleted) was opened against `main` with a trivial docs-only change, and its
`Dependency review (PR only)` job (`security.yml`) completed with
`conclusion: success` (checked via
`gh api repos/RantAI-dev/RantAI-Lakehouse/actions/jobs/<id>`, all steps
`success`, including the `Dependency review` step itself).

It is still left out of required status checks for now — not because it's
expected to fail, but so it can prove itself stable across a few more real
PRs (e.g. one that actually touches a manifest with a diff for it to
evaluate) before being promoted to required in the branch-protection command
below.

## Branch protection on `main` (applied 2026-10-30)

Applied on 2026-10-30 with the command below, at the product owner's
request (`REL-1`). The repository is public, so GitHub's free plan allows
it. An earlier attempt during the public-release hardening pass had been
blocked by a permission classifier. Check what is live with
`gh api repos/RantAI-dev/RantAI-Lakehouse/branches/main/protection`; when a
required job is renamed, run the command again with the new name.

```sh
gh api \
  --method PUT \
  -H "Accept: application/vnd.github+json" \
  repos/RantAI-dev/RantAI-Lakehouse/branches/main/protection \
  -f 'required_status_checks[strict]=true' \
  -f 'required_status_checks[checks][][context]=ci-required' \
  -f 'required_status_checks[checks][][context]=Repo lints' \
  -f 'required_status_checks[checks][][context]=cargo audit (advisories)' \
  -f 'required_status_checks[checks][][context]=cargo deny check (all)' \
  -f 'required_status_checks[checks][][context]=gitleaks (working tree)' \
  -F 'enforce_admins=false' \
  -F 'required_pull_request_reviews=null' \
  -F 'restrictions=null' \
  -F 'allow_force_pushes=false' \
  -F 'allow_deletions=false'
```

The check names above reflect the real `name:` values from
`.github/workflows/{ci,security}.yml` as of this writing — not
guessed. `ci-required` consolidates and enforces all scope-dependent tests
in `ci.yml`, including the image build and `/health` smoke test
(`images`, `image-smoke`). The check `Build lakehouse-api image · smoke test
/health` that used to be listed here no longer reports on PRs (`docker.yml` is
tag-only), so it must NOT be required: a required check that never reports
blocks every merge under `strict` mode. If a workflow's job names change, this list needs to be updated to
match, or `strict` mode will block merges on a check that no longer reports.

Why each choice:

- **Required checks are the honest-green ones only.** `ci-required` (enforcing
  all scope-required CI jobs), `Repo lints`, `cargo audit`, `cargo deny`,
  and `gitleaks (working tree)` are the jobs that are expected to actually pass
  on a healthy `main`.
- **`gitleaks (full git history)` is deliberately NOT in this list.** It now
  reports only findings outside the `.gitleaksignore` baseline (see
  [SECURITY.md](../SECURITY.md) "Known exposure" and the `history-scan`
  section above). Requiring it would let one new finding block every merge,
  or pressure someone into baselining a real one — neither is acceptable.
  It stays present and visible in the Actions UI, not gating.
- **`Dependency review (PR only)` is also NOT in this list**, even though
  it can now pass on a public repo (GHAS dependency review is free for
  public repositories — see "Dependency review: now works, verified on a
  real PR" above for the actual PR run that verified this). It's left
  optional for now rather than required so a
  future job rename or dependency-graph hiccup can't silently block merges
  before anyone's had a chance to watch it run a few times; promote it to
  required once it's proven stable.
- **`enforce_admins: false` is deliberate**, not an oversight. This is
  presently a solo/small-team project; enforcing admin restrictions too
  early risks locking the owner out of their own repo during a hotfix. It
  is worth flipping to `true` once there is more than one active
  maintainer and a real PR review culture, at which point requiring PR
  reviews (`required_pull_request_reviews`, currently left `null`/off
  above) is also worth turning on.
- **Force pushes and branch deletion are blocked** (`allow_force_pushes:
  false`, `allow_deletions: false`) regardless of the above — those are
  cheap to disallow and protect against both accidents and, given the
  history-rewrite decision this repo is deliberately deferring (see
  above), against an *accidental* force-push doing that rewrite before a
  human has actually signed off on it.
- **Push protection matters here too.** GitHub secret-scanning push
  protection is now enabled on this repository, independently of branch
  protection: a future commit containing a recognized secret pattern
  (including the two custom patterns in `.gitleaks.toml`'s rules, once/if
  they're also expressed as a GitHub secret-scanning custom pattern) will
  be blocked at push time, before it ever reaches `main`. This is a
  second, earlier line of defense, not a replacement for branch protection.
- Consider **requiring signed commits** and **linear history** once the
  team's workflow is settled; neither is load-bearing for this phase.

This has to be applied by someone with admin rights on the repo — it is not
something a workflow file can configure for itself, and (as noted above) it
could not be applied programmatically during this pass either.
