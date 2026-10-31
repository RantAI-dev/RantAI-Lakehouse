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

## 2. Decisions (made by the planner; the owner's four are marked OWNER)

- **D1 (OWNER #1).** The tool is `lakehousectl` (proposed).
- **D2 (OWNER #2).** Releases live on GitHub Releases. `install.sh` is served from the release
  and from a short rantai.dev address (proposed).
- **D3 (OWNER #3).** A cosign ECDSA P-256 key pair (proposed).
  - The private key is a GitHub Actions secret in a `release` environment with a required
    reviewer, available to tag builds only.
  - The public key is committed at `ops/release/release.pub` and compiled into `lakehousectl`
    with `include_bytes!`.
  - CI dry runs use a throwaway key generated in the job. They never touch the real key.
- **D4 (OWNER #4).** SemVer as in `reference/release-policy.md`. The first release through the
  pipeline is 0.2.0 (proposed).
- **D5. Install layout.**
  - Everything lives in one directory, `/opt/rantai-lakehouse` by default (`--dir` to change it):
    - `docker-compose.yml`, the release's file with digests pinned;
    - `.env`, mode 0600;
    - `release.json` and `release.json.sig`;
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
  - `release.json` and `release.json.sig`;
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
