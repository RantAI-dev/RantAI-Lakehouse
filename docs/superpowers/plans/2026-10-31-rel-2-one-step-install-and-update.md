# REL-2: install and update in one step, online or air-gapped

Backlog `REL-2` (P1). Spec: `docs/core/specs/rel-2.md`. Feature page:
`docs/core/features/one-step-install.md`. Base: `main` at `677845db`. Every `file:line` below was
read there.

Planner: Claude Opus. Developers: other agents (`AGENTS.md`). A developer does not change a
decision below. If the code disagrees with the plan, stop and report it.

## 0. Shape of the solution

One idea carries the design: **sign one file, verify everything through it.**

- **The release manifest** (`release.json`) is signed with the release key. It holds:
  - the version;
  - every image's digest;
  - the checksum of the Compose file and of `lakehousectl`;
  - the lowest version an update may start from, and the required stops.
- **`lakehousectl`** (a static Rust binary) holds the public key. It checks the manifest's
  signature, then checks every image and file against the digests in it, online or offline. It
  needs no `cosign`, no registry and no network to verify.
- Images are still signed with `cosign` in the registry, for operators who pull them by hand.
- **Rollback is a restore.** Migrations stay forward-only. An update takes a backup first, and
  going back restores it, as GitLab does.
- **Object storage is not copied before an update.** Iceberg data files are immutable, and the
  catalog that points at them is in Postgres, which is backed up. A restored catalog points at
  files that still exist, provided nothing expires them in between. So `lakehousectl update`
  pauses table maintenance (snapshot expiry, orphan removal) until the update is confirmed or
  rolled back. This is decision D9 below.

## 1. Slices

| Slice | What | Depends on | Machine |
| --- | --- | --- | --- |
| **S1** | Product prerequisites: readiness and version, the console in Compose, digest pins, a complete local-disk backup | nothing | bluefin (Rust and Compose) and the desktop or this VM (console) |
| **S2** | The release pipeline: publish, sign, SBOM, the manifest, the offline bundle | owner decisions 2, 3, 4 | bluefin |
| **S3** | `lakehousectl install` (online and offline) and `install.sh` | S1, S2 (format only) | bluefin |
| **S4** | `lakehousectl update`, `backup`, `restore`, `rollback`, `status`, `self-update`, and the CI upgrade test | S3 | bluefin |
| **S5** | Docs, the signed release policy, and cutting 0.2.0 | S1–S4 | planner and owner |

S1 starts now. S2 to S4 are written here so the whole design is reviewable, and each gets a
"Plan check" by the planner before it starts, against the code of that day.

## 2. Decisions (the owner's four, marked OWNER, signed on 2026-10-31)

- **D1 (OWNER #1).** The tool is `lakehousectl`.
- **D2 (OWNER #2).** Releases live on GitHub Releases. `install.sh` is served from the release
  and from `https://get.rantai.dev/lakehouse`, a Cloudflare redirect to
  `https://github.com/RantAI-dev/RantAI-Lakehouse/releases/latest/download/install.sh`.
- **D3 (OWNER #3).** A cosign ECDSA P-256 key pair. The planner created it on 2026-10-31.
  - The encrypted private key and its password are the `COSIGN_PRIVATE_KEY` and
    `COSIGN_PASSWORD` secrets of the `release` environment. Its required reviewer is the product
    owner, and it deploys from `v*` tags only. No other copy exists.
  - The public key is committed at `ops/release/release.pub` and compiled into `lakehousectl`
    with `include_bytes!`.
  - CI dry runs use a throwaway key generated in the job. They never touch the real key.
- **D4 (OWNER #4).** SemVer as in `reference/release-policy.md`. The first release through the
  pipeline is 0.2.0.
- **D5. Install layout.**
  - Everything lives in one directory, `/opt/rantai-lakehouse` by default (`--dir` to change it):
    - `docker-compose.yml`, the release's file with digests pinned;
    - `.env`, mode 0600;
    - `release.json` and `release.json.sigstore.json`;
    - `backups/`;
    - `state.json`, which holds the current and previous versions and the last backup.
  - The Compose project name is `rantai-lakehouse`. The operator never edits the Compose file.
    Settings go in `.env`, or in `override.yml`, which `lakehousectl` passes as a second `-f`
    when it exists.
- **D6. Secrets.** Generated only when missing, never overwritten, with the same set
  `ops/init-env.sh:64-90` fills.
  - `lakehousectl` reuses that logic (Rust port) and keeps `.env.example` as the list of keys.
  - The first administrator's password is generated and printed once, and must be changed at
    first sign-in. That already holds today (`AUTH_BOOTSTRAP_*`, `OPERATIONS.md:1025-1040`).
- **D7. Readiness.** A new public route, `GET /health/ready`, answers:
  - 200 `{"ready":true,"version":"…","checks":{…}}` when all of these hold: Postgres is
    reachable and at this build's schema version, ClickHouse answers `SELECT 1`, and Lakekeeper's
    config answers;
  - 503 with the same shape otherwise.

  Each check is only `"ok"` or `"failed"`, with no upstream text (principle 4). `version` is
  `CARGO_PKG_VERSION`; `/health` gains it too. The public-route count test changes from 13 to 14,
  with the reason in the commit.
- **D8. Backup set.** One backup is a directory `backups/<UTC timestamp>-<version>/` holding:
  - `pg_dumpall` of the Postgres instance;
  - ClickHouse `BACKUP DATABASE … TO Disk('backups', …)`, using a local backup disk configured in
    ClickHouse;
  - a tar of the `lakehouse_connector_secrets` volume;
  - a copy of `.env`, `release.json` and `state.json`;
  - a `manifest.json` with each part's size and SHA-256.

  Restore checks those checksums first. S1 builds this as scripts and the `backups` profile
  gains a local-disk target. S4's `lakehousectl backup` calls the same steps.
- **D9. Update safety.**
  - Before backing up, `lakehousectl update` stops the writers: the API, the Dagster daemon and
    the code location. Postgres, ClickHouse, RustFS and Lakekeeper stay up so they can be backed
    up.
  - It sets `MAINTENANCE_PAUSED=true` in `.env`, so the nightly expiry and orphan jobs skip until
    `lakehousectl update` clears it after success or after rollback.
  - The update succeeds only when `/health/ready` reports the NEW version as ready within 10
    minutes. On a timeout or any non-ready state, it rolls back:
    1. stop;
    2. restore the backup;
    3. pin the previous digests;
    4. start;
    5. wait for ready on the old version.
  - It reports both outcomes.
- **D10. Upgrade path.** `release.json` has `minFrom` (the lowest version that may update
  directly) and `stops` (versions that must be passed through). `lakehousectl update` computes
  the path and runs each hop with its own backup. It refuses a downgrade other than through
  `rollback`.
- **D11. Bundle.** One file, `rantai-lakehouse-<version>-linux-x86_64.tar.zst`, holding:
  - `lakehousectl`;
  - `release.json` and `release.json.sigstore.json`;
  - `docker-compose.yml`;
  - `images/<name>.tar` (`docker save` per image, so `docker load` needs no registry);
  - `sbom/<name>.cdx.json`;
  - `SHA256SUMS`.

  Its size is printed in the release notes.
- **D12. Hosts.** Linux x86_64 first. arm64 follows when every third-party image in the stack
  is checked to have an arm64 build.

## 3. S1 tasks (start now)

Branch `feat/rel-2-s1-prerequisites` off `main`. One commit per task, in this order.

**S1-T1. Readiness and version** (D7). Rust, `lakehouse-api`.
- Add `/health/ready` next to `/health` (`policy.rs:157` area).
- Add `version` to `/health` too.
- The schema check compares the highest applied migration with the highest one embedded in the
  binary.
- Tests: ready; not ready when the database is behind; no upstream text in the body; the
  public-route count (`policy.rs:1266`).
- *Accept:* `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api health`; `cargo test -p lakehouse-api policy`.

**S1-T2. The console in Compose.** Add a `console` service built from `Dockerfile.frontend`.
- `RUST_API_URL` points at the API service.
- Publish it on `${CONSOLE_HOST_PORT:-3000}`.
- It depends on `lakehouse-api` being healthy, with a healthcheck on its own port, and
  `restart: unless-stopped`.
- Read `deploy-staging.yml:125-141` for how staging runs it, and match it.
- `README.md`'s quick start drops the host `bun … next dev` step for operators, and keeps it as
  the developer path.
- *Accept:* `docker compose --profile '*' config --quiet`, then a real `up` of the default
  profile from a clean project: the console answers on its port and signs in. Quote it.

**S1-T3. Every image pinned by digest.**
- Every `image:` in `docker-compose.yml` becomes `name:tag@sha256:…`, as Debezium already is
  (`:3263`). Floating tags become fixed ones.
- Add a repo lint, `ops/lint/check_image_digests.py`, that fails on an `image:` without a digest,
  except the locally built ones, which are named `${…_IMAGE:-…:local}`. Wire it into the
  "Repo lints" CI job.
- *Accept:* the lint passes, `config --quiet` passes, and the default-profile `up` from S1-T2
  still works.

**S1-T4. A complete backup to local disk** (D8).
- Extend `ops/backups/` and `scripts/restore-from-backup.sh`:
  - a local-disk target (no S3 needed);
  - the connector-secrets volume and `.env` included;
  - per-part checksums, checked before a restore.
- The S3 target stays as an option.
- *Accept:* on a clean project, seed a connector with a stored credential, back up, destroy the
  volumes, restore, and show that the connector's credential still decrypts. Quote the commands.
  Update `OPERATIONS.md:1410-1530`.

**S1-T5. The console shows its version** (TypeScript).
- The console build gets its version from `package.json`.
- The account menu shows "Version x.y.z".
- When `/health`'s `version` differs from the console's, a small banner says "The server runs
  x, this console is y. Reload, or ask your operator to update." This is T3 Code's pattern.
- Tests for both.
- *Accept:* `bun run typecheck`, `bun run lint`, and the touched tests.

**S1 verification before handoff:**
- `cargo fmt --check`, plus scoped clippy and tests (CI runs the workspace);
- `bun run typecheck && bun run lint`, plus the touched tests;
- `docker compose --profile '*' config --quiet` and the real `up` above;
- `python3 ops/lint/check_image_digests.py`;
- `git status`.

## 4. S2–S4 outline (each gets a Plan check before it starts)

**S2. Release pipeline** (`.github/workflows/release.yml`, on `v*` tags and on manual dry runs).
1. Build the product images: `lakehouse-api`, the Dagster image, `oidc-mock` and `console`, for
   x86_64.
2. Push them to `ghcr.io/rantai-dev/…` by digest.
3. Sign each with cosign (D3).
4. Attach a CycloneDX SBOM per image, with `syft`.
5. Render the release `docker-compose.yml`, where every local `build:` becomes the published
   image at its digest.
6. Write `release.json` (D10) and sign it.
7. Build the bundle (D11).
8. Create the GitHub Release with the bundle, the manifest, the signature, the SBOMs,
   `SHA256SUMS` and `install.sh`.

A dry run on a branch uses a throwaway key and does not publish.

**S3. `lakehousectl install`.** A new crate, `rust/crates/lakehousectl`, built static with musl.
- Pre-flight (spec minimums; `--ignore-preflight=<check>` to override one, which is logged).
- Verify the manifest against the embedded key.
- Get the images: pull by digest online, or `docker load` from the bundle, then compare the
  loaded image ids against the manifest.
- Write the D5 layout and D6 secrets.
- `docker compose up -d`, then wait for `/health/ready` and the console.
- Print the address and the first sign-in.
- `install.sh` detects the architecture, downloads `lakehousectl` and `SHA256SUMS` from the
  release, checks the checksum, and runs `lakehousectl install "$@"`.
- Tests: unit tests (pre-flight, manifest verification with a test key, secrets idempotence),
  plus a CI job that installs from a dry-run bundle on a clean runner.

**S4. Update and the rest.** Implements:
- `update` (D9, D10);
- `backup` and `restore` (D8);
- `rollback`;
- `status`;
- `self-update`.

A CI job, "upgrade test", proves it on each release:
1. Install the previous release.
2. Seed data.
3. Update to the candidate.
4. Assert the data and version.
5. Force a failure (an image that never becomes ready) and assert the automatic rollback and the
   old data.

**S5. Release.**
- Sign the release policy (D4) and write the upgrade notes.
- Cut 0.2.0 through the pipeline.
- Measure install time and bundle size into `docs/plans/REL-2-RESULT.md`.

## 4a. S2 plan check (planner, 2026-10-31, at `main` `f9dffa86`) — the release pipeline

Read today:
- **`.github/workflows/docker.yml`** builds only the API, on `v*` tags, with `push: false`.
  Its `push-ghcr` job is an inert `echo`. **S2 replaces this file.**
- **Locally built images:** `lakehouse-api` (context `rust`, extra build context
  `pipeline_src=dagster/dispar_orchestrate`), the Dagster image (context `dagster`), `oidc-mock`
  (context `ops/oidc-mock`) and `console` (context `.`, `Dockerfile.frontend`). The gate-only
  builds (g7 runner, g6 stubs) are not released.
- **Versions live in four places:**
  - the tag;
  - `package.json` `version`;
  - `rust/crates/lakehouse-api/Cargo.toml` `version` (what `CARGO_PKG_VERSION` and
    `/health/ready` report);
  - the `CHANGELOG.md` `## [x.y.z]` heading.
- **The signing secrets** are in the `release` environment (D3), and the public key is
  `ops/release/release.pub`.

Branch `feat/rel-2-s2-release-pipeline` off `main`. It already holds this plan update, the
signed decisions and `ops/release/release.pub`. One commit per task.

- **S2-T1. `ops/release/check_version.py <tag>`.**
  - It fails unless the tag, `package.json`, the API's `Cargo.toml` and a
    `## [x.y.z] - YYYY-MM-DD` heading in `CHANGELOG.md` all name the same version.
  - Add pytest cases for: pass; each one of the four differing; a missing changelog heading.
- **S2-T2. `ops/release/render_compose.py --images <json> -o <file>`.**
  - It writes the release's Compose file from `docker-compose.yml`.
  - Each locally built service loses `build:` and gets `image: <published ref>@sha256:…`.
  - Every service whose profiles are only gate or test profiles (`test`, `g*`) is dropped. The
    default, `dagster`, `trino`, `seaweedfs` and `backups` services are kept.
  - It fails if any image left has no digest. Reuse `check_image_digests.py`'s parser, do not
    copy it (rule 4).
  - Tests: no `build:` remains; every `image:` has a digest; no gate service remains;
    `docker compose -f <out> config --quiet` passes in CI.
- **S2-T3. `ops/release/make_manifest.py` and `ops/release/make_bundle.sh`.**
  - `release.json` holds `schemaVersion: 1`, `version`, `gitSha`, `createdAt`, `minFrom`,
    `stops: []`, `images` (`service → ref@sha256`) and `files` (`name → sha256`; the Compose
    file now, `lakehousectl` from S3).
  - `minFrom` defaults to two minor versions back, per the spec, and can be overridden from a
    checked-in `ops/release/upgrade-path.json`. That file is the one place a release names a
    required stop.
  - The bundle is D11 without `lakehousectl` until S3. Images are saved with `docker save` by
    digest reference, and it carries `SHA256SUMS`. Compression is `zstd -19 -T0`. It prints its
    size.
  - Tests for the manifest's shape, and for the checksums matching the files.
- **S2-T4. `.github/workflows/release.yml`** (replaces `docker.yml`).
  - **Triggers:** `push` of `v*` tags (real), `workflow_dispatch`, and `pull_request` on paths
    `.github/workflows/release.yml`, `ops/release/**`, `Dockerfile.frontend`,
    `rust/Dockerfile`, `dagster/Dockerfile` and `ops/oidc-mock/**`. The last two triggers are
    dry runs.
  - **Real run:**
    1. `check_version.py`;
    2. build the four images and push them to `ghcr.io/rantai-dev/rantai-lakehouse-<name>`
       tagged with the version, recording each digest;
    3. for each, `cosign sign --key env://COSIGN_PRIVATE_KEY <ref>@<digest>`, then attach a
       CycloneDX SBOM (`anchore/sbom-action`, then `cosign attest --type cyclonedx`);
    4. render the Compose file and write the manifest;
    5. `cosign sign-blob` the manifest into `release.json.sigstore.json` (D3a), then verify it
       at once with `ops/release/release.pub`;
    6. build the bundle;
    7. `gh release create` as a **draft**, with the bundle, its `.sha256`, `release.json`, the
       `.sigstore.json`, the SBOMs, `SHA256SUMS` and the Compose file. The owner publishes the
       draft.

    Signing steps run in a job bound to `environment: release`, so the owner's approval gates
    the key.
  - **Dry run:** the same steps, except:
    - images go to a job-local `registry:2` service, so the digests are real;
    - the key is generated in the job and verified against its own public key;
    - nothing is published;
    - the bundle and manifest are uploaded as workflow artifacts. The step summary prints the
      bundle size.
  - **Least privilege:** `permissions:` is `contents: read` by default, `packages: write` and
    `contents: write` only in the real job, and no `pull_request_target`. The real job never runs
    for a PR.
- **S2-T5. Docs.**
  - `docs/CI.md` gains "Release pipeline": what runs, the dry run, the key, the draft, and the
    one manual step after the first real release, making the GHCR packages public.
  - `docs/OPERATIONS.md` gains a pointer.
  - `CHANGELOG.md` gets a REL-2 line under Unreleased.
- **S2 accept:**
  - the pytest files pass;
  - `bash -n` and `shellcheck` (if present) pass on the bundle script;
  - `actionlint` (if present) passes on the workflow;
  - **the PR's own dry run is green**, and its summary shows the bundle size. Quote it in the
    handoff.
  - The developer cannot run the real release; the planner and the owner do that in S5.

## 5. Out of scope

Kubernetes or Helm, multi-host, zero-downtime updates, Windows and macOS hosts, automatic
unattended updates, reversible migrations, and arm64 (D12).

## 6. Handoff (developers append below, one entry per slice)

### Handoff S1 (developer, 2026-10-31)

**Commits** (branch `feat/rel-2-s1-prerequisites`, base `main` at `677845db`), one per task, in
plan order:

| Commit | Task | What |
| --- | --- | --- |
| `999d79cc` | S1-T1 | `feat(api)`: a readiness route and the build's version (D7) |
| `f324ced0` | S1-T2 | `feat(compose)`: the console runs in the stack |
| `0ff64e87` | S1-T3 | `feat(compose)`: every third-party image is pinned by digest |
| `5442428c` | S1-T4 | `feat(backup)`: a complete backup on the local disk (D8) |
| `64506418` | S1-T5 | `feat(console)`: the console shows its version |

**Commands, with their results.** Every cargo command went through `bin/cargo-locked`, in the
foreground, on the shared `CARGO_TARGET_DIR`; no `cargo clean`.

S1-T1:

- `cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -- -D warnings` — clean.
  (`lakehouse-store` is in the command because the schema-version helpers live there: it owns
  the migration set, so it answers both "what does this build expect" and "what does the
  database have".)
- `cargo test -p lakehouse-api health` — 32 passed (lib target), 1 passed (bin target), 0 failed.
- `cargo test -p lakehouse-api policy` — 69 passed (lib), 69 passed (bin), 0 failed.
- `cargo test -p lakehouse-api --test sec11_guard` — 4 passed, 0 failed (the new route carries
  no upstream text, and the guard says so).

S1-T2:

- `docker compose --profile '*' config --quiet` — clean.
- `docker compose -p rel2 up -d --build` from a clean project — the stack came up;
  `rel2-lakehouse-api-1 Healthy`, `rel2-console-1 Started`.
- `curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/` → `200`.
- `curl -X POST http://localhost:3000/api/auth/login` (the bootstrap admin) → `200` with
  `{"id":"…","name":"Bootstrap Admin","mustChangePassword":true}` — the console signs in
  through its own port.
- `docker inspect rel2-console-1 --format '{{.State.Health.Status}}'` → `healthy`.

S1-T3:

- `python3 ops/lint/check_image_digests.py` — "46 third-party image(s) pinned by digest, 8
  locally built image(s) exempt", exit 0.
- `python3 -m unittest discover -s ops/lint -p 'test_*.py'` — 33 tests, OK.
- `docker compose --profile '*' config --quiet` — clean.
- `docker compose -p rel2 down -v` then `docker compose -p rel2 up -d --build` — the stack came
  up healthy, `GET /health/ready` answered `200` with all three checks `"ok"`, and the console
  signed in on `:3000`.

S1-T4 (the drill, on a clean `-p rel2` project):

- Seeded a connector with a stored credential (`POST /api/connectors`, `credential.source:
  "managed"`), and saw the sealed file in the volume:
  `connector_managed_conn_backup_drill_connector_mvwmoq7x_password`.
- `COMPOSE_PROJECT=rel2 ops/backups/backup-local.sh /tmp/opencode/rel2-backups` → 19 parts,
  439,710 bytes, `manifest.json` with every part's size and SHA-256; the tar holds the sealed
  credential.
- `docker compose -p rel2 down -v` — every volume gone.
- `COMPOSE_PROJECT=rel2 scripts/restore-from-backup.sh --local <backup-dir>` → 32 s in total
  (Postgres 8 s, ClickHouse 7 s, the connector secrets 1 s, the configuration 0 s, the stack
  16 s). The manifest's checksums were checked before anything was written.
- The credential still decrypts: `docker compose -p rel2 exec -T lakehouse-api
  lakehouse-api rotate-secrets-key` → "0 resealed, 0 sealed from plain, 1 already under the
  current key, 0 failed" — the API's own key rotation opened the restored sealed file with the
  restored key. The connector's row is back too (`SELECT id, name, tenant, secret_ref FROM
  connector` → `conn-backup-drill-connector-mvwmoq7x`).
- `bash -n` on `ops/backups/backup-local.sh` and `scripts/restore-from-backup.sh` — clean.

S1-T5:

- `bun install --frozen-lockfile` — 739 packages.
- `bun run typecheck` — clean.
- `bun run lint` — 0 errors, 10 warnings, all pre-existing (none in a touched file).
- `bun test src/features/version src/components/app-shell/app-navbar.test.tsx` — 6 pass, 0 fail.
- `bun test src/components` — 40 pass, 0 fail (the app shell, which the account menu lives in).
- Live on the `-p rel2` stack: `GET http://localhost:3000/api/health` →
  `{"status":"ok","database":"ok","version":"0.1.0"}`, and the console's own `/health` page
  still answered `200` (the new rewrite does not shadow it).

**The S1 verification block, on the final commit:**

- `cargo fmt --check` — clean.
- `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -- -D warnings` — clean.
- `cargo test -p lakehouse-api health` — 33 passed (lib), 31 passed (bin), 1 passed, 0 failed.
- `cargo test -p lakehouse-api policy` — 69 passed (lib), 69 passed (bin), 0 failed.
- `bun run typecheck` — clean. `bun run lint` — 0 errors, 10 pre-existing warnings.
- `docker compose --profile '*' config --quiet` — clean.
- `python3 ops/lint/check_image_digests.py` — 46 pinned, 8 exempt.
- `python3 ops/lint/check_intra_package_imports.py` — OK.
- `python3 ops/lint/check_bare_iceberg_count.py` — no violation.
- `python3 ops/lint/check_compose_init_readiness.py` — OK.
- `python3 ops/lint/check_connector_gate_coverage.py` — OK (19 types covered, 0 waived).
- The default profile's `up` from a clean project (`-p rel2`): every service healthy,
  `GET /health/ready` → `200` with all three checks `"ok"`, the console answered `200` on
  `:3000` and signed in.
- `git status` — clean (nothing generated staged).

**Not verified, with the reason:**

- The full workspace run (`cargo clippy --workspace --all-targets --all-features` and
  `cargo test --workspace`): not run here — this slice's machine rules say CI runs the
  workspace and only the scoped commands are run locally. The two crates this slice touches
  (`lakehouse-api`, `lakehouse-store`) were clipped and tested, with `--all-targets`.
- `(cd dagster && python -m pytest dispar_orchestrate -q)`: not run — this slice changes no
  Python in the code location (`dagster/dispar_orchestrate/` is untouched). The repo lints that
  scan Python did run.
- The images that only the optional profiles use (`mysql`, `mcr.microsoft.com/mssql/server`,
  `mongo`, `apache/kafka`, `chrislusf/seaweedfs`, `trinodb/trino`, `rust`, `alpine/socat`,
  `alpine`, `amazon/aws-cli`, `curlimages/curl`, `openfga`, `gotenberg`,
  `quay.io/lakekeeper/catalog`, `rustfs`): their digests were read from the registry with
  `docker buildx imagetools inspect`, and one of them (`alpine/socat:1.8.1.3`) was pulled to
  prove the pinned reference resolves. The profiles that use the others were not started, so
  those digests are not proven by a run here.
- The account menu and the version banner: proved by their unit tests and by the live
  `/api/health` answer. No browser was driven in this slice, so no screenshot shows the menu.

**Where the plan left room, and what was decided** (for the reviewer):

- The readiness probe's `lakekeeper` check asks for `{catalog}/v1/config?warehouse=…`:
  `Lakekeeper` answers `400` ("No warehouse specified") without the warehouse parameter —
  measured against the running v0.13.3. A probe without it would call a working catalog
  unhealthy.
- The three readiness checks are bounded by the same 3 s as the service probes
  (`crate::health::PROBE_TIMEOUT`, now `pub(crate)` and shared).
- The console's own version is read by importing `package.json` (the bundler inlines it at
  build time), not by a build argument a deployment would have to keep in step.
- The API's compose healthcheck asks `/health` (liveness), not `/health/ready`: the API serves
  its Phase 1 routes with Postgres down, so "the process answers" is the honest question for
  compose, and the console should come up and show what is broken. The readiness route is what
  an installer or an updater waits on.
- The local backup is a host-side script (`ops/backups/backup-local.sh`), not a new service in
  the `backups` profile: it needs no new image and no new service in the default stack, and it
  runs the same steps S4's `lakehousectl backup` will call. The compose `backups` profile keeps
  its S3 job, unchanged.
- The `ClickHouse` local backup disk is configured in `ops/backups/clickhouse/backups.xml`
  (mounted into the `clickhouse` service) and its files live in a named volume the script
  copies out of — and the restore copies back into.
- The repository root now carries a tracked `.dockerignore`: the console image's build context
  IS the repository root, and without that file the image carried the deployment's `.env`
  (generated credentials) and the git history.
- The console reaches the API's health route through a new `/api/health` rewrite (the API's
  route is at `/health`, not under `/api/`). The console's own `/health` page is a filesystem
  route and is served before any rewrite, so the two cannot collide — checked live.

**Known gaps in this slice** (not fixed here; the reviewer decides):

- The S3 restore path in `scripts/restore-from-backup.sh` still waits for Postgres and
  ClickHouse without failing when they do not come up. The local path added for this slice fails
  closed. The S3 path's behaviour is unchanged, on purpose ("the S3 target stays as an option").
- The gate harnesses' own compose override files still carry unpinned images; the new lint reads
  `docker-compose.yml` only, and says so.
- The backup carries no `release.json` and no `state.json` yet — they do not exist until S2 and
  S4. The script says so in its output rather than pretending.
- `CONSOLE_INTERNAL_URL` (the console as the PDF renderer reaches it) is still empty by default,
  so the PDF export stays off until it is set to `http://console:3000`.

### Handoff S2 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s2-release-pipeline`, base `main` at `f9dffa86`), one per task, in
plan order:

| Commit | Task | What |
| --- | --- | --- |
| `729f1a84` | S2-T1 | `ops/release/check_version.py`: the tag, the package, the crate and the changelog must agree |
| `a7abea01` | S2-T2 | `ops/release/render_compose.py`: the release's Compose file, rendered from the template |
| `d2759368` | S2-T3 | `ops/release/make_manifest.py` and `ops/release/make_bundle.sh`: the manifest and the offline bundle |
| `184b95d5` | (S2-T3) | the bundle script lost an unused variable `shellcheck` flagged |
| `36065cf4` | S2-T4 | `.github/workflows/release.yml` (replaces `docker.yml`) |
| `8d91e514` | S2-T5 | the docs: `docs/CI.md`, `docs/OPERATIONS.md`, `CHANGELOG.md` |

**Commands, with their results.** Everything ran in the foreground on this machine.

The tooling's own tests:

- `python -m pytest ops/release -q` (pytest 9.2.0 in a throwaway venv) — **52 passed, 0 failed**.
- `python3 -m unittest discover -s ops/release -p 'test_*.py'` — **52 tests, OK** (the same
  files are `unittest.TestCase` classes, so they run under either runner).
- `python3 ops/release/check_version.py v0.1.0` — exit 0 ("the tag, package.json, the API
  crate and the changelog all name 0.1.0"); `… v0.2.0` — exit 1 with all three differences
  printed (the tag names 0.2.0, the package and the crate name 0.1.0, the changelog has no
  `## [0.2.0]` heading).

The scripts and the workflow:

- `bash -n ops/release/make_bundle.sh` — clean.
- `shellcheck ops/release/make_bundle.sh` (shellcheck 0.10.0, downloaded to `/tmp`; it is
  **not** installed on this machine) — **clean**. It found one thing on the first run
  (`SC2034`: `SCRIPT_DIR` unused) and that is what commit `184b95d5` removes.
- `actionlint .github/workflows/release.yml .github/workflows/ci.yml` (actionlint 1.7.12,
  installed with `pip install actionlint-py` into the throwaway venv; it is **not** installed
  on this machine) — **`release.yml` is clean**; `ci.yml` reports three shellcheck warnings
  (SC1007 at line 2070, SC2034 and SC2155 at line 2929). They are **pre-existing**: the same
  warnings are on `main` (lines 2024 and 2883 there), in jobs this slice does not touch, and
  `ci.yml` does not run `actionlint`. Not fixed here — fixing another job's shell would
  widen this slice's diff, and it needs a backlog item.

The repo's lints and the Compose files:

- `python3 ops/lint/check_image_digests.py` — "46 third-party image(s) pinned by digest, 8
  locally built image(s) exempt", exit 0.
- `python3 ops/lint/check_intra_package_imports.py` — OK.
- `python3 ops/lint/check_bare_iceberg_count.py` — no violation.
- `python3 ops/lint/check_compose_init_readiness.py` — OK.
- `python3 ops/lint/check_connector_gate_coverage.py` — OK (19 types covered, 0 waived).
- `python3 -m unittest discover -s ops/lint -p 'test_*.py'` — 33 tests, OK.
- `python3 -m unittest discover -s ops/gates -p 'test_*.py'` — 29 tests, OK.
- `docker compose --profile '*' config --quiet` — clean (the template, with the local `.env`).
- `docker compose --env-file <throwaway> -f <rendered> --profile '*' config --quiet` —
  **clean**, twice: once with the placeholder digests CI uses, and once against the
  rendered file the pipeline writes. The rendered file is 2,863 lines and holds 35 services
  (the 57 of the template, less the 22 that are gate or test harnesses).

**The bundle, end to end, with a real image** (the one local image build this slice was
allowed):

- `docker compose -p rel2s2 build lakehouse-api` — the API image, built (mostly cached).
- A throwaway `registry:2` on `localhost:5000`; the image pushed to it, so it had a REAL
  digest (`sha256:b48a99a6…`).
- `ops/release/render_compose.py`, `ops/release/make_manifest.py` and
  `ops/release/make_bundle.sh` ran over it: the bundle is
  `rantai-lakehouse-0.2.0-linux-x86_64.tar.zst`, **76,508,621 bytes (72 MiB)**.
- The bundle holds `docker-compose.yml`, `release.json`, `release.json.sig`,
  `images/rantai-lakehouse-api.tar`, `sbom/rantai-lakehouse-api.cdx.json` and
  `SHA256SUMS`; `sha256sum -c SHA256SUMS` says **OK for all five files**.
- `docker load` of the bundle's image tar restored an image whose ID is **exactly the digest
  the manifest names** (`sha256:b48a99a6…`) — which is the comparison REL-2 S3's installer
  will make.
- Cleanup: the registry container, the pushed image, the compose project and the scratch
  directory are gone. The API image itself stays in the local store (it is a build
  artifact, not tracked by git).

**Not verified, with the reason:**

- **The dry run is not verified.** It needs a GitHub runner: the four images, the throwaway
  registry, cosign, `syft` and the bundle are all CI-side. The PR's own dry run is the
  proof, and its summary is where the bundle's size appears. Nothing in this slice ran a
  `cosign` command: `cosign` is not installed on this machine, so the signing, the
  attestation and the `sign-blob`/`verify-blob` pair are unexercised here. The dry run
  generates its key in the job and verifies against its own public key, so it cannot touch
  the release key; the real run signs with `env://COSIGN_PRIVATE_KEY` from the `release`
  environment.
- **The real release is not verified, and cannot be.** It needs a `v*` tag, the owner's
  approval on the `release` environment, and the release key. REL-2 S5 does that.
- `docker save` was exercised with one real image (the API's). The other three images were
  not built locally — the PR's dry run builds all four.
- `(cd dagster && python -m pytest dispar_orchestrate -q)`: not run — this slice changes no
  Python in the code location (`dagster/dispar_orchestrate/` is untouched) and no Dagster
  file. The repo lints that scan Python did run.
- No cargo command ran: this slice changes no Rust. The API crate's version was READ
  (`rust/crates/lakehouse-api/Cargo.toml`) by `check_version.py`, not changed.
- The rendered Compose file was checked with `docker compose config`, never STARTED: no
  release image exists yet, so there is nothing to start. Its contents are proved by the
  tests and by `config`; that it boots is REL-2 S3's job (it installs a release).

**Where the plan left room, and what was decided** (for the reviewer):

- **Which services the release drops.** The plan says "every service whose profiles are
  only gate or test profiles (`test`, `g*`) is dropped". That rule alone keeps the gate
  runners that live in the `dagster` profile (`g3a-test-runner`, `g4-test-runner`,
  `gold-export-test-runner`, the G7 runner …), and the G7 runner is built from `ops/g7` —
  a build the plan says is NOT released ("the gate-only builds (g7 runner, g6 stubs) are not
  released"). So the renderer recognises a harness by its PROFILE **or** by its NAME
  (`g<digit>…` or `…-test-runner`), and drops both.
- **The one harness that stays.** `debezium-server` waits for `g4-source-init` (the demo
  table `p5_cdc.orders` and its publication `p5cdc_pub`) with
  `condition: service_completed_successfully`. Dropping it would leave the CDC service
  waiting for something that never runs, and the Debezium server would fail on a missing
  publication. The renderer therefore keeps a harness that a KEPT service waits for — the
  exception is computed from the `depends_on` graph, not from a list of names. Everything
  else that is a harness is dropped (22 services).
- **The service → image mapping lives in one checked-in file.**
  `ops/release/release-images.json` says which published image each service runs. The
  pipeline and the CI check both read it, so they cannot drift; a test holds it to the
  template (every service the template builds appears in it, and vice versa).
- **The release Compose file keeps the template's comments.** A comment that talks about
  building an image, or about a local `:local` tag, describes the TEMPLATE, not the
  rendered file. The rendered file says so in its own header, in the first lines, rather
  than dropping the comments an operator reads for the "why" of each setting.
- **`minFrom`'s default.** Two minor versions back: `0.2.0` → `0.0.0`, `0.3.0` → `0.1.0`.
  Below `0.2.0` there is no such version and the floor is `0.0.0`. A stop is named in
  `ops/release/upgrade-path.json`, which ships with no entries (no release has named one).
- **`install.sh` is attached when it exists.** The plan lists it among the release's
  assets; it is written in REL-2 S3. The workflow copies it from `ops/release/install.sh`
  when that file is there and says nothing when it is not, so the release is never
  published with a missing asset.
- **The release's own `SHA256SUMS`.** The bundle carries one over its contents; the
  release carries one over its assets (the bundle, the manifest, the signature, the
  SBOMs), so an operator who downloads them can check them against the release page.
- **The buildx builder in the dry run** runs with `driver-opts: network=host`, because a
  builder runs in a container of its own and `localhost:5000` inside it is the builder, not
  the runner. The real run needs no such thing: it pushes to `ghcr.io`.

**Known gaps in this slice** (not fixed here; the reviewer decides):

- **The offline path cannot compare a loaded image with the manifest's digest yet.** The
  manifest records the REGISTRY digest of each image (`repo@sha256:…`), which is what
  `docker pull` resolves. `docker load` of a bundle's tar restores the image's ID (its
  config digest) and, on a daemon with the classic image store, NOT its `RepoDigests` — so
  an installer cannot compare the loaded image against the manifest's digest as the plan
  words it ("compare the loaded image ids against the manifest"). On this machine the
  daemon has the containerd store and the loaded image DID keep its `RepoDigests`, so the
  comparison worked here; on a classic daemon it would not. REL-2 S3 has to settle this:
  either the manifest also carries each image's config digest, or the installer checks the
  bundle's own `SHA256SUMS` chain instead. **Nothing in S2's output blocks either choice**,
  but the choice is S3's.
- The dry run's build has no `cache-to`: it only READS the `api-image` and `dagster-image`
  caches `ci.yml` writes from `main`, so a dry run never evicts them.
- The images are signed with a key that lives in the `release` environment; there is no
  key rotation procedure yet, and no record of which key signed which release beyond the
  signature itself.
- `cosign attest` pushes the SBOM attestation to the registry as a referrer (or, on a
  registry without the referrers API, as a tag). The dry run's `registry:2` and GHCR both
  accept it, but the dry run is the only place this is exercised — by the PR's CI.
- The `release` environment's secrets (`COSIGN_PRIVATE_KEY`, `COSIGN_PASSWORD`) are
  assumed to hold the base64 of the encrypted private key and its password, which is what
  cosign's `env://` provider expects. The plan does not state the encoding; the workflow
  says so in a comment. The FIRST real release is where that assumption is proven, and it
  fails loudly (cosign cannot decrypt) if it is wrong.

## 7. Review (planner appends below)

### Review S1 round 1 (planner, 2026-10-31, at `f79d4235`)

**What holds.**
- **T1.** `/health/ready` reports only `ok` or `failed` per check, never upstream text, and is
  ready only when the schema matches the binary. `/health` carries the version. The public count
  is 14.
- **T2.** The console service has no default secret. It depends on the API being healthy. The
  new root `.dockerignore` keeps `.env`, `.git` and `ops` out of the console image.
- **T3.** 46 images are pinned by digest, and the lint is in CI.
- **T4.** The backup directory is `0700`, and parts are checksummed. The drill proved a stored
  connector credential decrypts after restore.
- **T5.** The version banner follows T3 Code's pattern.
- **Two choices the plan left open, accepted:**
  - the readiness probe passes `warehouse=`, because Lakekeeper answers 400 without it;
  - the local backup is a host-side script, not a new service.

**SHOULD-FIX 1: the public readiness route fans out with no cap.**
- Every unauthenticated `GET /health/ready` runs a Postgres query, a ClickHouse query and a
  Lakekeeper HTTP call, which includes a token file read. A loop of anonymous requests turns into
  three times as much backend load.
- `probe_lakekeeper` also builds a new `reqwest::Client` each time, with redirects on.
- **Fix:**
  - Cache the computed `ReadyBody` for 2 seconds. Concurrent callers share one probe:
    `tokio::sync::Mutex` plus a stored `Instant`, or a `OnceCell` per window.
  - Use one shared client (`OnceLock`) with `redirect::Policy::none()` and the probe timeout.
  - Add a test: two calls within the window run the probe once (count with a test hook or a
    `wiremock` for Lakekeeper).
  - Cite `REL-2 S1 review SHOULD-FIX 1`.

**Open: SHOULD-FIX 1.**

### Fix for review round 1 (developer, 2026-11-01)

**SHOULD-FIX 1 — fixed in `32d940d5`** (branch `feat/rel-2-s1-prerequisites`).

- **The 2 s cache.** `readiness::ready_body` (the route's new entry point) reuses the computed
  `ReadyBody` for `CACHE_WINDOW = 2 s`. The cache is `readiness::ReadyCache` — a
  `tokio::sync::Mutex` over one `(ReadyBody, Instant)` entry — and the lock is held ACROSS the
  probe, so callers that arrive together share ONE probe instead of each running their own. The
  window is measured from the END of the probe, so a cached answer is never older than the window
  says it is.
- **Where the cache lives.** In `AppState` (`readiness_cache`), the same shape as
  `health::HealthCache`: the cache belongs to the deployment it answers for, not to the process,
  so a test's own state has its own cache and no test can see another's answer.
- **One HTTP client.** `readiness::http_client` is a `OnceLock` holding the ONE shared
  `reqwest::Client`, built with `redirect::Policy::none()` and `PROBE_TIMEOUT` as its own
  timeout. Redirects are off because the probe answers with the catalog's reader token: a
  redirect must not carry that token to another host. A client that cannot be built at all fails
  the catalog check closed (`"failed"`), with the reason in the log — never a panic.
- **Tests** (`readiness::tests`):
  - `two_readiness_answers_within_the_window_probe_once` — a `wiremock` catalog counts the
    probes: two answers inside the window produce exactly ONE request to `/v1/config`.
  - `a_read_past_the_window_re_probes` — the test's own clock, no sleeping: a read past the
    window re-probes.
  - `the_catalog_probe_does_not_follow_a_redirect` — a `302` from the catalog leaves the check
    `"failed"` and the redirect target is never requested.
- **Cited at the fix site** as `REL-2 S1 review SHOULD-FIX 1` (the module doc, the cache
  constant, the client, the probe, the route's doc comment, `AppState`'s field and the three
  tests), and in the commit body.

**Commands run, in the foreground, through `bin/cargo-locked`** (all on `32d940d5`):

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean, no diff |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean, no warning and no error |
| `cargo test -p lakehouse-api readiness` | **10 passed, 0 failed**, 2854 filtered out (the lib and the bin binary each run the 10 unit tests) |
| `cargo test -p lakehouse-api health` | **33 passed, 0 failed**, 2869 filtered out (the lib and the bin binary each run the 33 unit tests) |

**Not verified, with the reason:**

- The full crate suite (`cargo test -p lakehouse-api` over every target) and the full workspace
  run: not run here — this slice's machine rules say CI runs them; the scoped commands above are
  what runs locally. The crate was, however, clipped with `--all-targets`, so every test target
  compiles.
- The fix was not exercised against a live `Postgres`/`ClickHouse`/`Lakekeeper` stack: the
  behaviour it changes (how many backend calls a burst of requests causes) is proven by the
  `wiremock` count, not by a live run.

### Review S1 round 2 (planner, 2026-10-31, at `af17539e`)

SHOULD-FIX 1 is fixed (`32d940d5`).
- The ready body is cached for 2 seconds in `AppState`.
- The lock is held across the probe, so concurrent callers share one probe.
- There is one `OnceLock` client with redirects off and the probe timeout.
- The tests count exactly one catalog call for two answers in the window, re-probe after it, and
  refuse a redirect.

**Open: no BLOCKER, no SHOULD-FIX.** CI runs the full workspace and the compose jobs.

### Review S2 round 1 (planner, 2026-10-31, at `14ced118`)

**What holds.**
- The real job runs only on a `v*` tag push, in the `release` environment. The dry run never
  needs a secret, and there is no `pull_request_target`.
- The default `permissions` is `contents: read`.
- `check_version.py`, the renderer (which reuses the digest parser), the manifest and the bundle
  are covered by 52 tests.
- The one-image bundle check proved a loaded image id equals the manifest's digest.
- **Accepted:** the renderer keeps `g4-source-init`, because `debezium-server` waits for it.
- **Carried to the S3 plan check:** on a classic Docker image store, the offline path cannot
  compare a loaded image with its registry digest.

**BLOCKER 1: the real release cannot sign.**
- The `release` job sets `COSIGN_KEY: env://COSIGN_PRIVATE_KEY`, but nothing sets
  `COSIGN_PRIVATE_KEY`. `grep` finds no `secrets.COSIGN_PRIVATE_KEY` anywhere, so the first real
  tag fails at "Sign each image".
- The dry run uses a key file, so it cannot catch this.
- The comment also says the secret is "the base64 of the encrypted private key". It is the PEM
  that `cosign generate-key-pair` writes, which `env://` reads as is.
- **Fix:**
  - Set `COSIGN_PRIVATE_KEY: ${{ secrets.COSIGN_PRIVATE_KEY }}`, as in BLOCKER 2.
  - Make the dry run take the **same path**: export its throwaway private key's PEM into
    `COSIGN_PRIVATE_KEY` and sign with `env://COSIGN_PRIVATE_KEY`, so the dry run proves the
    real job's wiring.
  - Correct the comment.

**BLOCKER 2: the signing key is visible to every step of the release job.**
- `COSIGN_PASSWORD`, and after BLOCKER 1 the key, sit in the job-level `env:`. So every step
  sees them, including third-party actions pinned only by movable tags (`anchore/sbom-action@v0`,
  `sigstore/cosign-installer@v4.1.2`, the `docker/*` actions).
- **Fix:**
  - Move both secrets into the `env:` of only the three steps that use them: "Sign each image",
    the attest step and the `sign-blob` step.
  - In both jobs, pin every third-party action to a full commit SHA, with the version in a
    trailing comment (`uses: owner/action@<40-hex> # v4.1.2`).
  - Add a test or lint step, `ops/release/test_release_workflow.py` (it parses the YAML), that
    fails if:
    - a `secrets.COSIGN_*` appears in a job-level `env:`;
    - a `uses:` in `release.yml` is not pinned to a 40-hex SHA.

**Open: BLOCKER 1, BLOCKER 2.**

### Fixes for review round 1 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s2-release-pipeline`, base `cc9f7645`), one per blocker:

| Commit | Finding | What |
| --- | --- | --- |
| `f1d72644` | REL-2 S2 review BLOCKER 1 | the real release signs with the key it is given |
| `8a04449e` | REL-2 S2 review BLOCKER 2 | the signing key is visible to three steps only |

**BLOCKER 1 (`f1d72644`).** The release job now sets
`COSIGN_PRIVATE_KEY: ${{ secrets.COSIGN_PRIVATE_KEY }}`, so `env://COSIGN_PRIVATE_KEY` resolves; the
dry run takes the SAME path — the throwaway key's PEM goes into the environment through
`$GITHUB_ENV` and every signing step signs with `env://COSIGN_PRIVATE_KEY` — so the dry run proves
the real job's wiring. The comment that called the secret "the base64 of the encrypted private
key" is corrected (`env://` reads the PEM as it is); `docs/CI.md`'s secret table says the same now.
The dry run's empty-password comment is corrected too: cosign DOES encrypt the throwaway key with
that empty password — it asks for nothing — which the verified run below shows.

**BLOCKER 2 (`8a04449e`).** The two secrets are in the `env:` of the three steps that sign ("Sign
each image", "Attest each SBOM to its image", "Sign the manifest and verify the signature") and of
no other step; the job-level `env:` keeps only the `env://COSIGN_PRIVATE_KEY` reference, which is a
name and not the key. Every third-party action is pinned to a full commit SHA in BOTH jobs, the
release in a trailing comment. Each SHA was resolved from the tag the file used, with
`gh api repos/<owner>/<action>/git/ref/tags/<tag>`; all eight resolved to a commit object, so no
annotated tag had to be dereferenced:

| Action (the tag the file used) | SHA | The release that SHA is |
| --- | --- | --- |
| `actions/checkout@v7` | `3d3c42e5aac5ba805825da76410c181273ba90b1` | `v7.0.1` |
| `actions/setup-python@v7` | `6d4fd22b20465ccc0f7354476588e8df7c9ca0e0` | `v7.1.0` |
| `actions/upload-artifact@v7` | `cf430e030ddbb5b0abf93d22962f4752f3646cd9` | `v7.0.2` |
| `docker/setup-buildx-action@v4` | `f87e5991a6d7451dcb8d9637bfbc97413f497069` | `v4.4.1` |
| `docker/build-push-action@v7` | `c3c9e263c25d99ce0380d002d59b67737d91b0dc` | `v7.4.0` |
| `docker/login-action@v4` | `81c6f93d4bc991e510b342a00b419b3bb4efc5c6` | `v4.7.0` |
| `sigstore/cosign-installer@v4.1.2` | `6f9f17788090df1f26f669e9d70d6ae9567deba6` | `v4.1.2` |
| `anchore/sbom-action@v0` | `e22c389904149dbc22b58101806040fa8d37a610` | `v0.24.0` |

The comment is the release the SHA is: the tag the SHA was resolved from points at it (`v0` of the
SBOM action is `v0.24.0`). `ops/release/test_release_workflow.py` (new, 12 cases) parses the
workflow and fails when a `secrets.COSIGN_*` sits in a job-level `env:`, when a `uses:` is not pinned
to a 40-hex SHA, and when a pin does not name its release in a trailing comment. It holds the three
signing steps to carrying the secrets, and it runs the checks against synthetic workflows, so a
check that cannot fail is itself a failure. It parses with PyYAML, so the two pipeline jobs and
`ci.yml`'s "Unit-test the release tooling" job install `pyyaml==6.0.3` beside `pytest`.

**Commands run, in the foreground, on `8a04449e`:**

| Command | Result |
| --- | --- |
| `python -m pytest ops/release -q` | **64 passed, 0 failed** (52 before this round, 12 new) |
| `python3 -m unittest discover -s ops/release -p 'test_*.py'` | **64 tests, OK** |
| `actionlint .github/workflows/release.yml` | clean, exit 0 (shellcheck 0.10.0 on the PATH, so the `run:` blocks were checked too) |
| `bash -n ops/release/make_bundle.sh` | clean, exit 0 |
| `shellcheck ops/release/make_bundle.sh` | clean, exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | no violation |
| `actionlint .github/workflows/ci.yml` | the same three PRE-EXISTING shellcheck warnings as before this round (SC1007, SC2034, SC2155), now at lines 2071 and 2930 because this round added one comment line to that file; no new finding |

**The signing path, verified locally with the cosign the pipeline installs.** The pinned
`sigstore/cosign-installer@v4.1.2` installs **cosign v3.0.6** (its `cosign-release` default), so the
wiring was checked against that binary, against a throwaway `registry:2` on `localhost:5000`:

- `COSIGN_PASSWORD="" cosign generate-key-pair` -> an ENCRYPTED key (`-----BEGIN ENCRYPTED SIGSTORE
  PRIVATE KEY-----`), no prompt;
- `cosign sign --key env://COSIGN_PRIVATE_KEY <ref>` -> exit 0;
- `cosign attest --key env://COSIGN_PRIVATE_KEY --type cyclonedx --predicate <sbom> <ref>` -> exit 0.

So BLOCKER 1's fix is exercised on the real binary, not only reasoned about.

**A NEW finding, NOT fixed here — the planner's decision, not the developer's.** While verifying the
above, the manifest-signing step was found to FAIL with the cosign the pipeline installs:

- `cosign sign-blob --key "$COSIGN_KEY" --output-signature release.json.sig release.json` (the
  workflow's command) with cosign v3.0.6 prints "Flag --output-signature has been deprecated, please
  use --bundle to provide the output bundle location" and then errors: "signing release.json: create
  bundle file: open : no such file or directory". v3 ignores `--output-signature`, and
  `--new-bundle-format=false` is refused ("must provide --new-bundle-format or --bundle").
- The v3 form works: `cosign sign-blob --key env://COSIGN_PRIVATE_KEY --bundle release.json.bundle
  release.json` writes the bundle, and `cosign verify-blob --key cosign.pub --bundle
  release.json.bundle release.json` answers "Verified OK".
- With cosign **v2.6.1** (downloaded to check the other way) the same command is no better: every
  keyed call asks to upload to the PUBLIC transparency log first ("Are you sure you would like to
  continue? [y/N]") and fails in a runner, which has no terminal, with "user declined the prompt" —
  `cosign sign`, `cosign attest` and `cosign sign-blob` alike. cosign v3.0.6 does NOT ask (that is
  why the two image-signing steps above pass).

So the dry run reaches "Sign the manifest and verify the signature" and fails there, and the real
release would fail at the same step. It is NOT fixed in this round, because the choice is the
plan's: the artifact the plan names (`release.json.sig`, a detached signature that `lakehousectl`
verifies with the committed public key) either becomes a cosign **bundle** (v3's form; the file name
can stay, but `lakehousectl` must verify it with `--bundle`), or the installer is pinned to a v2
release and the transparency-log prompt is answered (`--tlog-upload=false`, and
`--insecure-ignore-tlog` on the verification). The first is the smaller change and the one the
local run proves; the second keeps the detached-signature artifact. This is the reviewer's call,
and the release policy (`docs/core/reference/release-policy.md`) is where the answer belongs.

**Not verified, with the reason:**

- **The dry run is not verified.** It needs a GitHub runner: the four images, the throwaway
  registry, `syft` and the bundle are all CI-side. The PR's own dry run is the proof, and it will
  fail at the manifest-signing step above until that finding is decided.
- **The real release is not verified, and cannot be.** It needs a `v*` tag, the owner's approval on
  the `release` environment, and the release key. REL-2 S5 does that.
- **The secrets' scoping is checked by parsing the workflow, not by running it:** the test proves
  that no `secrets.COSIGN_*` sits in a job-level `env:` and that the three signing steps carry
  them. What a runner does with a step-level `env:` is GitHub's behaviour, not this repository's.
- No cargo command and no Dagster command ran: this round changes no Rust, no code location and no
  Compose file.


### Review S2 round 2 (planner, 2026-11-01, at `fe94c79a`)

- **BLOCKER 1 is fixed** (`f1d72644`). The release job sets `COSIGN_PRIVATE_KEY` from the
  secret. The dry run puts its throwaway key into the same variable and signs with
  `env://COSIGN_PRIVATE_KEY`, so it now proves the real wiring.
- **BLOCKER 2 is fixed** (`8a04449e`).
  - Both secrets are in the `env:` of the three signing steps only.
  - All eight third-party actions are pinned to commit SHAs, with a version comment.
  - `test_release_workflow.py` enforces both rules.
- **The developer's finding is accepted.** The pinned installer brings cosign v3, which
  ignores `--output-signature`, so `sign-blob` fails. The plan decides the format:

**D3a (planner, 2026-11-01): the manifest's signature is a Sigstore bundle.**
- The file is `release.json.sigstore.json`, written by
  `cosign sign-blob --key env://COSIGN_PRIVATE_KEY --bundle release.json.sigstore.json release.json`.
  It replaces `release.json.sig` everywhere: the workflow, the bundle (D11), `SHA256SUMS`, the
  release assets, `docs/CI.md` and this plan's D5 and D11.
- **No transparency-log upload.** Neither run may upload to the public Rekor log:
  - offline verification must not depend on it;
  - a dry run must not publish throwaway entries.

  Use cosign v3's supported way to sign with a key and no transparency log (`--tlog-upload=false`
  if it exists in v3, otherwise a signing config without a tlog), and say in the workflow which
  one it is. If v3 cannot sign a blob with no tlog upload, **stop and report**.
- **Verification:**
  - In CI, right after signing: `cosign verify-blob --key <public key> --bundle
    release.json.sigstore.json --offline` (or the v3 equivalent that skips the tlog).
  - **For S3:** `lakehousectl` reads the bundle's `messageSignature.signature` (base64 DER
    ECDSA P-256 over SHA-256 of `release.json`) and checks it against the built-in key. It
    ignores every tlog field, and it also checks that `messageSignature.messageDigest` equals the
    SHA-256 of the `release.json` beside it.
- **Test:** add a pytest that, given a bundle file produced by the dry-run steps (or a checked-in
  fixture made with a throwaway key), extracts the signature and verifies it with Python's
  `cryptography` against the matching public key. This proves the S3 verification recipe before
  S3 exists.

**Open: D3a to implement.**

### D3a (developer, 2026-11-01)

**Commit** `6bdc7e4d`, branch `feat/rel-2-s2-release-pipeline`, base `6f0d2596`: the manifest's
signature is a Sigstore bundle, and nothing is uploaded to the transparency log.

**What changed, and where:**

- `.github/workflows/release.yml`: `release.json.sig` becomes `release.json.sigstore.json`
  everywhere (the signing step, the bundle step, the dry run's artifact list, the release
  assets, `SHA256SUMS`); the manifest is signed with `cosign sign-blob --bundle` and verified
  with `cosign verify-blob --bundle --offline --insecure-ignore-tlog`; every cosign command
  that signs — the images, the SBOM attestations, the manifest, in BOTH jobs — carries
  `--use-signing-config=false --tlog-upload=false`; the bundle script is given
  `--sigstore-bundle release.json.sigstore.json`.
- `ops/release/make_bundle.sh`: the option `--signature` is `--sigstore-bundle`, the file it
  carries into the bundle is `release.json.sigstore.json`, and `SHA256SUMS` names it.
- `ops/release/test_sigstore_bundle.py` (new) and `ops/release/fixtures/sigstore/` (new): the
  S3 verification recipe, proved now.
- `ops/release/test_release_workflow.py`: the pipeline's new properties are checked on every
  commit (the bundle, the offline verification, the no-tlog flags, the bundle script's
  argument).
- `ops/release/test_make_bundle.py`: the new option and file name.
- `.github/workflows/ci.yml` and both jobs of `release.yml`: `cryptography==50.0.2` is pinned
  where the release tooling's tests are installed — the same pin `dagster/pyproject.toml`
  already uses.
- `docs/CI.md`, `CHANGELOG.md`, this plan's D5 and D11 and its S2-T4 outline: the new file
  name, and the no-Rekor property with its operator consequence.

**The mechanism, and that v3 CAN sign a blob with no tlog upload** (measured with cosign
**v3.0.6**, the version the pinned `sigstore/cosign-installer` provides, a throwaway key with
an empty password, and a CONNECT proxy that logs every host it is asked for):

- The default v3 `cosign sign-blob` posts to `https://rekor.sigstore.dev/api/v1/log/entries`
  and to `https://timestamp.sigstore.dev/api/v1/timestamp` — both seen in the proxy log, the
  Rekor post refused with "Forbidden". So v3's default DOES publish to the public log, which
  is what D3a forbids.
- `--tlog-upload=false` alone is REFUSED by v3: "--tlog-upload=false is not supported with
  --signing-config or --use-signing-config". The flag needs `--use-signing-config=false`
  beside it, and then it works: the same command makes NO connection at all (the proxy log is
  empty), and the bundle it writes carries the signature, the digest and the key's hint — and
  no tlog entry.
- `cosign sign` and `cosign attest` with the two flags: no connection either; and
  `cosign verify --key … --insecure-ignore-tlog` then verifies the signature and the
  attestation against the key.
- `cosign verify-blob --bundle … --offline --insecure-ignore-tlog` inside a network namespace
  with no route at all: "Verified OK" — the verification needs no network.
- The alternative v3 mechanism (a `--signing-config` file with no transparency-log service)
  also works and was tried, but it needs the signing config to name a TSA-free, log-free
  service list; the two flags are shorter and need no extra file in the repository.

**Commands, with their results** (all in the foreground, on the final commit):

| Command | Result |
| --- | --- |
| `pytest ops/release -q` | **74 passed** |
| `python3 -m unittest discover -s ops/release -p 'test_*.py'` | **74 tests, OK** |
| `actionlint .github/workflows/release.yml` | clean, exit 0 |
| `actionlint .github/workflows/ci.yml` | clean, exit 0 |
| `bash -n ops/release/make_bundle.sh` | clean, exit 0 |
| `shellcheck ops/release/make_bundle.sh` (0.10.0) | clean, exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | no violation |

**The bundle, end to end**, over a manifest written by
`ops/release/make_manifest.py` and signed by cosign v3 with a throwaway key:

- `ops/release/make_bundle.sh` wrote the bundle; it holds `docker-compose.yml`,
  `release.json`, `release.json.sigstore.json`, `images/…tar`, `sbom/…cdx.json` and
  `SHA256SUMS`; `sha256sum -c SHA256SUMS` says **OK for all five files**.
- `cosign verify-blob --key <throwaway> --bundle
  release.json.sigstore.json --offline --insecure-ignore-tlog release.json` → "Verified OK",
  on the file as it came out of the bundle.

**The S3 recipe, proved now** (`ops/release/test_sigstore_bundle.py`): the bundle's
`messageSignature.signature` (base64 DER, ECDSA P-256 over the SHA-256 of the manifest)
verifies against the matching public key with `cryptography`; `messageSignature.messageDigest`
is the SHA-256 of the manifest beside it; a tampered manifest is refused; and the bundle
carries no transparency-log entry. The fixture is a bundle made with a throwaway key; its
private half is not in the repository.

**Not verified, with the reason:**

- **The pipeline's own dry run is not verified.** It needs a GitHub runner: the four images,
  the throwaway registry, cosign, `syft` and the bundle are all CI-side. The PR's dry run is
  the proof, and its summary is where the bundle's size appears.
- **The real release is not verified, and cannot be.** It needs a `v*` tag, the owner's
  approval on the `release` environment, and the release key. REL-2 S5 does that.
- **The release key's own signature is not verified.** The commands were proved with a
  throwaway key; the release key is the same kind of key (ECDSA P-256) in the same
  `env://COSIGN_PRIVATE_KEY` form the real job uses, but the real key never left the
  `release` environment.
- No cargo command and no Dagster command ran: this round changes no Rust, no code location
  and no Compose file.

### Review S2 round 3 (planner, 2026-11-01, at `72240311`)

- **D3a is implemented.**
  - `release.json.sigstore.json` replaces `.sig` everywhere.
  - Every signing command (sign, attest, sign-blob, in both jobs; 11 flag sites) carries
    `--use-signing-config=false --tlog-upload=false`.
  - Offline `verify-blob` follows each signing.
- **The evidence is strong.** Through a logging proxy, v3's default posted to the public Rekor
  log and to the timestamp authority. With the flags, the command opened no connection at all.
  `verify-blob --offline --insecure-ignore-tlog` passed in a network namespace with no route.
- **The S3 recipe is proven** by `test_sigstore_bundle.py`:
  - the signature verifies with `cryptography`;
  - the digest matches the manifest;
  - a tampered manifest is refused;
  - the bundle holds no tlog entry.

  The fixture's private key is not in the repository.
- **Checks:** 74 tests pass, and `actionlint` and `shellcheck` are clean.

**Open: no BLOCKER, no SHOULD-FIX.**
- The PR's own CI dry run is the last proof: it builds the four images, signs them with a
  throwaway key, and builds the bundle.
- The real release is S5.

### Review S2 round 4 (planner, 2026-11-01): the PR's dry run, at `dc0b095`

The dry run built and pushed all four images and signed them through `env://COSIGN_PRIVATE_KEY`.
It then failed at the first SBOM step.

**BLOCKER 1 (round 4): the SBOM folder is never created.**
- `anchore/sbom-action` scanned the image and then failed with
  `ENOENT: no such file or directory, open 'sbom/rantai-lakehouse-api.cdx.json'`. Nothing creates
  `sbom/` first.
- The real `release` job has the same step, so it would fail the same way.
- **Fix:** add a `mkdir -p sbom` step before the first SBOM step, in **both** jobs.

**SHOULD-FIX 1 (round 4): the dry run's key is in the environment of every later step.**
- The dry run writes its throwaway key to `$GITHUB_ENV`, so the runner prints the
  (encrypted, empty-password) PEM in every later step's environment block. The SBOM step's log
  shows it.
- It is not a secret, because it is generated per run, but the dry run is meant to copy the real
  job's shape: the key visible to the signing steps only.
- **Fix:** keep the PEM in a step output, or a file under `$RUNNER_TEMP`, and set
  `COSIGN_PRIVATE_KEY` in the `env:` of the three signing steps only, as the real job does.
- Extend `test_release_workflow.py`: `COSIGN_PRIVATE_KEY` must not be written to `$GITHUB_ENV`.

**Open: BLOCKER 1 and SHOULD-FIX 1 (round 4).**

### Fixes for review round 4 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s2-release-pipeline`, base `7be6cc90`), one per finding:

| Commit | Finding | What |
| --- | --- | --- |
| `64120d52` | REL-2 S2 review round 4 BLOCKER 1 | the SBOM folder is created before the SBOM action writes into it |
| `23a28f7e` | REL-2 S2 review round 4 SHOULD-FIX 1 | the dry run's throwaway key reaches the three signing steps only |

**BLOCKER 1 (`64120d52`).** Both jobs create `sbom/` in a step of their own, before the first
SBOM step: `anchore/sbom-action` writes into the folder and does not create it, so the first SBOM
step failed with ENOENT. `ops/release/make_bundle.sh` wants the folder too — it refuses to run
when `--sbom-dir` is not a directory — so without this step the bundle step would have failed as
well. `ops/release/test_release_workflow.py` holds both jobs to it, and a synthetic workflow that
does not create the folder is reported.

**SHOULD-FIX 1 (`23a28f7e`).** The dry run's throwaway key travels as the OUTPUT of the step that
generates it, and the three signing steps take it from there into their own `env:` — the same shape
as the real job, which takes the release key from the `release` environment's secrets into the
`env:` of the same three steps. Nothing writes the key to `$GITHUB_ENV` any more, so no later step
of the dry run can read it, and the SBOM step's environment block is clean. The signing path is
unchanged: `env://COSIGN_PRIVATE_KEY` resolves in both jobs, so the dry run still proves the real
wiring (REL-2 S2 review BLOCKER 1).

Two details, both stated at the site in the workflow:

- The PEM is masked in the log, line by line (`::add-mask::`), so the dry run's log looks like the
  real job's, where the release key is `***`: a private key should not be readable in a log.
- The key file leaves the workspace with the step that generated it (`rm -f cosign.key`): the PEM
  travels in the output now, and nowhere else.

The empty `COSIGN_PASSWORD` stays in the dry run's job environment: it is empty, so there is nothing
to hide — the KEY is what is scoped. `docs/CI.md`'s dry-run paragraph says the same.

**The test extension** (`ops/release/test_release_workflow.py`, 4 new cases): no step writes
`COSIGN_PRIVATE_KEY` to `$GITHUB_ENV` (a synthetic workflow that does is reported), the key is in
the `env:` of the dry run's three signing steps, the dry run's job environment holds no key, and
the step that generates the key publishes it as an output (the old case, which required
`$GITHUB_ENV`, is replaced by this one). The checks read the `run:` blocks without their comment
lines: a comment is not a command, and the workflow explains itself in them.

**Commands run, in the foreground, on `23a28f7e`:**

| Command | Result |
| --- | --- |
| `python3 -m pytest ops/release -q` | **80 passed, 0 failed** (74 before this round, 6 new: 2 for BLOCKER 1, 4 for SHOULD-FIX 1) |
| `actionlint .github/workflows/release.yml` | clean, exit 0 (actionlint v1.7.7, with shellcheck 0.10.0 on the PATH, so the `run:` blocks were checked too) |

The pytest run used a virtualenv's interpreter (`/tmp/opencode/relvenv/bin/python -m pytest
ops/release -q`): the system Python is externally managed (PEP 668), so `pip install --user pytest`
is refused, and the virtualenv carries `pytest==8.3.4` over the system site packages (PyYAML,
cryptography). `actionlint` and `shellcheck` were not installed on this machine; both were
downloaded to `/tmp/opencode` (actionlint v1.7.7, shellcheck v0.10.0) to run the checks above.

The same two commands were run again, in the foreground, on the final commit `b0abeb27` (this
section and `docs/CI.md` only): **80 passed, 0 failed**, and `actionlint` clean, exit 0. The
repository's own Python guards ran too: `python3 ops/lint/check_intra_package_imports.py` → OK,
`python3 ops/lint/check_bare_iceberg_count.py` → no violation.

**Not verified, with the reason:**

- **The dry run is not verified.** It needs a GitHub runner, and the PR's own dry run is the proof:
  both findings are runner behaviour (the SBOM action's write, the runner's environment block).
- **The masking of the PEM is not verified.** `::add-mask::` is the runner's own command; it is
  checked here only by the shape of the step (one command per line of the PEM).
- The real release is not verified, and cannot be. It needs a `v*` tag, the owner's approval on the
  `release` environment, and the release key. REL-2 S5 does that.
- No cargo command and no bun command ran: this round changes no Rust, no TypeScript, no code
  location and no Compose file.

### Review S2 round 5 (planner, 2026-11-01): the PR's second dry run, at `0cef322`

The SBOM step now works, and the key is masked (`***`) and visible to the signing steps only. The
run then failed at "Attest each SBOM to its image": `cosign attest --type cyclonedx` answered
`invalid character '<' looking for beginning of value`.

**BLOCKER 1 (round 5): the SBOM is XML.**
- `anchore/sbom-action` was given the format `cyclonedx`, which syft writes as CycloneDX **XML**.
  `cosign attest --type cyclonedx` reads a **JSON** predicate.
- **Fix:** use `format: cyclonedx-json` in every SBOM step of both jobs. The `.cdx.json` file names
  are then true.
- Add a check to `test_release_workflow.py`: every sbom-action step uses `cyclonedx-json`.

**Noted, not a finding:** cosign v3 prints `--tlog-upload has been deprecated, prefer using a
--signing-config file with no transparency log services`. The flag still works, and the
proxy-logged proof stands. Replacing it with a `--signing-config` file is left for when the flag
is removed; add a line to `docs/CI.md` saying so.

**Open: BLOCKER 1 (round 5).**
