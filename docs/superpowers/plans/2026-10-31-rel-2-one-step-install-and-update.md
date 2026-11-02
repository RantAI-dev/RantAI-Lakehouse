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

## 4b. S3 plan check (planner, 2026-11-01, at `main` `2c110ebc`): `lakehousectl install`

Read today:
- **`release.json`** (`ops/release/make_manifest.py`) holds `schemaVersion`, `version`, `gitSha`,
  `createdAt`, `minFrom`, `stops`, `images` (service → `ref@sha256:<manifest digest>`, every
  service of the rendered Compose file) and `files`. `files` covers only `docker-compose.yml`
  today.
- **Signature:** `release.json.sigstore.json`, a Sigstore bundle with no tlog. The recipe that
  verifies it is proven in `ops/release/test_sigstore_bundle.py`: `messageSignature.signature` is
  an ECDSA P-256 DER signature over SHA-256 of `release.json`, and `messageDigest` is that
  SHA-256. The fixture is `ops/release/fixtures/sigstore/` (bundle, manifest, throwaway public
  key).
- **The bundle** (`make_bundle.sh`) holds `docker-compose.yml`, `release.json`,
  `release.json.sigstore.json`, `images/<name>.tar` (`docker save` by reference), `sbom/` and
  `SHA256SUMS`, as a `.tar.zst`.
- **Secrets:** `ops/init-env.sh:64-90` lists what a fresh `.env` needs, and `.env.example` is the
  list of keys.
- **The release key:** `ops/release/release.pub`.
- **Workspace:** `rust/Cargo.toml` members `crates/*`.

**The gap S3 must close first (T0).** `docker save` of a pulled image drops its registry manifest
digest. After `docker load`, the host knows only the image **ID**, which is the config digest. So
an air-gapped install cannot compare a loaded image against `images` (manifest digests). The
manifest must also carry each image's ID.

Branch `feat/rel-2-s3-lakehousectl-install` off `main`. One commit per task.

- **S3-T0. The manifest carries image IDs.**
  - `make_manifest.py` gains `imageIds`: unique image reference → `sha256:<config digest>`. The
    pipeline fills it after pulling or building each image (`docker image inspect --format
    '{{.Id}}' <ref>`), in both jobs of `release.yml`.
  - `files` gains `SHA256SUMS`'s own entries? No: keep `files` as it is. The bundle's
    `SHA256SUMS` is checked against the tar contents, and the signed manifest is the root of
    trust.
  - Tests in `test_make_manifest.py`.
  - *Accept:* `pytest ops/release -q`, and `actionlint`.
- **S3-T1. The crate.** `rust/crates/lakehousectl`, a binary, built with `clap` (derive).
  - `verify` module: parse the Sigstore bundle, then check
    `messageDigest == sha256(release.json)` and the ECDSA P-256 signature against the key built
    in with `include_bytes!("../../../../ops/release/release.pub")`. Use the `p256` and `ecdsa`
    crates (pin minors, and pass `cargo deny`).
  - Test against the S2 fixture (throwaway key, injected in tests) and against a tampered
    manifest.
  - *Accept:* `cargo fmt --check`; `cargo clippy -p lakehousectl --all-targets -- -D warnings`;
    `cargo test -p lakehousectl`; `cargo deny check`.
- **S3-T2. Pre-flight** (spec minimums). Each check is its own function, testable with injected
  facts:
  - Linux on x86_64;
  - Docker 24 or newer, and Compose 2.24 or newer (from `docker version` and
    `docker compose version`);
  - 4 or more CPUs, and 16 GiB or more of memory (`/proc/meminfo`);
  - 50 GiB or more free under `--dir`;
  - the published ports free: `CONSOLE_HOST_PORT` (3000), `API_HOST_PORT` (8080) and the
    others the rendered Compose file publishes.

  `--ignore-preflight <name>` skips one check and logs it. Every failure says what to fix.
- **S3-T3. `lakehousectl install [--bundle <dir>] [--dir /opt/rantai-lakehouse]`** (D5, D6):
  - **verify** the manifest;
  - **online:** read the release assets from GitHub Releases (latest, or `--version`), then
    `docker pull` each `images` reference by digest;
  - **offline:** check `SHA256SUMS`, run `docker load` on each `images/*.tar`, and check every
    loaded image ID against `imageIds`, refusing any mismatch;
  - **write the layout** of D5, with `.env` mode 0600;
  - **generate secrets** following D6, only when missing; port `init-env.sh`'s list into Rust,
    and test that a second run changes nothing;
  - **start the stack** with `docker compose -p rantai-lakehouse up -d`, then wait for
    `/health/ready` (`ready: true`, version = the manifest's) and for the console, both with
    bounded waits;
  - **print** the console address and the first administrator's email and password (generated,
    shown once, must be changed at first sign-in).
- **S3-T4. `install.sh`** (`ops/release/install.sh`):
  - detect the architecture (x86_64 only, with a clear refusal otherwise);
  - download `lakehousectl-linux-x86_64` and `SHA256SUMS` from the release, check the checksum,
    and run `lakehousectl install "$@"`;
  - use `set -euo pipefail` and pass shellcheck.

  The release pipeline builds `lakehousectl` (static musl) and adds it to the bundle, the
  release assets, `files` in the manifest and `install.sh`.
- **S3-T5. The CI proof.** The dry run gains a job that, on a fresh runner:
  1. downloads the dry run's bundle artifact;
  2. runs `lakehousectl install --bundle` with the throwaway key, which a test-only build flag or
     environment variable may point to, **never in a release build**;
  3. asserts that `/health/ready` reports ready and the console answers.

**S3 verification:** the scoped cargo commands, `pytest ops/release`, `shellcheck`, `actionlint`.
The PR's dry run is the end-to-end proof.

## 4c. S4 plan check (planner, 2026-11-01, at `main` `5661a043`): update, backup, rollback

Read today:
- **The CLI** (`rust/crates/lakehousectl/src/cli.rs`) has `verify`, `preflight` and `install`.
  `install` writes the D5 layout, including `state.json` with `currentVersion`,
  `previousVersion: null` and `lastBackup: null` (`install.rs`, `write_layout`).
- **Backup and restore exist as repo scripts**: `ops/backups/backup-local.sh` (D8: Postgres
  `pg_dumpall`, ClickHouse `BACKUP … TO Disk('backups')`, the connector-secrets volume, `.env`,
  and a `manifest.json` of checksums) and `scripts/restore-from-backup.sh --local` (checks the
  checksums first).
  - Both `cd` to the repository root and run `docker compose -p <project>` there
    (`backup-local.sh:60-75`).
  - An installed host has neither script.
- **Gap found by this check: the release's Compose file bind-mounts files the install does not
  write.**
  - `clickhouse` mounts `./ops/backups/clickhouse/backups.xml` (`docker-compose.yml:143`), which
    is the backup disk `Disk('backups')` needs.
  - `trino-maintenance-cron` (profile `trino`) mounts `./ops/trino/optimize_bronze.sh` (`:3266`).
  - On an installed host Docker creates an empty directory in their place. ClickHouse still
    starts, which is why the install proof passed, but a backup cannot run.
  - The other relative mounts are test runners in test profiles, which a release does not run.
- **`MAINTENANCE_PAUSED` (D9) is not implemented anywhere.** `git grep` finds it only in this
  plan.
- **The local backup does not hold the warehouse files** (`RustFS`). D8 never said it did. A
  restore therefore returns the catalogs, Postgres with Lakekeeper's tables and ClickHouse, to
  the moment of the backup, while data files written after it stay as orphans. That is safe
  only while no expiry or orphan-file job deletes files that the restored catalog still names.
  This is why D9 pauses them.
- **No earlier release exists to update from.** 0.2.0 is the first (D4). So the S4 proof cannot
  install "the previous release". It builds a second candidate in the job instead (D21).

**Decisions (planner):**
- **D14. The release ships every file its Compose file mounts.**
  - The pipeline adds each relative bind-mount source used by a service that a release runs
    (no test-runner profile) to the bundle and to the manifest's `files`, with its SHA-256.
    Today that is `ops/backups/clickhouse/backups.xml` and `ops/trino/optimize_bronze.sh`.
  - `install` and `update` write them into `--dir` at the same relative path, after checking
    each against the signed manifest.
  - A release-tooling test fails when the rendered Compose file mounts a relative source that
    `files` does not list.
- **D15. One backup and one restore, shipped as tools.**
  - `backup-local.sh` and `restore-from-backup.sh` gain `LAKEHOUSE_DIR`: the directory holding
    `docker-compose.yml` and `.env`, defaulting to the repository root, so a developer's
    behaviour is unchanged. With it they run `docker compose -p <project> -f
    "$LAKEHOUSE_DIR/docker-compose.yml" --env-file "$LAKEHOUSE_DIR/.env"`, plus `override.yml`
    when it exists.
  - The two scripts ship as release files (D14 mechanism) under `tools/`.
  - `lakehousectl backup` and `lakehousectl restore <dir>` run them with `LAKEHOUSE_DIR=--dir`
    and `COMPOSE_PROJECT=rantai-lakehouse`. There is no Rust port, so the steps have one
    source.
  - The backup set also gets `release.json` and `state.json`, which now exist. That closes the
    gap `backup-local.sh`'s header admits.
  - `lakehousectl backup` writes `state.json`'s `lastBackup`.
- **D16. `MAINTENANCE_PAUSED`.**
  - The Dagster services get `MAINTENANCE_PAUSED: ${MAINTENANCE_PAUSED:-}`.
  - The snapshot-expiry and orphan-file jobs read it with `== "true"`. When it is set, they skip
    and record the reason in `skipped_verbs`; they do not fail.
  - `lakehousectl` sets it in `.env` before the backup of an update, and clears it after a
    success or after a rollback. The services that read it are recreated by `up -d`.
- **D17. `lakehousectl update [--bundle <dir>] [--version <v>] [--ready-timeout <secs>]`.**
  The steps run in this order, and nothing running is touched before step 4:
  1. Obtain and verify the new release, as `install` does.
  2. Compute the path (D10) from `state.json`'s `currentVersion`:
     - the same version: nothing to do;
     - older: refused, pointing to `rollback`;
     - below `minFrom`, or with a `stops` entry between: online, each hop runs in turn; offline,
       it is refused, naming the bundles of the stops.
  3. Run the pre-flight checks (including `image-store` for a bundle), write the D14 files to a
     staging directory, and pull or load every image of the new release. A failure here leaves
     the running stack as it was.
  4. Set `MAINTENANCE_PAUSED=true`. Stop the writers: `lakehouse-api`, `console`, and
     `dagster-daemon`, `dagster-code-location` and `dagster-webserver` when they run. Postgres,
     ClickHouse, RustFS and Lakekeeper keep running.
  5. Take a backup (D15) to `backups/<stamp>-<old version>/`, and record it in `state.json`.
  6. Keep the old `docker-compose.yml` and `release.json` as `*.previous`. Put the new ones and
     the D14 files in place, then `up -d --pull never`.
  7. Wait up to `--ready-timeout` (default 600 s, D9) for `/health/ready` to report the new
     version as ready, and for the console.
  8. On success: clear the pause, and set `state.json`'s `currentVersion` to the new version and
     `previousVersion` to the old one.
  9. On a timeout or a failure from step 6 on, roll back by itself (D18), and exit non-zero.
     It reports both what failed and what the rollback did.
- **D18. `lakehousectl rollback [--yes]`**, and the automatic rollback:
  - stop the stack;
  - `restore` the update's backup (`state.json`'s `lastBackup`);
  - put the `*.previous` files back, and `up -d --pull never`;
  - wait for ready on the old version;
  - clear the pause, and set `state.json` back.
  - By hand it first says that data written since the update is lost, and it needs `--yes` or an
    interactive "yes".
  - It refuses when there is no previous release or no backup.
- **D19. `lakehousectl status`.** It shows:
  - the installed version (`state.json`);
  - each service's state (`docker compose ps --format json`);
  - the API's `/health/ready`;
  - the backups (newest first, with size);
  - online, whether a newer release exists. Offline it says that it did not check.
- **D20. `lakehousectl self-update`** (online).
  1. Read the newest release's `release.json` and its signature, and verify them with the
     compiled-in key.
  2. Download `lakehousectl-linux-x86_64`, and check it against that manifest's
     `files["lakehousectl"]`.
  3. Replace the running binary atomically (a write to a temporary file, then a rename), and
     say which version it now is.
  - Offline, the bundle's own `lakehousectl` is the new tool, and the help text says so.
- **D21. The proof, `update-proof`, a new job in the dry run, after `install-proof`.** On a fresh
  runner, air-gapped as `install-proof` is:
  1. Install the dry run's bundle A, version `V`.
  2. Seed markers: a row in a marker table in the application database, a row in a ClickHouse
     marker table, and a stored connector credential through the API.
  3. Build bundle B: the API image rebuilt with its version set to `V-update-proof.1`, every
     other image reused from A, and the manifest signed with the job's throwaway key.
     `lakehousectl update --bundle B` must report B ready, and every marker must still be
     there.
  4. Build bundle C, version `V-update-proof.2`, whose API image never becomes ready (it sleeps).
     `lakehousectl update --bundle C --ready-timeout 180` must exit non-zero and report the
     rollback. Afterwards `/health/ready` must say B and ready, and every marker must still be
     there.
  - The real "previous release" test, installing 0.2.0 and then updating, joins this job once
    0.2.0 exists (S5), with the reason written in the job.

**PR slicing:**
- **S4a** (one PR): T1–T3 (D14, D15, D16) and `backup` and `restore`.
- **S4b** (one PR, after S4a merges): T4–T7 (D17–D21).

Branch `feat/rel-2-s4-lakehousectl-update` off `main`, one commit per task.

- **S4-T1 (D14). The release ships its mounted files.**
  - Change `render_compose.py` or `make_manifest.py` (whichever owns `files`), `make_bundle.sh`
    and `release.yml`'s assets, and the new release-tooling test.
  - `install` writes and checks the files, with a Rust test.
  - *Accept:* `pytest ops/release -q`, `actionlint`, `cargo test -p lakehousectl`.
- **S4-T2 (D15). The scripts take `LAKEHOUSE_DIR`, ship as tools, and gain `release.json` and
  `state.json` in the set.** Add `lakehousectl backup` and `restore <dir>`, with tests that a
  backup and a restore call the shipped scripts with the right environment, using fakes.
  - *Accept:* `shellcheck` on both scripts, `pytest ops/release -q`, `cargo test -p
    lakehousectl`.
  - Then one real run on bluefin: install a small bundle, `lakehousectl backup`, destroy the
    volumes, `lakehousectl restore`, and the data is back. Quote it.
- **S4-T3 (D16). `MAINTENANCE_PAUSED`.** Change the compose environment of the Dagster services,
  and the two jobs' skip.
  - Python tests with full-sentence names, and no network.
  - *Accept:* `(cd dagster && python -m pytest dispar_orchestrate -q -k "maintenance or
    expiry or orphan")`, `docker compose --profile '*' config --quiet`.
- **S4-T4 (D17, D18).** `update` and `rollback`, with every step a function that takes
  injected facts and a fake docker runner, as `preflight` is.
  - Unit tests:
    - the path rules;
    - the order of the steps;
    - a failure at step 3 touches nothing;
    - a failure at step 7 rolls back;
    - `rollback` refuses when there is no backup.
  - *Accept:* `cargo test -p lakehousectl`, clippy.
- **S4-T5 (D19). `status`**, with tests.
- **S4-T6 (D20). `self-update`**, with tests using a fake download, including a refusal on a
  wrong checksum.
- **S4-T7 (D21). `update-proof`**, with workflow tests in `test_release_workflow.py`. The PR's dry
  run is its proof.

**Out of scope for S4:** a `lakehousectl` backup to S3 (the `backups` profile remains the S3
path, and `status` names it), a backup of the warehouse files, and zero-downtime updates.

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

### Handoff S3 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s3-lakehousectl-install`, base `main` at `2c110ebc`), one per
task, in plan order:

| Commit | Task | What |
| --- | --- | --- |
| `87f2c13b` | S3-T0 | `release.json` carries each image's id |
| `4ccdc14b` | S3-T1 | the crate, and the verification of a release |
| `b5c3932b` | S3-T2 | the pre-flight checks |
| `6c2efe98` | S3-T3 | `lakehousectl install` |
| `6d7ed009` | S3-T4 | the installer in the release, and `install.sh` |
| `aa6b5e7a` | S3-T5 | the CI proof that the bundle installs |
| `3fe050ab` | (fix) | the image id is the config digest on every Docker store |

**Commands, with their results.** Every cargo command went through `bin/cargo-locked`, in the
foreground, on the shared `CARGO_TARGET_DIR`; no `cargo clean`; no workspace build.

The crate:

- `cargo fmt --check` — clean (the whole workspace, and `-p lakehousectl`).
- `cargo clippy -p lakehousectl --all-targets -- -D warnings` — clean, no warning and no error.
- `cargo test -p lakehousectl` — **43 passed, 0 failed**.
- `cargo deny check all` — **advisories ok, bans ok, licenses ok, sources ok**.

The release tooling:

- `pytest ops/release -q` — **130 passed, 0 failed** (107 before this slice, 23 new).
- `actionlint .github/workflows/release.yml` — clean, exit 0 (actionlint v1.7.7, with
  shellcheck 0.10.0 on the PATH, so the `run:` blocks were checked too).
- `bash -n` and `shellcheck` (0.10.0) on `ops/release/make_bundle.sh`,
  `ops/release/image_name.sh` and `ops/release/install.sh` — clean, exit 0.
- `python3 ops/lint/check_intra_package_imports.py` — OK.
- `python3 ops/lint/check_bare_iceberg_count.py` — no violation.

**The tool, run.** The binary was built and run here (the native build; the static musl build
is CI-side, see below):

- `lakehousectl verify --dir ops/release/fixtures/sigstore`, with the `test-key` build and
  `LAKEHOUSECTL_PUBLIC_KEY_FILE=ops/release/fixtures/sigstore/throwaway.pub` → "the manifest
  in ops/release/fixtures/sigstore verifies against the release key", exit 0.
- The same binary on a **tampered** manifest → "the signature is over a different file: the
  bundle names the digest 1d4e1a9e…, the manifest beside it hashes to 555308cb…", exit 1.
- A **release** build (no `test-key`) with the same variable set → "LAKEHOUSECTL_PUBLIC_KEY_FILE
  is set and ignored: this build verifies against the release key compiled into it." and "the
  release key did not sign this manifest", exit 1. That is the "a release build never accepts
  another key" property, proved on the binary.
- `lakehousectl install --bundle <a bundle whose manifest was tampered with>` → refused with
  the digest mismatch, and **nothing was written**: the install directory did not exist
  afterwards. The manifest is verified before anything is written.
- `lakehousectl preflight --compose docker-compose.yml --dir /tmp` → "memory: this host has
  13 GiB of memory, and the release needs 16 GiB or more" and "disk: there are 5 GiB free
  under the install directory, and the release needs 50 GiB or more" — **this machine is below
  the spec's minimums**, so both checks fired. The ports check read the repository's own
  Compose file and found the console's 3000, the API's 8080 and Postgres' 5432 (a unit test
  holds that against the real file).

**A finding, and the fix (`3fe050ab`).** The plan's recipe for the image id —
`docker image inspect --format '{{.Id}}'` — is the **config digest** on Docker's classic
image store and the **image index digest** on the containerd store. Measured here (Docker
29.9.2, the containerd store): the image's `.Id` is `sha256:d9e853e8…` (the index digest)
while its config digest is `sha256:bf8527eb…` — the blob the saved tar names as `Config`.
A manifest built on one store and loaded on the other would compare two different identities,
and the install would refuse a good bundle. So:

- the pipeline names the **config digest** in `imageIds` (the identity a LOADED image carries
  on both stores): when the store reports the index digest — it equals the reference's
  registry digest — the config digest is read from the image's own saved tar;
- the installer accepts the loaded image's ID when it is **either** the config digest
  (`imageIds`) **or** the registry digest (`images`). Both are covered by the release key, and
  they are the two identities Docker's stores give the same image.

**Not verified, with the reason:**

- **The dry run and the proof job are not verified.** They need a GitHub runner: the four
  images, the throwaway registry, cosign, syft, a ~3 GiB bundle, and the whole stack started
  on it. The PR's own CI is the proof, and its summary is where the bundle's size appears.
- **The static musl build is not verified here.** The `x86_64-unknown-linux-musl` target and
  `musl-tools` are not installed on this machine, and the pipeline installs both. The binary
  was built and run natively — the same source, the same features, the same dependencies.
- **The whole install was not run end to end here.** It needs a manifest signed with the
  release key, and that key is in the `release` environment's secrets. The proof job does it
  with the dry run's throwaway key, which is the only key this slice may touch.
- **The full workspace run** (`cargo clippy --workspace --all-targets --all-features` and
  `cargo test --workspace`): not run here — this slice's machine rules say CI runs them; the
  crate was clipped with `--all-targets` and tested, and `cargo deny check all` covered the
  whole dependency graph.
- **No bun command and no Dagster command ran**: this slice changes no TypeScript and no code
  location. `docker compose --profile '*' config --quiet` was not run either: this slice
  changes no Compose file (the release's Compose file is RENDERED by the pipeline from the
  unchanged template).
- The `install.sh` script was tested against a fake `curl` and a fake `uname`, not against
  GitHub: the script's own logic (the refusal, the checksum check, the hand-over) is what
  those tests cover.

**Where the plan left room, and what was decided** (for the reviewer):

- **The command surface.** The plan gives the crate a `verify` module and the pre-flight
  checks, but no command for them. They became `lakehousectl verify` and
  `lakehousectl preflight` — the modules' own CLI surface, so each module is reachable and
  testable on its own, and the install runs the same checks.
- **The pre-flight's ports** come from the release's own Compose file, not from a list in the
  tool: the release publishes more than the console and the API, and the operator's `.env`
  may move any of them. The `.env` is read for the values it sets.
- **The install's HTTP client is written here** (a plain GET on the loopback), so the binary
  carries no TLS stack: the offline path needs no network at all, and the online path
  downloads the release's assets with `curl`, the tool `install.sh` already uses. A static
  musl binary with a TLS stack would need a C toolchain in the pipeline.
- **The secrets are the Rust port of `ops/init-env.sh`** (D6): the repository's
  `.env.example` is the list of keys, COMPILED INTO the binary (the installer has to work on
  a host that has never seen this repository), the values are generated only when they are
  missing, an existing `.env` is never overwritten, and a second run changes nothing. The
  first administrator's password is printed once — on a re-install the tool says where the
  password is instead of printing it again.
- **`state.json`** (D5) carries `schemaVersion`, `currentVersion`, `previousVersion` and
  `lastBackup`. `previousVersion` and `lastBackup` are null until REL-2 S4 fills them.
- **The bundle carries the installer** as `lakehousectl`, and the signed manifest names its
  checksum in `files` — so the bundle's copy is the file the release published, checked
  against the signature and not only against the bundle's own `SHA256SUMS`.
- **`install.sh` is a bash script** (`set -euo pipefail`, as the plan asks), so the documented
  command is `curl -fsSL https://get.rantai.dev/lakehouse | bash`. The spec's example says
  `| sh`, which dash cannot run (`pipefail` is bash). The reviewer may want the spec's
  wording changed, or the script rewritten in POSIX sh.
- **The CI proof skips two pre-flight checks** (`disk` and `memory`): a GitHub runner has
  neither the 50 GiB of free disk nor the 16 GiB of memory the release asks for. The skips are
  said out loud by the tool, and the minimums themselves are proved by the operator's
  acceptance checklist (REL-2-AC3), not by a runner.

**Known gaps in this slice** (not fixed here; the reviewer decides):

- **The bundle's images are verified against the manifest by ID, and the ID is checked
  against the tar's own content only through that.** The bundle's `SHA256SUMS` covers the
  image tars, and the signed manifest names the images' digests and ids — but the tar's
  config blob is not hashed against the manifest's `imageIds` by the installer itself. An
  air-gapped install would be stronger if the installer read the config digest out of each
  tar (the tar's `manifest.json` names it) and compared it with the signed manifest, which
  needs a tar reader in the tool. The plan's own check (the loaded image's ID) is what is
  implemented.
- **The offline path trusts the bundle's `SHA256SUMS`, which is not signed.** The signed
  manifest names the images' digests and ids, so the two chains meet at the image's ID; a
  bundle whose tars were rewritten would need the manifest's ids to match too.
- **The online path downloads the release's assets with `curl`**, so the online install needs
  `curl` on the host. The pre-flight does not check for it (the plan's list does not name
  it); the offline path needs nothing but Docker.
- **The install does not roll back.** REL-2 S4 adds `update` with the automatic rollback
  (D9); this slice only installs.
- **The proof job starts the whole stack on a runner** (35 services in the rendered Compose
  file, default profile). Whether a GitHub runner has the memory and the disk for it is a
  runner fact, and the PR's own CI answers it.

### Fixes for review S3 round 1 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s3-lakehousectl-install`, base `b0855ae8`), one per finding:

| Commit | Finding | What |
| --- | --- | --- |
| `a5f49eca` | REL-2 S3 review round 1 BLOCKER 1 | the online install reads the release into memory |
| `f34f19e2` | REL-2 S3 review round 1 SHOULD-FIX 2 | the bundle's image tars are hashed, not just named |
| `5757949d` | REL-2 S3 review round 1 SHOULD-FIX 3 | a second install does not overwrite the first one's state |
| `bf0f7c43` | REL-2 S3 review round 1 SHOULD-FIX 4 | the administrator's email is the one the `.env` holds |
| `38ccc6f4` | REL-2 S3 review round 1 SHOULD-FIX 5 | the installer script leaves nothing behind |

**BLOCKER 1 (`a5f49eca`).** The online install wrote the release's three assets into
`temp_dir()/lakehousectl-release-<version>`, with `create_dir_all` and `curl --output`. The
installer runs as root, and that name is a name anybody can guess: a local user who creates the
directory first, with a symlink at `release.json`, has root write the file it points at, and a
directory an earlier run left behind was reused as it was.

The assets are small text files, so they are read into memory and nothing is written at all:

- `sys::output_bytes` runs a command and returns its standard output byte for byte — not trimmed,
  because the manifest's signature is over its exact bytes (`sys::output` is now that plus the
  trimming it always did, and its doc says what it always did);
- `install::download` reads one asset with `curl` to its standard output, and `install::asset_url`
  names the three URLs;
- the temporary directory is not touched any more: there is no path of the tool's own for another
  local user to own.

The download is injected (`obtain_with`), so a test drives the online path with no network: it
holds the three assets that are asked for, and that the temporary directory holds nothing of the
tool's own before and after. A second test holds `sys::output_bytes` to the exact bytes (a
trailing newline is part of the answer).

**SHOULD-FIX 2 (`f34f19e2`).** `checksums_cover()` asked whether a line of the bundle's
`SHA256SUMS` named the tar, and never hashed it: a copy damaged on its way to the host was loaded
as it was, and `docker load` was the first thing that noticed. Each `images/<name>.tar` is now
hashed with SHA-256 and compared with its own line BEFORE the load:

- `checksum_of` reads the digest the list names for a file (the paths are relative to the bundle's
  root, as `sha256sum` writes them);
- `sha256_of` hashes the file in pieces — an image tar is hundreds of megabytes, and the installer
  has no reason to hold one in memory;
- `verify_checksum` refuses a file the list does not name, and a file whose digest differs, with
  the message that says what does not match.

The bundle's `SHA256SUMS` is not signed, and the code says so: this check catches damage, and the
TRUST check stays the image's ID against the SIGNED manifest's `imageIds`. Three tests: a file
that matches its checksum is accepted, a damaged one is refused with the file named, and a file
the list does not name is refused.

**SHOULD-FIX 3 (`5757949d`).** `write_layout()` always rewrote `state.json` — `previousVersion:
null`, `lastBackup: null` — so a second install over the first one's directory threw away the
state of the install that was there, for ANY release and with nothing said. The install now reads
the state before it writes anything:

- the SAME release may be installed again, and `state.json` is kept as it is: the previous version
  and the last backup of the first install survive a re-run;
- any OTHER release is refused, and the refusal names the release that is there and points at
  `lakehousectl update`, the command that installs another release (REL-2 S4);
- a `state.json` that names no release is refused rather than overwritten: the tool cannot tell
  what is installed, and it fails closed.

The release's own files — the Compose file, the manifest and its signature — are written again:
they are the bytes of the release, checked against the signed manifest, so a re-install of the
same release writes the same bytes. Three tests: the state survives a re-install of the same
release, another release is refused with the update named, and a state that names no release is
refused and left alone.

**SHOULD-FIX 4 (`bf0f7c43`).** With an existing `.env`, `secrets::write_env()` read
`AUTH_BOOTSTRAP_EMAIL` from the compiled-in template, so a re-install printed
`admin@example.com` while the stack was configured with the address the operator had put in the
file. The email is now read from the `.env` that is there. Two tests: a fresh `.env` takes the
email from the template, and an existing `.env` gives back the email it holds.

**SHOULD-FIX 5 (`38ccc6f4`).** `install.sh` ended with `exec "$work/$BINARY" install "$@"`.
`exec` replaces the shell, so the `EXIT` trap that removes `$work` never ran, and the downloaded
binary stayed on the host after every install. The installer is run instead of exec'd, and the
script exits with the installer's own status. Two tests: the files the script downloaded are gone
when it ends, and the script's status is the installer's status. The first was checked against the
OLD script, where it fails with "the binary was left behind".

**Commands run, in the foreground, on the final commit `38ccc6f4`:**

| Command | Result |
| --- | --- |
| `cargo fmt --check` (the whole workspace, through `bin/cargo-locked`) | clean, no diff |
| `cargo clippy -p lakehousectl --all-targets -- -D warnings` | clean, no warning and no error |
| `cargo test -p lakehousectl` | **53 passed, 0 failed** (43 before this round, 10 new) |
| `bash -n ops/release/install.sh` | clean, exit 0 |
| `shellcheck ops/release/install.sh` (0.10.0) | clean, exit 0 |
| `python -m pytest ops/release -q` | **132 passed, 0 failed** (130 before this round, 2 new) |
| `python3 -m unittest discover -s ops/release -p 'test_*.py'` | **132 tests, OK** |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | no violation |
| `git status` | clean (nothing generated staged) |

Every cargo command went through `/home/shiro/rantai/bin/cargo-locked` (the `bin/cargo-locked` of
the earlier rounds is not in the repository), in the foreground, on the shared
`CARGO_TARGET_DIR`; no `cargo clean`; no workspace build. The pytest run used a virtualenv's
interpreter (`/tmp/opencode/relvenv/bin/python -m pytest ops/release -q`): the system Python is
externally managed (PEP 668), so `pip install --user pytest` is refused, and the virtualenv
carries `pytest==8.3.4` over the system site packages (PyYAML, cryptography). `shellcheck`
(0.10.0) is not installed on this machine; it was already downloaded to `/tmp/opencode/bin` by
the earlier rounds of this slice.

**Not verified, with the reason:**

- **The PR's dry run and the `install-proof` job are not verified.** They need a GitHub runner:
  the four images, the throwaway registry, cosign, syft, a multi-GiB bundle, and the whole stack
  started on it. The PR's own CI is the proof.
- **The online download was not exercised against GitHub.** The test drives the online path with
  an injected fetch, and the `curl` call is the same one `install.sh` uses, but no real release
  was downloaded here: the release key and the releases are not on this machine.
- **The whole install was not run end to end.** It needs a manifest signed with the release key,
  and that key is in the `release` environment's secrets.
- **The static musl build is not verified here.** The `x86_64-unknown-linux-musl` target and
  `musl-tools` are not installed on this machine, and the pipeline installs both. The binary was
  built and tested natively — the same source, the same features, the same dependencies.
- **The full workspace run** (`cargo clippy --workspace --all-targets --all-features` and
  `cargo test --workspace`): not run here — this slice's machine rules say CI runs them; the crate
  was clipped with `--all-targets` and tested.
- **No bun command, no Dagster command and no Compose command ran**: this round changes no
  TypeScript, no code location and no Compose file.
- **`cargo deny check all` was not run** in this round: no dependency changed (the crate's
  `Cargo.toml` is untouched), and the earlier round of this slice ran it green.

### Fix for the PR #313 dry run (developer, 2026-11-01)

**Commit `3a004234`** (branch `feat/rel-2-s3-lakehousectl-install`): the musl target goes on the
PINNED toolchain.

The dry run failed in "Build lakehousectl (static, musl)" with `error[E0463]: can't find crate
for core ... the x86_64-unknown-linux-musl target may not be installed`. `rustup target add` ran
at the repository root, so it installed the target for the runner's DEFAULT toolchain, and the
build that followed ran inside `rust/`, on the toolchain pinned by `rust/rust-toolchain.toml`
(1.99.1) — which has no musl target.

All THREE steps that build `lakehousectl` for musl (the dry run, the release job and the install
proof) now change to `rust/` BEFORE `rustup target add`, so `rustup` reads the pin and adds the
target to it. The target is NOT added to `rust-toolchain.toml`: every developer reads that file,
and a target in it is a download every developer pays for a build they do not make.

**Commands run** (from the repository root, on `3a004234`):

- `actionlint .github/workflows/release.yml` — no findings, exit 0.
- `python3 -m pytest ops/release -q` — **135 passed**, 1 warning (the system pytest's `anyio`
  plugin `PytestRemovedIn10Warning`; it is there before this commit too, at 132 passed).
- `python3 ops/lint/check_intra_package_imports.py` — `OK: all intra-package dispar_orchestrate
  imports resolve`.
- `python3 ops/lint/check_bare_iceberg_count.py` — `R11 guard: no bare count() against a Bronze
  Iceberg table found.`

The three new tests in `ops/release/test_release_workflow.py`:

- `test_every_musl_target_is_added_on_the_pinned_toolchain` — every step that adds the musl
  target adds it AFTER `cd rust`, in the same step;
- `test_a_musl_target_added_outside_rust_is_reported` — the check was run against a synthetic
  workflow with the old order, and it fails there;
- `test_the_toolchain_pin_does_not_ask_for_the_musl_target` — `rust/rust-toolchain.toml` does
  not name the musl target.

The first check was also run against the OLD `release.yml` (the file at `3e24eeca`): it fails
there with "the musl target is added before `cd rust`".

**Not verified:** the dry run itself — it needs the `release` environment's key and a runner, so
the fix is proved here by the workflow's own checks and by the test that fails against the old
file, and the dry run is what proves it end to end. **No cargo ran**: this commit touches no
Rust.

### Fix for the PR #313 install-proof (developer, 2026-11-01)

**Commit `f9d00a2b`** (branch `feat/rel-2-s3-lakehousectl-install`): the proof reads the bundle from
`dry-run/dist/`, the folder the artifact actually keeps it in.

The dry run failed in "Install the bundle" with `tar (child): dry-run/rantai-lakehouse-*.tar.zst:
Cannot open: No such file or directory`.

`actions/upload-artifact` stores every file at its path relative to the ROOT of the upload list —
the least common ancestor of the search paths. That list names files from `dist/`, from `sbom/`
and from the repository root itself, so the root is the repository root and the folders survive.
The proof's `download-artifact` step unpacks the artifact into `dry-run/`, so the bundle is at
`dry-run/dist/rantai-lakehouse-*.tar.zst` and `cosign.pub` is at `dry-run/cosign.pub`. The
`cosign.pub` path was right; the bundle's path was not.

The layout was not guessed: the central directory of the artifact of the failed run (PR #313,
run `41300418587`, artifact `release-dry-run`) was read over HTTP range requests, and it holds
24 entries — `dist/rantai-lakehouse-0.1.0-linux-x86_64.tar.zst`,
`dist/rantai-lakehouse-0.1.0-linux-x86_64.tar.zst.sha256`, `release.json`,
`release.json.sigstore.json`, `docker-compose.release.yml`, `lakehousectl-linux-x86_64`,
`cosign.pub` and the 16 `sbom/*.cdx.json` files. Every file the proof reads is now on that list
under the same relative path.

**Commands run** (from the repository root, on the commit above):

- `actionlint .github/workflows/release.yml` — no findings, exit 0.
- `python3 -m pytest ops/release -q` — **139 passed**, 1 warning (the system pytest's `anyio`
  plugin `PytestRemovedIn10Warning`; it is there before this commit too, at 135 passed).
- `python3 ops/lint/check_intra_package_imports.py` — `OK: all intra-package dispar_orchestrate
  imports resolve`.
- `python3 ops/lint/check_bare_iceberg_count.py` — `R11 guard: no bare count() against a Bronze
  Iceberg table found.`

The four new tests in `ops/release/test_release_workflow.py`:

- `test_every_file_the_proof_reads_is_on_the_upload_list` — every path the proof job reads under
  `dry-run/` is on the dry run's upload list, under the SAME relative path (the folder is
  compared, not just the glob, so `dist/*.tar.zst` cannot stand for a file read from the
  artifact's root);
- `test_a_proof_reading_a_file_the_dry_run_does_not_upload_is_reported` — the check above was run
  against a synthetic workflow whose proof reads the bundle from the artifact's root, and it
  fails there with `dry-run/rantai-lakehouse-*.tar.zst`;
- `test_the_proof_downloads_the_artifact_the_dry_run_uploaded` — the upload step and the
  download step name the SAME artifact;
- `test_the_proof_unpacks_the_artifact_where_it_reads_it` — the proof reads from the folder it
  unpacked the artifact into.

The first check was also run against the OLD `release.yml` (the file at `ab70ff2f`): it fails
there with `dry-run/rantai-lakehouse-*.tar.zst`, and passes against the fixed file.

**Not verified:** the dry run itself — it needs the `release` environment's key and a runner, so
the fix is proved here by the workflow's own checks, by the test that fails against the old file
and by the artifact's own central directory. **No cargo ran**: this commit touches no Rust.

### Fixes for review S3 round 3, D13 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s3-lakehousectl-install`), one per item, each pushed:

| Commit | Item | What |
| --- | --- | --- |
| `7c2b171f` | 1 — `ops/release/make_bundle.sh` | the bundle is built on the containerd image store, from tags |
| `4737eabc` | 2 — `.github/workflows/release.yml` | the containerd image store in three jobs, and an air-gapped install proof |
| `0c181076` | 3 — `rust/crates/lakehousectl` | the `image-store` pre-flight check, and `docker compose up --pull never` |

**Item 1 (`7c2b171f`).** `make_bundle.sh` refuses to run unless Docker's containerd image store
is on: `docker info --format '{{json .DriverStatus}}'` must name `io.containerd.snapshotter.v1`.
The refusal says how to switch (`{"features": {"containerd-snapshotter": true}}` in
`/etc/docker/daemon.json`, then a restart of Docker) and that the classic store's images are
hidden while the containerd store is on. For each image the script now pulls the digest
reference, TAGS it (`docker tag <ref@digest> <ref-without-digest>`) and saves the TAG — never
`--platform`, and never the digest reference. The header says why, and the site's comment says
why.

The three new tests in `ops/release/test_make_bundle.py` use the fake `docker` (no network): the
bundle saves each image by its tag and not by the digest reference, it never passes `--platform`,
and it is refused on a classic store with the daemon.json snippet in the message.

**Item 2 (`4737eabc`).** Three jobs, one step each, named "Turn on the containerd image store":
the daemon's own `daemon.json` is MERGED with `jq` (not replaced), Docker is restarted with
`systemctl restart docker`, and the store is ASSERTED with `docker info` — the step fails the
job when the store is not on. The step sits before the job's first image pull or build: the dry
run's step 3 (its first build is step 8), the release job's step 3 (its first build is step 9).
buildx builds in a builder container of its own, so the images it pushes come back through the
registry: the pull that has to happen on the containerd store is the "The published images, by
digest" step, and the step's comment says so.

`install-proof` gains the same step (before the install) and a second one, "Cut the runner off
from the registries": `/etc/hosts` sends `registry-1.docker.io`, `auth.docker.io`,
`index.docker.io`, `ghcr.io`, `quay.io` and `production.cloudflare.docker.com` to `0.0.0.0`, and
`docker pull alpine:3.20` must FAIL before the install runs. The proof is air-gapped in fact: a
runner that can still pull would let Compose fetch the images the bundle should have loaded.

The six new tests in `ops/release/test_release_workflow.py` check the ORDER (the store is on
before the first pull or build, in each of the three jobs; the store is on before the install;
the cut is before the install) and the air-gap step itself (the six hosts, `/etc/hosts`, and the
proof of the cut). Two synthetic workflows make the checks fail: one pulls an image with no
containerd step, the other installs before the cut. The checks were also run against the OLD
`release.yml` (the file at `7c2b171f`): they report `dry-run`, `release` and "the proof has no
air-gap step" there, and nothing against the fixed file.

**Item 3 (`0c181076`).** A new pre-flight check, `image-store`, is a function of the injected
facts like the others; the facts come from `docker info --format '{{json .DriverStatus}}'`. It
runs for a BUNDLE install only (`preflight::run(facts, ignored, bundle)`), because an online
install pulls every image by its digest and a pull registers the digest on either store. On the
classic store it refuses, with the D13 text: the daemon.json snippet, the restart of Docker, and
the warning that the classic store's images are hidden while the containerd store is on. A host
whose store could not be read is refused too — an air-gapped install does not guess.
`lakehousectl preflight --bundle` runs the check on its own.

`docker compose up` is now `docker compose up -d --pull never`, online and offline: a missing
image is an error, never a silent fetch. The arguments are built apart from the call
(`compose_up_args`), so a test can read them.

Eight new Rust tests: the containerd store passes the check, the classic store is refused with
the fix text, an unreadable store is refused, the check does not run for an online install, it
runs for a bundle install, it can be skipped by name, and the Compose arguments end with
`--pull never`.

**Commands run, in the foreground, on the final commit `0c181076`:**

| Command | Result |
| --- | --- |
| `/home/shiro/rantai/bin/cargo-locked fmt --check` (the whole workspace) | clean, no diff, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked clippy -p lakehousectl --all-targets -- -D warnings` | clean, no warning and no error, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl` | **61 passed, 0 failed** (53 before this round, 8 new) |
| `actionlint .github/workflows/release.yml` | no findings, exit 0 |
| `shellcheck ops/release/*.sh` (0.10.0) | clean, exit 0 |
| `python -m pytest ops/release -q` | **148 passed** (139 before this round, 9 new) |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` |
| `docker compose --profile '*' config --quiet` (throwaway env values) | clean, exit 0 |
| `git status` | clean (nothing generated staged) |

Every cargo command went through `/home/shiro/rantai/bin/cargo-locked` (the `bin/cargo-locked` of
the earlier rounds is not in the repository), in the foreground, on the shared
`CARGO_TARGET_DIR`; no `cargo clean`; no workspace build. The pytest run used the virtualenv's
interpreter (`/tmp/opencode/relvenv/bin/python -m pytest ops/release -q`, pytest 8.3.4 over the
system site packages). `shellcheck` (0.10.0) is not installed on this machine; it was downloaded
to `/tmp/opencode/d13/shellcheck` for this round.

**The measurement (item 4), on bluefin.** Docker 29.9.2, Compose v5.6.1, the host's own
containerd store (`io.containerd.snapshotter.v1`). `lakehousectl` was built with
`--features test-key` (a native debug build) and pointed at a THROWAWAY cosign key: cosign
v3.1.4 generated the key, signed the manifest into a Sigstore bundle
(`--use-signing-config=false --tlog-upload=false`, no transparency log), and the binary read the
throwaway public key through `LAKEHOUSECTL_PUBLIC_KEY_FILE`. The S2 fixture was NOT reused: its
private half is not in the repository, and the manifest of a real install has to name real images
and their IDs.

The bundle was made by the NEW `make_bundle.sh`, from a two-service Compose file
(`alpine/socat:1.8.1.3@sha256:466c8a03…` and `alpine:3.20@sha256:d9e853e8…`, both pinned by
digest), with the manifest written by `ops/release/make_manifest.py` and the image IDs read the
way the pipeline reads them (the config digest out of the image's own saved tar, because the
containerd store reports the index digest as the image's ID). The bundle is 9 823 287 bytes
(9 MiB). Its archives carry what D13 says they must: `index.json` names
`io.containerd.image.name: docker.io/alpine/socat:1.8.1.3` (and
`docker.io/library/alpine:3.20`) with the registry digest beside it, and `manifest.json` carries
the tag and the config digest the signed manifest names.

Then, in this order:

1. `docker rmi alpine/socat:1.8.1.3 alpine:3.20` — and both digest-pinned references stopped
   resolving (`No such image`), which is the state PR #313's install proof was in.
2. **The air gap, in fact.** `/etc/hosts` was given the six registry entries pointing at
   `0.0.0.0` (the same entries the proof job now writes), and the cut was PROVED:
   `docker pull alpine:3.20` failed with `dial tcp 0.0.0.0:443: connect: connection refused`,
   and `curl https://registry-1.docker.io/v2/` failed too.
3. `lakehousectl install --bundle unpacked --dir install --ignore-preflight disk
   --ignore-preflight memory` (the memory and disk checks are skipped because this host has
   13 GiB of RAM and `/tmp` is a 5 GB tmpfs — the same two skips the CI proof uses). It printed:

   ```
   lakehousectl: the manifest of release 0.2.0 verifies against the release key.
   lakehousectl: the disk check was skipped, as asked.
   lakehousectl: the memory check was skipped, as asked.
   lakehousectl: loading unpacked/images/socat.tar
   lakehousectl: loading unpacked/images/alpine.tar
   lakehousectl: starting the stack.
   ```

   The `image-store` check ran and passed (the containerd store is on). Both images loaded, and
   their IDs matched the SIGNED manifest — the tool would have stopped otherwise. The stack
   started: `docker compose -p rantai-lakehouse ps` showed
   `rantai-lakehouse-alpine-1  alpine:3.20@sha256:d9e853e8…  Up` and
   `rantai-lakehouse-socat-1  alpine/socat:1.8.1.3@sha256:466c8a03…  Restarting (1)` — socat
   exits because the port it connects to refuses, and the restart policy brings it back. The
   digest-pinned references RESOLVED after the load, and the loaded images' IDs are the registry
   digests the manifest pins (`sha256:466c8a03…`, `sha256:d9e853e8…`).
4. **How far it got:** through the signature, the pre-flight, the layout, both loads, the image
   ID check and the stack start — and it stopped at the READINESS PROBE, which cannot answer:
   neither socat nor alpine serves `/health/ready`. The tool waits 10 minutes for it; the run
   above was stopped by a 200-second timeout at that wait, so the tool's own "did not become
   ready within 10 minutes" message was NOT seen here.

**The buildx question (the order against the job).** Measured too, because the pipeline builds
with buildx and pulls the images back by digest: a tiny image (`FROM alpine:3.20`) was built by
`docker buildx build` in a builder container of its own and pushed to a local registry
(`localhost:5000/d13-probe:0.1.0@sha256:07295677…`); the runner pulled it back by digest (the
pipeline's "The published images, by digest" step), tagged it and saved the tag (the bundle's
path); after `docker rmi` and `docker load` the digest-pinned reference RESOLVED and the image's
ID was the pushed digest. So the images buildx pushes can still be pulled, tagged, saved and
loaded afterwards.

**The counterfactual.** The same image, saved from the DIGEST reference (what `make_bundle.sh`
did before this round): the archive's `index.json` carries NO `io.containerd.image.name`, and
after `docker rmi` + `docker load` the digest-pinned reference does NOT resolve
(`No such image`) — the image lands as a dangling `<none>`. That is the BLOCKER, reproduced and
then fixed.

**Everything created was removed afterwards.** The compose project, its containers and its
network (`docker compose -p rantai-lakehouse down -v`); the images this round pulled
(`alpine/socat:1.8.1.3`, `registry:2`, `moby/buildkit`, the probe image); the registry container
and the buildx builder; the temporary directories. `/etc/hosts` was restored from its backup and
the two files hash the same (`34237b3f095501aac5a64024e4ad4b7c`). `alpine:3.20` and
`localhost:5000/probe` were on the machine before the measurement and were left as they were.

**Not verified, with the reason:**

- **The PR's dry run and the `install-proof` job.** They need a GitHub runner: the four images,
  the throwaway registry, cosign, syft, a multi-GiB bundle and the whole stack. The workflow's
  own checks, and the checks run against the OLD workflow, are what stands in for them here.
- **The release build's own images** (`ghcr.io/rantai-dev/…`) were not in the measurement: the
  bundle's path does not care which registry an image comes from, and the release key is in the
  `release` environment's secrets. The measurement used third-party images and a throwaway key,
  as the task allowed.
- **The static musl build** was not verified here: the `x86_64-unknown-linux-musl` target is not
  installed on this machine. The measurement used the native debug build — the same source, the
  same features.
- **The readiness probe was not allowed to finish** (see item 4, step 4): the measurement's
  images do not serve `/health/ready`, so the run was stopped at the wait. The stack start is
  what was measured.
- **The SBOMs in the measurement bundle were stand-ins** (a minimal CycloneDX JSON per image),
  not syft's: the bundle only copies them, and the measurement is about the images and the
  install.
- **The full workspace run** (`cargo clippy --workspace --all-targets --all-features` and
  `cargo test --workspace`): not run here — this round changes one crate, and the crate's rules
  say the scoped run is what a commit runs. CI runs the full one on the PR.
- **No bun command and no Dagster command ran**: this round changes no TypeScript and no code
  location.
- **`cargo deny check`** was not run: no dependency changed (the crate's `Cargo.toml` is
  untouched).
- **An observation, not fixed:** `sys::output` captures a command's standard error and drops it,
  so `docker compose`'s own output never reaches the operator — and `compose_up`'s message says
  "The output above says why" while nothing is printed above. It is pre-existing and outside
  D13's decision; the review may want it as a separate finding.

### Fixes for review S3 round 4 (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s3-lakehousectl-install`), one per finding, each pushed:

| Commit | Finding | What |
| --- | --- | --- |
| `d92af2e2` | BLOCKER 1 — `.github/workflows/release.yml` | the dry run starts its throwaway registry itself, after the store switch |
| `19ac61cf` | SHOULD-FIX 2 — `rust/crates/lakehousectl` | a failed docker command carries Docker's own error output into the error |

**BLOCKER 1 (`d92af2e2`).** The dry run's registry was a `services:` container. GitHub starts a
service container before the job's first step, and the "Turn on the containerd image store" step
restarts Docker and switches the image store: the restart stops the registry container, and the
switch hides the image it ran from — the classic store's images are not in the containerd one. The
image builds that push to `localhost:5000` had no registry to push to.

The `services:` block is gone. The dry run starts the registry itself, in a step AFTER the switch:
`docker run -d --name release-registry --restart always -p 5000:5000 registry:2@sha256:a3d8…`, and
the step waits until `curl -fsS http://localhost:5000/v2/` answers — in a bounded loop (30 attempts,
2 seconds apart), so a registry that never answers fails the job instead of hanging it.

The image is PINNED by digest: `sha256:a3d8aaa63ed8681a604f1dea0aa03f100d5895b6a58ace528858a7b332415373`
is the `docker-content-digest` the registry answered for the tag `registry:2` on 2026-11-01
(measured, not assumed). The job's `RELEASE_REGISTRY` is unchanged (`localhost:5000`), and the
buildx builder still runs with `network=host`, so it reaches the registry's published port.

No other job of the workflow holds a service container: the `release` job and the `install-proof`
job have none, and the check below says so.

Six new tests in `ops/release/test_release_workflow.py`: no job that restarts Docker holds a
service container (a service that a step starts AFTER the switch is not reported), the dry run's
registry is started after the switch, is pinned by digest, is waited for with a bounded loop, and
is the registry the job pushes to. Two synthetic workflows make the check fail and pass: one with
a service container across the restart (reported), one that starts its service after the switch
(not reported). The check was also run against the OLD `release.yml` (the file at `02048fac`): it
reports `dry-run: registry` there, and nothing against the fixed file.

**SHOULD-FIX 2 (`19ac61cf`).** This is the observation the round-3 handoff left open. A new
`sys::output_or_stderr` carries the LAST 20 lines of what a command said on its error stream into
the error, on their own lines under the exit status. `sys::output` is unchanged: the `curl` calls
of `install.rs` and the pre-flight facts keep their own messages.

The docker calls use it: `pull`, `load`, `image_id` and `compose_up`. The messages are worded to
match what is printed: the advice ("Is this host online?") comes BEFORE the reason, and
`compose_up` no longer says "The output above says why" — the error IS the answer now.

No secret reaches that error output, and the module says so: the docker commands name images,
image files and the Compose file, and Docker's own error output names the same things back. The
signing key and the stack's secrets are never in a docker command's arguments, and Docker never
prints them.

Four new Rust tests: a failing command's error output is in the error; the error output is capped
at 20 lines — the LAST twenty of forty, not the first; a command that says nothing on its error
stream is an error of one line; and the stack error is the whole answer (it does not point at an
output that nothing prints).

**Commands run, in the foreground, on the final commit `19ac61cf`:**

| Command | Result |
| --- | --- |
| `/home/shiro/rantai/bin/cargo-locked fmt --check` (the whole workspace) | clean, no diff, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked clippy -p lakehousectl --all-targets -- -D warnings` | clean, no warning and no error, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl` | **65 passed, 0 failed** (61 before this round, 4 new) |
| `actionlint .github/workflows/release.yml` | no findings, exit 0 |
| `python3 -m pytest ops/release -q` | **154 passed** (148 before this round, 6 new) |
| `git status` | clean (nothing generated staged) |

Every cargo command went through `/home/shiro/rantai/bin/cargo-locked` (the `bin/cargo-locked` of
the earlier rounds is not in the repository), in the foreground, on the shared `CARGO_TARGET_DIR`;
no `cargo clean`; no workspace build.

**Not verified, with the reason:**

- **The PR's dry run.** It needs a GitHub runner: the four images, the throwaway registry, cosign,
  syft, a multi-GiB bundle and the whole stack. The workflow's own checks, and the check run
  against the OLD workflow, are what stands in for it here.
- **The registry container itself** was not started on this machine: the step needs a runner whose
  Docker has the containerd store on, and the digest of the image was read from the registry's own
  header rather than from a pull.
- **The `curl` calls of `install.rs`** keep their own messages: the finding named the docker
  calls, and `curl --show-error` already says why it failed next to the exit status.
- **The full workspace run** (`cargo clippy --workspace --all-targets --all-features` and
  `cargo test --workspace`): not run here — this round changes one crate, and the crate's rules
  say the scoped run is what a commit runs. CI runs the full one on the PR.
- **No bun command, no Dagster command and no compose command ran**: this round changes no
  TypeScript, no code location and no compose file.
- **`cargo deny check`** was not run: no dependency changed (the crate's `Cargo.toml` is
  untouched).

### Handoff S4a (developer, 2026-11-01)

**Commits** (branch `feat/rel-2-s4-lakehousectl-update`), one per task, each pushed:

| Commit | Task | What |
| --- | --- | --- |
| `fd25c005` | S4-T1 (D14) | the release ships every file its Compose file mounts |
| `5d659f66` | S4-T2 (D15) | one backup and one restore, shipped as tools |
| `ff9c334b` | S4-T3 (D16) | `MAINTENANCE_PAUSED` stops the jobs that delete files |

**S4-T1 (`fd25c005`, D14).** A relative bind-mount source
(`./ops/backups/clickhouse/backups.xml`) is a file the deployment needs NEXT TO its Compose file:
on a host that does not have it, Docker creates an empty DIRECTORY in its place — the service
starts, and it runs against nothing. The release now ships every relative mount of a service a
release runs, and the signed manifest names the checksum of each of them.

The rule lives in ONE place: `render_compose.relative_mount_sources` answers "which relative
mounts does this Compose file have", with the SAME `services_to_drop` the renderer drops the
acceptance harnesses by — so the answer is the same whether it is read from the template or from
the rendered file (rule 4). `make_manifest.py` asks that module for the answer instead of parsing
the file a second way, hashes each mounted file, and REFUSES a release that mounts a file it does
not carry. That refusal is the release-tooling check D14 asks for: the tooling fails when the
rendered Compose file mounts a relative source that `files` does not list. Two shipped files with
the same name are refused too, and so is a shipped file named after a release asset of its own
(the online install fetches a shipped file by its own name).

The install writes them. `files.rs` fetches every shipped file (from the bundle offline, from the
release's assets online), checks it against the SIGNED manifest, and writes it at the relative
path the release ships it at — the path the Compose file mounts it at. The bundle carries them
too: `make_bundle.sh` asks the manifest's own script for the list, checks each file against the
signed manifest, and puts it in the bundle at the path the install writes it at. The release job
publishes each of them as an asset of its own.

**S4-T2 (`5d659f66`, D15).** `lakehousectl backup` and `lakehousectl restore <dir>` run the two
scripts the release ships, under `tools/`. There is no Rust port of the steps: they have ONE
source. The two scripts gain `LAKEHOUSE_DIR` — the deployment's own directory, the one holding
`docker-compose.yml` and `.env`. It defaults to the repository's root, so a developer's behaviour
is unchanged; the scripts run `docker compose -p <project> -f "$LAKEHOUSE_DIR/docker-compose.yml"
--env-file "$LAKEHOUSE_DIR/.env"`, plus `override.yml` when there is one. The `.env` is named
only when it is there: a repository without one is a normal thing, and compose refuses to start on
a file that is not there.

The backup set gets `release.json` and `state.json` (D15): a restore that brings the data back
brings the release it was taken from back with it. The restore puts them back and keeps the ones
it replaces beside the new ones, so a restore never destroys the only copy. `lakehousectl backup`
records the backup it took in the install's state (`state.json`'s `lastBackup`) — the rollback of
an update reads that pointer (D18). The state's shape has ONE owner now (`state.rs`): the
install and the backup both read and write it there.

**S4-T3 (`ff9c334b`, D16).** The three Dagster services of the `dagster` profile get
`MAINTENANCE_PAUSED: ${MAINTENANCE_PAUSED:-}`. The jobs run in the code location, so that is the
service that reads the value; the daemon and the webserver carry it too, because `lakehousectl`
sets it in the install's `.env` and the services that read it are recreated by `up -d`.

While `MAINTENANCE_PAUSED=true` is set, the two verbs that DELETE files — the snapshot expiry and
the orphan-file removal — skip, the reason is recorded in `skipped_verbs`, and the run does not
fail. The value is read as `== "true"`: anything else, and an unset variable, is not a pause. The
Gold export's trim deletes files too, so it reads the same variable and records its own skip
(`console.gold_trim_run`, status `skipped`, reason `maintenance_paused`).

**The real run (S4-T2), on bluefin.** Docker 29.9.2, Compose v5.6.1, the host's own containerd
store (`io.containerd.snapshotter.v1`). `lakehousectl` was built with `--features test-key` (a
native debug build) and pointed at a THROWAWAY cosign key: cosign v3.1.4 generated the key and
signed the manifest into a Sigstore bundle (`--use-signing-config=false --tlog-upload=false`, no
transparency log), and the binary read the throwaway public key through
`LAKEHOUSECTL_PUBLIC_KEY_FILE`.

The bundle was made by the NEW `make_bundle.sh`, from a small Compose file of its own — Postgres
and ClickHouse, and nothing else — with the manifest written by `ops/release/make_manifest.py`
and the image ids read the way the pipeline reads them (the config digest out of the image's own
saved tar, because the containerd store reports the index digest as the image's id). The bundle
is 437 640 170 bytes (417 MiB) and it holds what D14 says it must: the mounted file
(`ops/backups/clickhouse/backups.xml`) and the two tools (`tools/backup-local.sh`,
`tools/restore-from-backup.sh`), each with its SHA-256 in the bundle's own `SHA256SUMS`.

Then, in this order:

1. `lakehousectl install --bundle bundle --dir install --ignore-preflight disk
   --ignore-preflight memory`. It wrote the layout, and the layout held the shipped files:

   ```
   install/docker-compose.yml
   install/.env
   install/ops/backups/clickhouse/backups.xml
   install/release.json
   install/release.json.sigstore.json
   install/state.json
   install/tools/backup-local.sh
   install/tools/restore-from-backup.sh
   ```

   Both images loaded and their ids matched the SIGNED manifest. The stack started, and
   `docker compose -p rantai-lakehouse ps` showed both services `Up (healthy)`. The install then
   waited for `/health/ready`, which this small stack cannot answer — there is no API in it — and
   the run was stopped at that wait (the same place the D13 measurement stopped). The tool's own
   "did not become ready within 10 minutes" message was NOT seen here.
2. The markers, one row in each database: `the postgres marker` in Postgres, `the clickhouse
   marker` in ClickHouse.
3. `lakehousectl backup --dir install` ran the SHIPPED script, and it said:

   ```
   ==> Backing up compose project 'rantai-lakehouse' to .../install/backups/20261101T192513Z-unknown
   --> Postgres: pg_dumpall
   --> ClickHouse: BACKUP DATABASE ... TO Disk('backups', ...)
       default
   --> ClickHouse: copying the backup files out of rantai-lakehouse_lakehouse_clickhouse_backups
   --> Configuration: .env
   --> Release: release.json
   --> State: state.json
   --> Manifest: manifest.json

   ==> Backup complete: .../install/backups/20261101T192513Z-unknown
       parts:     18
       size:      64631 bytes
   lakehousectl: the backup is recorded in the install's state.
   ```

   The ClickHouse `BACKUP DATABASE ... TO Disk('backups', ...)` worked BECAUSE the release
   shipped `ops/backups/clickhouse/backups.xml` and the install wrote it next to the Compose
   file: without D14 the disk does not exist and the backup cannot run at all. The install's
   `state.json` held `"lastBackup": ".../backups/20261101T192513Z-unknown"` afterwards.
4. The volumes were destroyed: `docker compose -p rantai-lakehouse down -v` removed the three
   volumes and the network, and `docker volume ls --filter "name=^rantai-lakehouse_"` was empty.
5. `lakehousectl restore .../backups/20261101T192513Z-unknown --dir install` ran the SHIPPED
   restore script: the checksums were checked first, Postgres was restored with `pg_dumpall`,
   ClickHouse was restored with `RESTORE DATABASE ... FROM Disk('backups', ...)` (it needed a
   second attempt, which the script retries), the `.env`, `release.json` and `state.json` were put
   back, and the stack came up again. It reported `Total restore time: 12s`.
6. **The markers are back in both databases:**

   ```
   $ docker compose -p rantai-lakehouse ... exec -T postgres psql -U lakehouse -d lakehouse \
       -tAc "SELECT id, note FROM s4_marker;"
   1|the postgres marker
   $ docker compose -p rantai-lakehouse ... exec -T clickhouse clickhouse-client \
       -q "SELECT id, note FROM s4_marker"
   1	the clickhouse marker
   ```

**Everything created was removed afterwards.** The compose project, its containers, its network
and its three volumes (`docker compose -p rantai-lakehouse down -v`); the `postgres:16.15` TAG
the bundle's `docker tag` added (the image itself was already on the machine, as `postgres:16`,
and the tag was removed — `docker rmi postgres:16.15` untagged it and the image stayed); the
temporary directories and the 417 MiB bundle. The ClickHouse image was LEFT on the machine: a
container that predates this measurement (`charming_yonath`, `intelligent_easley`) uses it, and
removing an image a stopped container holds would break that container's ability to start. The
throwaway cosign key went with the temporary directory.

**Commands run, in the foreground, on the final commit `ff9c334b`:**

| Command | Result |
| --- | --- |
| `/home/shiro/rantai/bin/cargo-locked fmt --check` (the whole workspace) | clean, no diff, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked clippy --workspace --all-targets --all-features -- -D warnings` | clean, no warning and no error, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked test --workspace` | **8848 passed, 0 failed**, 51 ignored, in 164 test binaries |
| `/home/shiro/rantai/bin/cargo-locked clippy -p lakehousectl --all-targets -- -D warnings` | clean, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl` | **89 passed, 0 failed** (65 before this slice, 24 new) |
| `python3 -m pytest ops/release -q` | **177 passed** (158 before this slice, 19 new) |
| `(cd dagster && python -m pytest dispar_orchestrate -q -k "maintenance or expiry or orphan")` | **60 passed**, 2349 deselected, 7 subtests passed (9 new) |
| `(cd dagster && python -m pytest dispar_orchestrate/test_gold_export.py -q)` | **45 passed** |
| `(cd dagster && python -m pytest dispar_orchestrate -q)` (the whole code location) | **2408 passed, 1 skipped**, 40 subtests passed, in 18m30s |
| `actionlint .github/workflows/release.yml` | no findings, exit 0 |
| `shellcheck ops/release/*.sh ops/backups/backup-local.sh scripts/restore-from-backup.sh` (0.10.0) | clean, exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` |
| `docker compose --profile '*' config --quiet` (throwaway env values) | clean, exit 0 |
| `git status` | clean (nothing generated staged) |

Every cargo command went through `/home/shiro/rantai/bin/cargo-locked` (the `bin/cargo-locked` of
the earlier rounds is not in the repository), in the foreground, on the shared
`CARGO_TARGET_DIR`; no `cargo clean`. The pytest run used the system interpreter
(`python3 -m pytest ops/release -q`); the Dagster runs used the repository's own virtualenv
(`dagster/.venv/bin/python`, pytest 9.2.0). `shellcheck` (0.10.0) is not installed on this
machine; the copy in `/tmp/opencode/bin/shellcheck` was used.

(The `cargo test -p lakehousectl` count in the S4-T1 commit message says "66 before this commit,
10 new"; the real counts are 65 before the slice and 76 after S4-T1 — the message is off by one.
The table above is the count.)

**One thing in the plan is wrong against the code, and it is reported rather than improvised.**
D14's list of "today's" relative mounts names TWO files —
`ops/backups/clickhouse/backups.xml` and `ops/trino/optimize_bronze.sh` — and the "Gap found by
this check" says "The other relative mounts are test runners in test profiles". That is not what
the code says: the release's Compose file mounts a THIRD relative source, and it is not a test
runner:

```
ops/seaweedfs/iam.template.json   (the `seaweedfs-iam-init` service, profile `seaweedfs`)
```

`seaweedfs-iam-init` renders `ops/seaweedfs/iam.template.json` into the `lakehouse_seaweedfs_iam`
volume, so on a host without the file Docker creates an empty directory in its place and the
service renders nothing. D14's RULE — "each relative bind-mount source used by a service that a
release runs (no test-runner profile)" — covers it, so the release ships all THREE and the new
tooling test asserts exactly that set. The plan's enumeration should be corrected by the planner;
the decision itself was not changed.

**Not verified, with the reason:**

- **The PR's dry run and the `install-proof` job.** They need a GitHub runner: the images, the
  throwaway registry, cosign, syft, a multi-GiB bundle and the whole stack. The workflow's own
  checks, and the checks run against the OLD workflow, are what stands in for them here.
- **The ONLINE path of the shipped files.** The measurement installed from a BUNDLE (the offline
  path). The online path — the release publishing one asset per shipped file and the install
  fetching it by its own name — is covered by a Rust test with a fake fetch, not by a real
  download from a GitHub release: the release does not exist yet.
- **The static musl build** was not verified here: the `x86_64-unknown-linux-musl` target is not
  installed on this machine. The measurement used the native debug build — the same source, the
  same features.
- **The readiness probe was not allowed to finish**: the measurement's stack holds no API, so
  `/health/ready` cannot answer. The stack start, the backup and the restore are what was
  measured.
- **The release build's own images** (`ghcr.io/rantai-dev/…`) were not in the measurement: the
  bundle's path does not care which registry an image comes from, and the release key is in the
  `release` environment's secrets. The measurement used third-party images and a throwaway key,
  as the task allowed.
- **The SBOMs in the measurement bundle were stand-ins** (a minimal CycloneDX JSON per image),
  not syft's: the bundle only copies them, and the measurement is about the files and the
  backup/restore.
- **No bun command ran**: this slice changes no TypeScript.
- **`lakehousectl update` and `lakehousectl rollback`** are NOT in this slice (S4-T4, D17 and
  D18). With them comes the third bullet of D16 — `lakehousectl` setting the pause in the
  install's `.env` before the backup of an update and clearing it after a success or a rollback
  — because there is no `update` to set it for yet.
- **`cargo deny check`** was not run: no dependency changed (no crate's `Cargo.toml` is touched).

### Fixes for review S4a round 1 (developer, 2026-11-01)

The reviewer's three SHOULD-FIX findings, one commit each, in the order the review names them. Every
commit is on `feat/rel-2-s4-lakehousectl-update` — the review commit `f34f28e0` is the base — and was
pushed as it was made.

| Commit | Finding | What |
| --- | --- | --- |
| `e11abbf6` | SHOULD-FIX 1 | a shipped file's path may not escape the install's own directory |
| `4b6ca96c` | SHOULD-FIX 2 | two files of the same name are not published as two assets |
| `4e61a122` | SHOULD-FIX 3 | the repository's own compose command is the one it always was |

**SHOULD-FIX 1 (`e11abbf6`).** `files::write` joins the install's own directory with the manifest's
key. The manifest is signed, but a key that is absolute, or that holds a `..` component, would
write OUTSIDE the directory the install owns — and a Compose file can really mount such a path:
`render_compose.relative_mount_sources` accepts a `../` source and hands it over as it is.

Both sides now refuse it, with the SAME rule: the path must be relative and must hold normal
components only — no leading `/`, no `.`, no `..`, no empty component.

- `make_manifest.py`: `check_install_path` refuses such a path. `shipped_files` calls it for every
  mount and every tool, and `build_manifest` calls it for every path the manifest names.
- `lakehousectl`'s `files.rs`: `check_install_path` refuses it BEFORE the file is read from the
  bundle or downloaded, and before it is written. A path that escapes is neither fetched nor
  written.

**SHOULD-FIX 2 (`4b6ca96c`).** The release publishes one asset per file, and the asset's name is
the file's own name — its LAST path component (`files::asset_name`). Two files whose last component
is the same are two assets with the same name, and the online install fetches a shipped file by that
name: it could not tell them apart. `make_manifest.py` now owns the rule in ONE place:
`check_file_names` refuses two paths whose last component is the same; `check_shipped_names` (the
rule for the shipped files) calls it, and `build_manifest` calls it over the WHOLE `files` mapping
of the manifest, so the manifest's own entries are part of the same rule.

**SHOULD-FIX 3 (`4e61a122`).** Before S4, both scripts ran `docker compose -p <project>` in the
repository root, and compose found the Compose file — and a developer's own
`docker-compose.override.yml` — by itself. S4 made them always pass `-f docker-compose.yml`, which
stopped that. Both scripts now name the Compose file, the `.env` and `override.yml` ONLY when
`LAKEHOUSE_DIR` is set — that is, when the deployment is somewhere else than the repository. When it
is not set, the command is exactly the old one. `lakehousectl backup` and `lakehousectl restore`
both set `LAKEHOUSE_DIR` (`backup.rs`), so an installed host is unaffected. The scripts read the
deployment's own directory from `DEPLOYMENT_DIR`, which is `LAKEHOUSE_DIR` when it is set and the
repository's root when it is not.

**Commands run, in the foreground, on the final commit `4e61a122`:**

| Command | Result |
| --- | --- |
| `cargo fmt --check` (the whole workspace) | clean, no diff, exit 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean, no warning and no error, exit 0 |
| `cargo test --workspace` | **8854 passed, 0 failed**, 51 ignored, in 164 test binaries (8848 before these fixes, 6 new) |
| `cargo clippy -p lakehousectl --all-targets -- -D warnings` | clean, exit 0 |
| `cargo test -p lakehousectl` | **95 passed, 0 failed** (89 before these fixes, 6 new) |
| `python3 -m pytest ops/release -q` | **187 passed** (177 at the review commit `f34f28e0`, 10 new) |
| `shellcheck ops/backups/backup-local.sh scripts/restore-from-backup.sh` (0.10.0) | clean, exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` |
| `(cd dagster && .venv/bin/python -m pytest dispar_orchestrate -q)` | **2411 passed, 1 skipped**, 40 subtests passed, in 18m13s (on the merged tree; 2408 before `main` came in) |
| `bun run typecheck` | clean, exit 0 |
| `bun run test` | **3082 pass, 1 skip, 0 fail**, 8372 expect() calls, in 313 files, 73.8s |
| `bun run lint` | **12356 problems (649 errors, 11707 warnings)** — red on `main` already, and this branch changes no TypeScript (see below) |
| `docker compose --profile '*' config --quiet` (throwaway env values) | clean, exit 0 |
| `git status` | clean (nothing generated staged) |

Every cargo command ran in the foreground on the shared `CARGO_TARGET_DIR`
(`/home/shiro/rantai/cargo-target`); no `cargo clean`. The pytest run used the system interpreter
(`python3 -m pytest ops/release -q`); the Dagster run used the repository's own virtualenv
(`dagster/.venv/bin/python`). `shellcheck` (0.10.0) is not installed on this machine under that
name; the copy in `/tmp/opencode/tools/shellcheck-v0.10.0/shellcheck` was used, as in the S4a
handoff. There is no `bin/cargo-locked` in the repository, so plain `cargo` was used.

**`main` was merged into this branch while the fixes were in.** `699798ed` ("Merge branch 'main'
into feat/rel-2-s4-lakehousectl-update") was pushed to the branch after the three fix commits, and
this handoff sits on top of it. The merge touches NONE of the files these fixes touch
(`ops/release/make_manifest.py`, `ops/release/test_release_files.py`,
`ops/release/test_backup_scripts.py`, `ops/backups/backup-local.sh`, `scripts/restore-from-backup.sh`,
`rust/crates/lakehousectl/src/files.rs`), and the checks for what was touched were re-run on the
merged tree: `cargo fmt --check` clean, `cargo clippy -p lakehousectl --all-targets -- -D warnings`
clean, `cargo test -p lakehousectl` **95 passed**, `python3 -m pytest ops/release -q` **187 passed**,
`shellcheck` on both scripts clean, the two `ops/lint` scripts OK, and the Dagster code location
**2411 passed, 1 skipped** (main's own code came with the merge — it is not this slice's). The
whole-workspace run in the table above was measured on `4e61a122`, BEFORE that merge, and the
reviewer runs the full block on the PR head anyway.

**The tests of the developer's path are real.** The three tests of the repository's own command (the
backup, the backup with an override of its own, and the restore) were run against the PRE-FIX
scripts — the two scripts were stashed, and the three tests failed — and pass against the fixed
ones.

**`bun run lint` is red on `main`, and this branch does not cause it.** This branch changes no
TypeScript: `git diff origin/main --stat -- src/ design-system/ design-kit/ components.json
package.json bun.lock` is empty, so the lint's count is the count on `main`. It is named here for
the reviewer (AGENTS.md, step 6).

**Not verified, with the reason:**

- **No PR is opened**: the developer never opens or merges one (AGENTS.md, step 6).
- **The real run on bluefin was not repeated.** The three fixes are two refusals and one shell
  command: the refusals are covered by the tests above, and the command by the two scripts' own
  tests with `docker` and `curl` as stand-ins. A real stack run would show what the S4a
  measurement already showed, and the release a real install needs does not exist yet.
- **The ONLINE path of the shipped files** is still covered by a Rust test with a fake fetch, not by
  a real download from a GitHub release (as in the S4a handoff).
- **The static musl build** was not verified here (as in the S4a handoff): the
  `x86_64-unknown-linux-musl` target is not installed on this machine.
- **`cargo deny check`** was not run: no dependency changed (no crate's `Cargo.toml` is touched).

### Handoff S4b (developer, 2026-11-02)

The developer of S4b lost their machine (bluefin) before this handoff was written, and the
reviewer wrote "Review S4b round 1" instead. This is the handoff the slice was missing: the
commits that are on the branch, read from the code, the counts of the checks, and what was not
verified.

**Commits** (branch `feat/rel-2-s4b-lakehousectl-update`, base `main` at `c239e3b0`), in plan
order:

| Commit | Task | What |
| --- | --- | --- |
| `42fa7b6b` | S4-T4 (D17, D18) | `lakehousectl update` and `lakehousectl rollback`: the nine steps of D17 in their order (`update.rs`), the restore of D18 (`rollback.rs`), and every step that touches the machine behind ONE trait, `runner::Runner` (`runner.rs`), so the whole path is driven by a fake |
| `ac095fc2` | S4-T5 (D19) | `lakehousectl status`: the five things D19 names — the installed release and the one before it, every service's state, the API's own `/health/ready`, the backups newest first with their size, and whether a newer release exists (offline it says it did NOT check) |
| `11a43698` | S4-T6 (D20) | `lakehousectl self-update`: the newest release's manifest is verified with the compiled-in key, `lakehousectl-linux-x86_64` is checked against the SIGNED manifest's `files["lakehousectl"]`, and the running binary is replaced by a rename — atomically |
| `d625454a` | S4-T7 (D21) | the `update-proof` job of the dry run: bundles B and C are built out of bundle A, the markers are seeded before the first update, and the job's own rules live in `ops/release/update_proof.py`, so they are tested with no Docker and no network |
| `1f9095c0` | fix | the shipped scripts are given an ABSOLUTE directory: the script `cd`s into the deployment's directory and hands its paths to `docker run -v`, and Docker reads a relative path there as a volume NAME |
| `db95e794` | review round 1 BLOCKER 1 | a failure in step 4 or step 5 puts the stack back — this handoff's fix |

**The BLOCKER 1 fix (`db95e794`).** `update::run` set `MAINTENANCE_PAUSED` and stopped the writers
(step 4), then took the backup (step 5). A failure in either one returned the error with the
writers STOPPED and the pause STILL SET — and nothing had been swapped, so the install still held
the OLD release's files. The operator was left with a stack that was down and a pause that had to
be cleared by hand.

A failure in step 4 or step 5 now puts the install back the way it was:

- the stack is started again on the OLD files (`up -d --pull never` — nothing was swapped, so the
  Compose file and the manifest in the install's own directory are still the old ones);
- it is waited for on the OLD version, with the same timeout the update waits with
  (`--ready-timeout`, default 600 s);
- the pause is cleared: the update restored nothing, so the jobs that delete files must not stay
  paused;
- the error says that the update stopped before it changed anything, what failed, and what the
  stack runs now.

A stack that does not come back is reported as it is — "the stack could not be started again",
and "the install may need a hand". The error never claims a stack that is not there.

Three steps can fail before the swap: the pause cannot be set, the writers cannot be stopped, and
the backup cannot be taken (nor recorded in the state). All three go through the same way back.

**The tests are real.** The three new tests were run against the code BEFORE this commit — the
fix was reverted, the tests kept — and all three failed (162 passed, 3 failed). With the fix they
pass. They use the fake runner the module already has: when the backup is what fails, the fake's
steps are `fetch-images`, `stop-writers`, `backup`, `up`, `console-ready` (when the writers are
what fail, there is no `backup`) — the stack is put back, and nothing is restored and nothing is
rolled back, for nothing was swapped. The two tests of the blocker's own words assert that the
stack runs the old version, that `.env` holds no `MAINTENANCE_PAUSED`, and that `state.json` is
byte for byte the file it was.

**Commands run, in the foreground, on the final commit `db95e794`:**

| Command | Result |
| --- | --- |
| `/home/shiro/rantai/bin/cargo-locked fmt --check` (the whole workspace) | clean, no diff, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked clippy -p lakehousectl --all-targets -- -D warnings` | clean, no warning and no error, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl` | **165 passed, 0 failed** (162 at the review commit `20f45a3a`, 3 new) |
| `python3 -m pytest ops/release -q` | **209 passed** |
| `actionlint .github/workflows/release.yml` | no findings, exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | `OK: all intra-package dispar_orchestrate imports resolve` |
| `python3 ops/lint/check_bare_iceberg_count.py` | `R11 guard: no bare count() against a Bronze Iceberg table found.` |
| `git status` | clean (nothing generated staged) |

Every cargo command went through `/home/shiro/rantai/bin/cargo-locked` (the shared
`CARGO_TARGET_DIR`, `/home/shiro/rantai/cargo-target`; no `cargo clean`), in the foreground. The
pytest run used the throwaway virtualenv at `/tmp/opencode/relvenv/bin/python` (pytest 9.2.0) —
the system `python3` on this machine has no pytest. `actionlint` is not installed under that name
on this machine; the copy at `/tmp/opencode/actionlint` was used.

**A flaky test on this branch, found while running the block above.** `cargo test -p
lakehousectl` fails about once in twenty runs, in `backup::tests`:

```
thread 'backup::tests::the_backup_is_told_which_version_it_belongs_to' panicked at crates/lakehousectl/src/backup.rs:323:60:
the backup runs: Failed("the backup tool failed: Text file busy (os error 26)")
```

It is NOT from this fix: the failing test is in `backup.rs`, which this fix does not touch, and
the same failure was reproduced at the review commit `20f45a3a` (162 tests, 1 failed, on run 23
of 25). `backup.rs` was last changed by `1f9095c0`, which added the `fake_tool` helper: the test
writes a shell script into a temporary directory and the tool then EXECUTES it. `ETXTBSY` is what
the kernel says when the file being executed still has an open write descriptor — in a test binary
with four threads, another thread's `fork` inherits the descriptor of the file the first thread is
writing, and the inherited descriptor keeps the inode "open for writing" until that child execs
its own script. Writing the script through a temporary name and renaming it into place does NOT
avoid it (measured: the failure still came, on run 8 of 40).

**The reviewer decides what to do with it.** The candidates are: a retry on `ETXTBSY` in
`sys::output_or_stderr_with_env` (production code, and `ETXTBSY` on a script that is being
rewritten under the tool is a real condition), the fake tool becoming a committed fixture that is
SYMLINKED into the temporary directory (the inode that is executed is then never opened for
writing), or a single-threaded run of that module. It is not fixed here: it is outside BLOCKER 1,
and the fix belongs to whoever owns `backup.rs`.

**One thing the reviewer may want to look at.** The pause is cleared in the install's `.env`
AFTER the stack has been started (`up -d`) — in the update's step 8, in the rollback, and now in
the way back this fix adds. Compose reads the `.env` when it starts the services, so the Dagster
services that read `MAINTENANCE_PAUSED` (D16) keep the value they were started with until the
next `up -d`. That is the branch's own behaviour, in the success path as much as in this one, and
it is not changed here: it is named so that the reviewer can decide.

**Not verified, with the reason:**

- **No real update run on a host.** The developer who wrote the S4b commits lost their machine
  (bluefin) before this handoff was written, and this machine has no install of a release to
  update: no release is published yet, and the update's own path (the writers stopped, the backup
  taken, the files swapped, the stack waited for, the rollback) is covered by the unit tests with
  the fake runner. The proof is the `update-proof` job, below.
- **The PR's dry run and its `update-proof` job.** They need a GitHub runner: the images, the
  throwaway registry, cosign, syft, a multi-GiB bundle and the whole stack. What stands in for
  them here is the job's own logic (`ops/release/update_proof.py`, tested in
  `ops/release/test_release_workflow.py` — 209 passed with the rest of `ops/release`) and
  `actionlint` on the workflow. The job ITSELF is proved by the dry run.
- **The whole-workspace cargo run** (`cargo clippy --workspace --all-targets --all-features -- -D
  warnings` and `cargo test --workspace`): not run here — this slice's machine rules say CI runs
  the workspace and the developer runs the scoped commands. The crate the slice touches
  (`lakehousectl`) was clipped and tested with `--all-targets`.
- **The Dagster code location** (`(cd dagster && python -m pytest dispar_orchestrate -q)`): not
  run — the branch changes no Python under `dagster/` (`git diff origin/main...HEAD -- dagster/`
  is empty). The two repo lints that scan it ran, and both are OK.
- **No bun command ran**: the branch changes no TypeScript (`git diff origin/main...HEAD -- src/`
  is empty).
- **`docker compose --profile '*' config --quiet`**: not run — the branch changes no compose file
  (`git diff origin/main...HEAD -- docker-compose.yml` is empty).
- **The branch has not merged `main`**: its base is `c239e3b0`, and `main` has moved 16 commits
  on since. The reviewer decides when to merge it; the checks above were run on the branch as it
  is.

### Fixes for review S4b round 2 (developer, 2026-11-02)

**Commits** (branch `feat/rel-2-s4b-lakehousectl-update`, on top of the review commit `270293cf`):

| Commit | Task | What |
| --- | --- | --- |
| `e1a37beb` | review round 2 BLOCKER 2 | the pause does not outlive the update: every path that clears `MAINTENANCE_PAUSED` recreates the services that read it |
| `fe77e06c` | review round 2 SHOULD-FIX 3 | the `backup::tests` flake: the fake tool is a committed file, symlinked into the temporary directory |

**BLOCKER 2 (`e1a37beb`).** Three paths clear the pause: the update's success (step 8), the
rollback (step 5), and the update's way back from a failure in step 4 or step 5. Every one of them
now recreates the services that read the pause, right after the clear:

- `pause::clear_and_recreate` owns the pair — the clear and the recreate — so no path can clear
  the pause and leave the services running with it (`pause.rs`);
- the recreate is the `up -d --pull never` the tool already runs: it recreates only the services
  whose environment changed, and it pulls nothing (REL-2 S3 D13);
- a recreate that fails is reported plainly, never hidden: the error says the services that read
  the pause could not be recreated, and that the jobs that delete files stay skipped until they
  are. The update's own success says the update is done and the stack runs the new release; the
  rollback's says what the rollback did;
- the state names the release the stack runs BEFORE the pause is cleared, in the update and in the
  rollback: a pause that could not be cleared must not leave the state naming the other release.

**The tests are real.** Five new tests, and the fake runner of `update.rs` and `rollback.rs` now
reads the install's `.env` at every call — so each test shows the pause being set before the
backup, the stack being created WITH it, and the stack being created again without it:

| Test | What it holds |
| --- | --- |
| `update::tests::the_services_are_recreated_after_the_pause_is_cleared` | the success path: the clear is followed by a start, and that start ran without the pause |
| `update::tests::the_services_are_recreated_when_the_update_stops_before_the_swap` | the step 4/5 way back: the same, after the stack was put back on the old files |
| `update::tests::a_failed_recreate_after_the_pause_is_cleared_is_reported` | the second start can fail, and then the error says so |
| `rollback::tests::the_rollback_recreates_the_services_after_the_pause_is_cleared` | the rollback: the same |
| `rollback::tests::a_rollback_that_cannot_recreate_the_services_says_so` | and its failure is reported |

The fix was reverted, the tests kept: 9 tests failed (161 passed, 9 failed). With the fix, 170 pass.

**SHOULD-FIX 3 (`fe77e06c`).** The fake tool the `backup::tests` run is now a COMMITTED file
(`rust/crates/lakehousectl/tests/fixtures/shipped-tool.sh`), SYMLINKED into the install's own
`tools/` directory. The file the kernel executes is then a file nobody writes, and it has no
reason to answer `ETXTBSY`. What the fake tool does is read from `fake-tool-mode` in the
install's own directory — `backup`, `restore`, `silent` or `fail`, the four the tests need.

**The review's first suggestion was not used, and why.** Running the script with `sh <script>`
would break the shipped scripts: they are BASH (`#!/usr/bin/env bash`, `[[ ]]`, arrays,
`set -o pipefail`), and `sh` refuses them. The handoff's candidate — the committed fixture,
symlinked into the temporary directory — is used instead. The handoff's other two candidates (a
retry on `ETXTBSY` in `sys::output_or_stderr_with_env`, and a single-threaded run of the module)
are not used either: the first hides a real condition in the production code, and the second
leaves the flake in place for every other test that runs beside it.

**Commands run, in the foreground, on the final commit `fe77e06c`:**

| Command | Result |
| --- | --- |
| `/home/shiro/rantai/bin/cargo-locked fmt --check` | clean, no diff, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked clippy -p lakehousectl --all-targets -- -D warnings` | clean, no warning and no error, exit 0 |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl` | **170 passed, 0 failed** (165 at the review commit `270293cf`, 5 new) |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl`, 50 times in a loop | **50 passed, 0 failed** |
| `/home/shiro/rantai/bin/cargo-locked test -p lakehousectl backup`, 50 times in a loop | **50 passed, 0 failed** |

The flake, measured on the code BEFORE the fix: `cargo test -p lakehousectl`, 50 times in a loop:
**49 passed, 1 failed** — run 31, `the_restore_runs_the_shipped_script_with_the_backup_directory`,
`Failed("the backup tool failed: Text file busy (os error 26)")`. `cargo test -p lakehousectl
backup`, 50 times in a loop, before the fix: 50 passed, 0 failed — the filter runs fewer tests
beside it, so the race has fewer chances.

Every cargo command went through `/home/shiro/rantai/bin/cargo-locked` (the shared
`CARGO_TARGET_DIR`, `/home/shiro/rantai/cargo-target`; no `cargo clean`), in the foreground.

**One thing the reviewer may want to look at: the profiles.** The review's premise is that step
6's `up -d` creates the Dagster services ("The Dagster services read `MAINTENANCE_PAUSED` when
they are created. Step 6's `up -d` creates them with the pause set"). In this repository's
Compose file the Dagster services carry `profiles: ["dagster"]`, and the release keeps that
profile (`ops/release/render_compose.py`: "the profiles (`dagster`, `trino`, `seaweedfs`,
`backups`) stay as they are, so an operator still chooses what to start"). `docker::compose_up`
does not pass `--profile`; only `compose_stop`, `compose_stop_all` and `compose_services` do, and
they pass `--profile '*'`.

So the tool's `up -d` starts the DEFAULT profile only, and a Dagster service of a deployment that
runs the `dagster` profile is not recreated by it. The fix is the review's — after the pause is
cleared, `up -d --pull never` runs again — and it does not change which services the tool starts.
Two things the reviewer may want to decide:

- whether the recreate (and the update's step 6, and the rollback's start) must name the `dagster`
  profile, so the services that read the pause are the ones that are recreated;
- whether `up -d` without `--profile` is what a deployment should get at all: the update STOPS the
  Dagster services (`compose stop` with `--profile '*'`), and step 6 starts the default profile
  only, so the Dagster services stay stopped after an update.

This is read from the code and the Compose file; it was NOT run on a host (see below).

**Not verified, with the reason:**

- **No real update run on a host.** There is no install of a release on this machine, and no
  release is published yet: the update's own path (the writers stopped, the backup taken, the
  files swapped, the stack waited for, the rollback) is covered by the unit tests with the fake
  runner. The proof is the `update-proof` job of the dry run, and the reviewer runs it.
- **The whole-workspace cargo run** (`cargo clippy --workspace --all-targets --all-features -- -D
  warnings` and `cargo test --workspace`): not run here — this slice's machine rules say CI runs
  the workspace and the developer runs the scoped commands. The crate the slice touches
  (`lakehousectl`) was clipped and tested with `--all-targets`.
- **The Dagster code location** (`(cd dagster && python -m pytest dispar_orchestrate -q)`): not
  run — the branch changes no Python under `dagster/` (`git diff origin/main...HEAD -- dagster/`
  is empty). The two repo lints that scan it ran on this branch before these two commits, and
  both are OK.
- **No bun command ran**: the branch changes no TypeScript (`git diff origin/main...HEAD -- src/`
  is empty).
- **`docker compose --profile '*' config --quiet`**: not run — the branch changes no compose file
  (`git diff origin/main...HEAD -- docker-compose.yml` is empty).
- **The branch has not merged `main`**: its base is `c239e3b0`, and `main` has moved on since
  (`7a49fcbb`). The reviewer decides when to merge it; the checks above were run on the branch as
  it is.
- **The pushes**: both commits were pushed to `origin/feat/rel-2-s4b-lakehousectl-update`. No push
  was refused, so no `git pull --no-rebase` and no resolve was needed.

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

### Fix for review round 5 (developer, 2026-11-01)

**Commit** `15e5b88f` (branch `feat/rel-2-s2-release-pipeline`), one commit for the finding and
the two notes that came with it.

**BLOCKER 1 (`15e5b88f`): the SBOM is CycloneDX JSON.** All eight SBOM steps — the four images
of each of the two jobs — now ask for `format: cyclonedx-json` instead of `format: cyclonedx`.
`anchore/sbom-action` passes the format straight to syft (`syft scan … -o <format>`), and syft's
`cyclonedx` name is an ALIAS of `cyclonedx-xml`: its own encoder lists `cyclonedx`, `cyclone` and
`cdx` among the XML encoder's aliases, while the JSON encoder's name is `cyclonedx-json`. So
the pipeline wrote CycloneDX **XML** into `sbom/<name>.cdx.json`, and `cosign attest --type
cyclonedx` — which reads a JSON predicate — answered `invalid character '<' looking for beginning
of value`. With `cyclonedx-json` the file is JSON, the `.cdx.json` names are true, and the
predicate cosign reads is the one syft wrote.

The reason is stated at the site, in both jobs, above the SBOM steps: a comment says that the
format is CycloneDX JSON, that syft writes the plain `cyclonedx` name as XML, and that the XML
predicate failed with cosign's own words (REL-2 S2 review round 5 BLOCKER 1).

**The check** (`ops/release/test_release_workflow.py`, 2 new cases). `sbom_steps` finds every
`sbom-action` step of every job; `sbom_formats` names every one whose `format:` is not
`cyclonedx-json`. The new cases hold both jobs to it — and to having SBOM steps at all, so the
check cannot pass by the steps going away — and report a synthetic workflow that asks for the
XML format. The synthetic case was run against the real workflow with the format reverted: the
check then named all eight steps and the suite went red, so the check can fail.

**The note in `docs/CI.md`**: cosign v3 prints the `--tlog-upload has been deprecated` warning.
The line says the flag still works, that the pipeline keeps it, and that the switch to a
`--signing-config` file is left for when the flag is removed (REL-2 S2 review round 5).

**Commands run, in the foreground, on `15e5b88f`:**

| Command | Result |
| --- | --- |
| `python3 -m pytest ops/release -q` | **82 passed, 0 failed** (80 before this round, 2 new: the check on both jobs and the synthetic XML workflow) |
| `actionlint .github/workflows/release.yml` | clean, exit 0 (actionlint v1.7.7, with shellcheck 0.10.0 on the PATH, so the `run:` blocks were checked too) |

The pytest run used the virtualenv's interpreter (`/tmp/opencode/relvenv/bin/python -m pytest
ops/release -q`): the system Python is externally managed (PEP 668), so `pip install --user
pytest` is refused, and the virtualenv carries `pytest==8.3.4` over the system site packages
(PyYAML, cryptography). `actionlint` (v1.7.7) and `shellcheck` (v0.10.0) were already
downloaded to `/tmp/opencode` by the earlier rounds of this slice; `actionlint` was run from
`/tmp/opencode/actionlint`, with `/tmp/opencode/bin` on the PATH so it found that `shellcheck`.

The repository's own Python guards ran too: `python3 ops/lint/check_intra_package_imports.py` →
OK, `python3 ops/lint/check_bare_iceberg_count.py` → no violation.

The same two commands were run again, in the foreground, on the final commit of this section (this
file and the fix only): **82 passed, 0 failed**, and `actionlint` clean, exit 0.

**Not verified, with the reason:**

- **The dry run is not verified.** It needs a GitHub runner, and the PR's own dry run is the
  proof: the format is a runner behaviour, and the only evidence here is syft's own encoder
  names (read from `anchore/syft` at `v1.42.3`, the version `anchore/sbom-action` v0.24.0
  installs) and cosign's error message from the PR's second run.
- **The real release is not verified, and cannot be.** It needs a `v*` tag, the owner's approval
  on the `release` environment, and the release key. REL-2 S5 does that.
- No cargo command and no bun command ran: this round changes no Rust, no TypeScript, no code
  location and no Compose file.

### Review S2 round 6 (planner, 2026-11-01): the dry run is green, at `0b89767`

The PR's dry run passed end to end:
- four images built, pushed by digest and signed;
- the CycloneDX JSON SBOMs attested;
- the manifest signed, and "Verified OK" offline;
- the bundle built, **401 MiB** compressed (1.61 GiB unpacked).

**BLOCKER 1 (round 6): the offline bundle holds only our four images.**
- `make_manifest.py` writes `images` from the published images alone (`release-images.json`).
  `make_bundle.sh` saves only what `images` names.
- Every third-party image the rendered Compose file runs is missing from both: Postgres,
  ClickHouse, Lakekeeper, OpenFGA, RustFS, Gotenberg, the init images, and the `trino`,
  `seaweedfs` and `backups` profiles.
- An air-gapped host could not start the stack. Section 0 of this plan says the manifest holds
  "every image's digest", so this misses the plan.
- **Fix:**
  - `images` in `release.json` maps **every service of the rendered Compose file** to its
    `ref@sha256:…`. Ours come from the published digests, and the third-party ones from the
    rendered file, which is already pinned by digest (the S1 lint).
  - `make_bundle.sh` saves every **unique** reference. That takes a `docker pull` by digest first
    for the third-party ones; ours are already local or are pulled.
  - **SBOMs:**
    - every image in the bundle carries one: run `syft <ref> -o cyclonedx-json` for each
      third-party image, in a loop step pinned to the same syft version the sbom-action uses;
    - only our four images are signed and attested, because we cannot sign other people's
      images in their registries. Their integrity comes from the digest in the signed manifest.
  - Keep the "every image has an SBOM" check.
  - **Runner disk:** the dry run must free disk first, as `g9-upload-large.yml` does. Print
    `df -h` before and after the bundle.
  - Tests in `test_make_manifest.py`: the manifest names every service of a rendered fixture,
    and a third-party image without a digest is refused.
- The bundle will be several GiB. Print its size in the summary, as now. The release notes report
  it (spec: "measured, never assumed").

**Open: BLOCKER 1 (round 6).**

### Fix for review round 6 (developer, 2026-11-01)

**Commit** `525c73c6` (branch `feat/rel-2-s2-release-pipeline`), one commit for the finding.

**BLOCKER 1 (`525c73c6`): the offline bundle holds every image the release runs.**

**The manifest.** `release.json`'s `images` maps EVERY service of the rendered Compose file to
its `ref@sha256:…`: ours from the published digests the pipeline read back from the registry,
the third-party ones from the rendered file, which the S1 lint already pinned by digest. A
third-party image without a digest is refused, and so is a published one without a digest. The
input flag is now `--published-images` (it was `--images`): the flag said "the images", and
the flag now says "the published ones" — the third-party ones come from the Compose file.

On this repository's own template the rendered file has **35 services and 17 unique images**,
of which **13 are third-party**. The manifest names all 35 services; the bundle holds 17 tars
and 17 SBOMs. Measured by running `render_compose.py` and `make_manifest.py` on the real
template (a throwaway published map, since the digests of a real release do not exist yet):
`make_manifest: wrote … for version 0.2.0 (35 image(s), minFrom 0.0.0)`, and the 13
third-party references the pipeline pulls are exactly the 13 the rendered file names.

**One owner for the rule.** `ops/release/compose_images.py` (new) answers "which image does
this service run, and is it ours" for the manifest builder AND for the release pipeline, so
the two cannot drift apart (rule 4). Its parser is the repository's own lint's parser
(`ops/lint/check_image_digests.py`), so the release and the S1 digest lint cannot disagree
about what an image is. `ops/release/image_name.sh` (new) is the one owner of the name a
bundle file carries (the repository name without the tag): `make_bundle.sh` and the
pipeline's SBOM loop ask the same file, so the bundle can never look for an SBOM nobody
wrote.

**The bundle.** `make_bundle.sh` saves every UNIQUE reference the manifest names, and pulls
the ones that are not in the local store yet, by their digest reference. The "every image
has an SBOM" check stays. Two references that would land on the same file name are REFUSED
rather than letting one overwrite the other's tar: `image_name.sh` drops the registry and
the tag, so `postgres:16.15@sha256:…` and `example.com/postgres:16.15@sha256:…` share a
name. On this repository's own images the 17 names are all different (checked with
`image_name.sh` over the rendered file's 17 references).

**The pipeline, both jobs.** Each job now:

- frees the runner's disk first, the way `g9-upload-large.yml` does (`df -h /`, the same
  `rm -rf` of the toolchains this job never uses, `docker image prune --all --force`,
  `df -h /`), and prints `df -h /` before and after the bundle;
- pulls the third-party images of the rendered file by digest (`ops/release/compose_images.py
  --compose docker-compose.release.yml --published-images images.json`, then `docker pull` in
  a loop);
- writes a CycloneDX **JSON** SBOM for each of them, with the syft the SBOM action installs:
  the action's own `download-syft` sub-action, at the SAME commit
  (`anchore/sbom-action/download-syft@e22c3899…`, v0.24.0), whose `cmd` output is the syft
  binary. The version is the same one the action installs — `src/SyftVersion.ts` at that
  commit is `v1.42.3`, and `src/github/SyftGithubAction.ts` reads it for both paths — so the
  third-party SBOMs and ours come from one scanner;
- reports the bundle's measured size in the summary (`images: 35 in the manifest, 4 of them
  ours, each signed and attested`) and in the release notes.

Only our four images are signed and attested: we cannot sign another project's image in its
own registry. The third-party images' integrity comes from the digest in the SIGNED
manifest, and the bundle holds the image that digest names.

**One deviation, stated.** The job's `timeout-minutes` goes from **90 to 150** in both jobs.
The bundle now holds every image the release runs, so the job pulls thirteen third-party
images, describes each of them, saves each of them and compresses the lot; 90 minutes was
the budget for four images and a 401 MiB bundle. The reason is written at the site.

**The check on the pin pattern.** `ops/release/test_release_workflow.py`'s pin pattern now
allows a SUB-ACTION (`owner/repo/path@<sha>`): the syft install is the SBOM action's own
sub-action at the same commit, and a pattern that could not name it would have reported the
pipeline's own syft install as unpinned. The action is still pinned to a commit SHA, and the
release in the trailing comment is still named.

**Commands run, in the foreground, on `525c73c6`:**

| Command | Result |
| --- | --- |
| `python3 -m pytest ops/release -q` | **107 passed, 0 failed** (82 before this round, 25 new: 12 in the new `test_compose_images.py`, 2 in `test_make_manifest.py`, 3 in `test_make_bundle.py`, 8 in `test_release_workflow.py`) |
| `actionlint .github/workflows/release.yml` | clean, exit 0 (actionlint v1.7.7, with shellcheck 0.10.0 on the PATH, so the `run:` blocks were checked too) |
| `bash -n ops/release/make_bundle.sh ops/release/image_name.sh` | clean, exit 0 |
| `shellcheck ops/release/make_bundle.sh ops/release/image_name.sh` | clean, exit 0 |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | no violation |
| `python3 ops/lint/check_image_digests.py` | 46 third-party image(s) pinned by digest, 8 locally built image(s) exempt |

The pytest run used the virtualenv's interpreter (`/tmp/opencode/relvenv/bin/python -m pytest
ops/release -q`): the system Python is externally managed (PEP 668), and the virtualenv
carries `pytest==8.3.4` over the system site packages (PyYAML, cryptography). `actionlint`
(v1.7.7) and `shellcheck` (v0.10.0) were already downloaded to `/tmp/opencode` by the
earlier rounds of this slice.

**The bundle script, exercised against a real Docker daemon.** With ONE small third-party
image — the pinned `alpine:3.20` of `docker-compose.yml` — and a tiny local image built
`FROM scratch`:

- the alpine image was removed from the local store first, so the script had to PULL it: the
  log shows `make_bundle: pulling alpine:3.20@sha256:d9e853e8…` and then
  `make_bundle: saving alpine`;
- the bundle held `images/alpine.tar`, `images/local-tiny.tar`, `sbom/alpine.cdx.json`,
  `sbom/local-tiny.cdx.json`, `docker-compose.yml`, `release.json`,
  `release.json.sigstore.json` and `SHA256SUMS`, and `sha256sum -c SHA256SUMS` answered `OK`
  for all seven files;
- `docker load -i images/alpine.tar` answered `Loaded image ID: sha256:d9e853e8…`, and the
  digest still resolved afterwards. The saved tar's `manifest.json` carries `"RepoTags":
  null` — the image is found by its DIGEST after the load, not by its tag. That is the same
  as for our four images before this round, and it is a note for REL-2 S3 (`lakehousectl`
  loads by digest), not a change here;
- the SBOMs were written by the same syft the pipeline installs (v1.42.3, downloaded to
  `/tmp/opencode/syftbin/syft` and checksum-verified) with the exact command the workflow
  runs: `syft scan "docker:alpine:3.20@sha256:d9e853e8…" -o "cyclonedx-json=…"` — it wrote
  a CycloneDX JSON document (`"bomFormat":"CycloneDX"`).

The test's images and its working directory were removed afterwards; the machine's own image
store is as it was.

**The syft the pipeline installs, checked at the pinned commit.** `download-syft/action.yml`
at `e22c3899…` exposes the input `run` (default `download-syft`) and the output `cmd`;
`src/index.ts` routes `run: download-syft` to `getSyftCommand()` and sets that output; and
`src/github/SyftGithubAction.ts` reads `SYFT_VERSION = core.getInput("syft-version") ||
VERSION` for both paths, with `src/SyftVersion.ts` = `v1.42.3`.

**Not verified, with the reason:**

- **The dry run is not verified.** It needs a GitHub runner, and the PR's own dry run is the
  proof: pulling thirteen third-party images, the size of the bundle and the runner's free
  disk are runner facts. What was checked here is the wiring, the syft version at the pinned
  commit, and the bundle script against a real daemon with one real image.
- **The bundle's real size is not verified.** It will be several GiB (the review says so);
  the summary and the release notes print the measured size, and the release notes now carry
  it, so the number the notes quote is the number the job measured. The 3 MiB measured here
  is the two-image test bundle, not a release.
- **The real release is not verified, and cannot be.** It needs a `v*` tag, the owner's
  approval on the `release` environment, and the release key. REL-2 S5 does that.
- **The runner's free disk is not measured.** `df -h` is printed before and after the
  bundle in both jobs, so the next dry run's log answers it.
- No cargo command, no bun command and no Dagster command ran: this round changes no Rust,
  no TypeScript, no code location and no Compose file.

### Review S2 round 7 (planner, 2026-11-01): the dry run with every image is green

- **Run 41266974262, all 41 checks green.** The dry run built and signed our four images and
  pulled every third-party image by digest. Every image got a CycloneDX JSON SBOM. The manifest
  was signed and "Verified OK" offline.
- **The bundle holds 17 images:** socat, alpine, aws-cli, seaweedfs, clickhouse-server, curl,
  debezium-server-iceberg, gotenberg, our api, console, dagster and oidc-mock, openfga,
  postgres, the Lakekeeper catalog, rustfs and trino.
- **Measured size: 3,118,474,714 bytes (2,974 MiB) compressed.** The spec says "measured, never
  assumed". This is the number for 0.1.0's images, and each release prints its own.
- **Runner disk:** 86 GB free before, and 91 GB after the bundle step, once the build cache was
  freed. That leaves room to spare.
- **BLOCKER 1 (round 6) is fixed.** The fix commits are `525c73c6` and `f9d0290b`.

**Open: no BLOCKER, no SHOULD-FIX.** The real release (S5) needs a `v*` tag and the owner's
approval in the `release` environment.

### Review S3 round 1 (planner, 2026-11-01, at `04dbae2f`)

Read: the whole `lakehousectl` crate, the `release.yml` diff, `install.sh`, `make_bundle.sh` and
`make_manifest.py`. The verification of the manifest (`verify.rs`) is right: the digest is
checked before the signature, the key is the compiled-in one unless the `test-key` feature is
on, and a release build says when it ignores `LAKEHOUSECTL_PUBLIC_KEY_FILE`. The `p256` 0.14 /
`ecdsa` 0.17 pin and its reason check out (`RUSTSEC-2026-0387` is in the advisory database, and
the lock has `crypto-bigint` 0.7.10). The finding on the two Docker image stores is good work.

- **BLOCKER 1. The online install downloads into a predictable shared directory.**
  `install.rs` `obtain()` writes to `std::env::temp_dir().join("lakehousectl-release-<version>")`
  with `create_dir_all` and `curl --output`. The installer runs as root. Any local user can
  create that directory first and put a symlink at `release.json`, and root's `curl` then writes
  through it into any file on the host. A directory left by an earlier run is also reused.
  *Fix:* read the three assets into memory (curl to stdout; they are small text files), or
  create a fresh private directory (`nix::unistd::mkdtemp`, mode 0700) and remove it at the
  end. Add a test that the download path is not a fixed name.
- **SHOULD-FIX 2. `SHA256SUMS` is checked for the name only.** `checksums_cover()` asks whether
  a line names the tar, and never hashes the tar. The plan says "check `SHA256SUMS`". *Fix:*
  hash each `images/<name>.tar` and compare it with its `SHA256SUMS` line before
  `docker load`, so a damaged copy fails with a clear message. Say in the comment that
  `SHA256SUMS` is not signed, so the image-id check against the signed manifest stays the
  trust check, and this one catches damage.
- **SHOULD-FIX 3. A second install overwrites the state of the first.** `write_layout()`
  always rewrites `state.json` (`previousVersion: null`), `docker-compose.yml` and
  `release.json`. *Fix:* when `--dir` already holds a `state.json`, install the same version
  again only (keep `state.json` as it is), and refuse any other version, pointing to
  `lakehousectl update` (S4). Test both.
- **SHOULD-FIX 4. With an existing `.env`, the email shown is the template's.**
  `secrets::write_env()` reads `AUTH_BOOTSTRAP_EMAIL` from `ENV_EXAMPLE` when the `.env`
  exists. *Fix:* read it from the existing file. Test it.
- **SHOULD-FIX 5. `install.sh` leaves the binary behind.** `exec` replaces the shell, so the
  `EXIT` trap never removes `$work`. *Fix:* run `"$work/$BINARY" install "$@"` without
  `exec`, and exit with its status.

**Proven by the PR's dry run, not by reading:** after `docker load` of a tar that
`docker save` wrote from a digest reference, both `docker image inspect <ref@digest>` and
`docker compose up` must find the image with no registry. The `install-proof` job is that test.
It must be green before merge.

**Open: BLOCKER 1, SHOULD-FIX 2–5.**

### Review S3 round 2 (planner, 2026-11-01, at `55173176`)

- **BLOCKER 1 is fixed** (`a5f49eca`). The three assets are read into memory with `curl` to
  stdout, and nothing is written under the temporary directory. `obtain_with` takes the fetch as
  a parameter, so the test proves it without a network.
- **SHOULD-FIX 2 is fixed** (`f34f19e2`). Each tar is hashed in 64 KiB reads and compared with
  its `SHA256SUMS` line before `docker load`. A missing line and a damaged file are both refused
  and tested, and the comment says what this check is for.
- **SHOULD-FIX 3 is fixed** (`5757949d`). The same version installs again and keeps
  `state.json`. Another version is refused and pointed at `lakehousectl update`. A `state.json`
  that names no version is refused rather than overwritten.
- **SHOULD-FIX 4 is fixed** (`bf0f7c43`). **SHOULD-FIX 5 is fixed** (`38ccc6f4`), and its test
  fails against the old script.

**Open: no BLOCKER, no SHOULD-FIX.** Merge waits for the PR's CI on this head: the required
checks, the workspace cargo run, `Release · dry run`, and the `install-proof` job that installs
the bundle on a fresh runner.

### Review S3 round 3 (planner, 2026-11-01): the PR's install proof, at `9339179`

The dry run is green, and `install-proof` now runs the installer: it unpacks the bundle,
verifies the manifest against the dry run's key, logs the skipped pre-flight checks and loads
the first image. Then it stops:

```
lakehousectl: loading bundle/images/socat.tar
lakehousectl: the image alpine/socat:1.8.1.3@sha256:466c8a03… is not in the local store
```

- **BLOCKER 1 (planner's miss in 4b). After `docker load`, nothing resolves a digest-pinned
  reference.** `docker save` of a pulled image does not carry its registry digest on Docker's
  classic image store. After `docker load` the host has the image, but not under
  `name:tag@sha256:<registry digest>`. Both the installer's check (`docker image inspect
  <ref@digest>`) and the release's Compose file pin images that way. So an air-gapped install
  cannot start even when the right image is loaded. Section 4b's T0 gave the installer a way to
  CHECK a loaded image (`imageIds`), but no way for Compose to FIND it. Round 1 named this as
  the one thing only the dry run could prove, and the dry run has disproved it.
- **Next.** The fix is a design choice, so it waits for a measurement on bluefin. On both image
  stores, it covers which `docker save` form, if any, lets a digest-pinned reference resolve
  offline after `docker load`, for `docker image inspect` and for `docker compose up --pull
  never`. The planner then writes the decision here (S3 D-amend) before any code changes.

**Open: BLOCKER 1.**

### S3 D-amend (planner, 2026-11-01): how an air-gapped host finds a loaded image

The measurement is on bluefin: Docker 29.9.2, with each image store tried. The containerd
store is the host's own, and the classic store was run in `docker:29-dind` with
`containerd-snapshotter=false`. The image was `alpine/socat:1.8.1.3@sha256:466c8a03…`.

- **Classic store:** no `docker load` of any archive form registers a repo digest. This
  includes an archive whose `manifest.json` carries `RepoDigests`. So a digest-pinned reference
  never resolves offline there. With a network, `docker compose up -d` silently pulls the image
  from the registry instead, which would hide the bug on a CI runner.
- **Containerd store:**
  - An archive saved from the TAG (`docker save name:tag`) carries `io.containerd.image.name` in
    `index.json`. After `docker load`, `name:tag@sha256:<list digest>` resolves, and
    `docker compose up --pull never` starts from it with no network. The image's `Id` is then
    the list digest the manifest pins.
  - An archive saved from the digest reference (what `make_bundle.sh` does today) loads as a
    dangling `<none>` image.
  - `docker save --platform` rewrites the archive's digest to the per-platform manifest, so it
    must never be used.

**Decision D13 (planner):**
1. **An air-gapped install requires Docker's containerd image store.** A new pre-flight check,
   `image-store`, runs only for `--bundle`. It refuses a classic-store host and says how to
   switch: `"features": {"containerd-snapshotter": true}` in `/etc/docker/daemon.json`, then a
   restart of Docker. It also says that images held by the classic store are hidden while the
   containerd store is on. An online install works on either store, because `docker pull` by
   digest registers the digest.
2. **The bundle is made on the containerd store, from tags.** Every job that builds a bundle
   (the dry run and the release) turns the containerd store on before its first pull or build,
   and asserts it is on. `make_bundle.sh` refuses to run on another store. For each image it
   pulls by digest, tags `name:tag`, and saves the TAG, never with `--platform`.
3. **Compose never pulls.** `lakehousectl` starts the stack with `docker compose … up -d --pull
   never`. A missing image is then an error, never a silent fetch.
4. **The image check after `docker load` stays.** On the containerd store, the loaded `Id` is
   the registry digest the signed manifest pins (`install.rs`'s existing comparison).
5. **`install-proof` turns the containerd store on before it installs.** It also gains a step
   that cuts the runner off from the registries (no `docker pull` possible) before
   `lakehousectl install --bundle`, so the proof is air-gapped in fact, not just in name.
6. **Air-gapped install on the classic store is not supported, and says so.** The pre-flight
   check is the honest refusal. If a customer needs it, it becomes a backlog item: a
   tag-pinned Compose file with the digests verified by the installer.

**Open: BLOCKER 1, to be fixed per D13.**

### Review S3 round 4 (planner, 2026-11-01, at `02048fac`): D13

D13 is implemented as decided. On bluefin, with an actual air gap, `make_bundle.sh` saves
each image by its tag, the installer's `image-store` check passes, both images load and are
found by their pinned digest, and the stack starts with `--pull never`. The counterfactual (the
old save from the digest reference) reproduced the blocker, so the measurement proves the fix.

- **BLOCKER 1. The dry run's registry does not survive the store switch.** `dry-run` runs its
  registry as a `services:` container. GitHub starts service containers before the first step,
  and "Turn on the containerd image store" then restarts Docker and hides the classic store's
  images. The registry the image builds push to is then gone. The bluefin measurement ran no
  service container, so it could not see this. *Fix:* start the registry as a step after the
  switch, and test the order.
- **SHOULD-FIX 2. The errors promise output the tool hides.** `sys::output` keeps stderr to
  itself, so a failed `docker compose up`, `docker load` or `docker pull` reaches the operator
  without its reason. Yet `compose_up` says "The output above says why". The developer reported
  this; it predates D13. *Fix:* carry the last lines of stderr into the error, and word the
  message to match.

**Open: BLOCKER 1, SHOULD-FIX 2.**

### Review S3 round 5 (planner, 2026-11-01, at `d0a64a7b`)

- **BLOCKER 1 is fixed** (`d92af2e2`). The registry is a step after the store switch, pinned
  by digest, and waited for with a bounded loop that prints its logs on failure. The service
  container is gone, and the tests check the order in every job that restarts Docker.
- **SHOULD-FIX 2 is fixed** (`19ac61cf`). A failed docker call now carries the last 20 lines of
  its stderr into the error, and `compose_up` no longer promises output it does not print.

**Open: no BLOCKER, no SHOULD-FIX.** Merge waits for this head's CI: the required checks, the
workspace cargo run, `Release · dry run`, and `install-proof`, which is now air-gapped and on
the containerd store.

### Review S4a round 1 (planner, 2026-11-01, at `ba9115a3`)

S4-T1 to T3 match D14–D16. The real run on bluefin is the proof that matters: the shipped
`backups.xml` made `BACKUP … TO Disk('backups')` work, and a backup, a `down -v` and a restore
brought both markers back. D14's list was the planner's miss, since
`ops/seaweedfs/iam.template.json` (profile `seaweedfs`) is a third mount. The developer followed
D14's rule rather than its list, which is right, and the tooling test pins all three.

- **SHOULD-FIX 1. A shipped file's path is written as the manifest names it.**
  `files::write` joins `dir` with the manifest's key. The manifest is signed, but a key that is
  absolute or holds a `..` component would write outside `--dir`. *Fix:* `make_manifest.py` and
  `files.rs` both refuse a name that is absolute or has a `..` component, or anything but
  normal components, with tests.
- **SHOULD-FIX 2. Release assets are named by the file's last component**
  (`files::asset_name`). Two shipped files with the same file name in different folders would
  collide on GitHub Releases. *Fix:* `make_manifest.py` refuses two `files` entries with the
  same last component, with a test.
- **SHOULD-FIX 3. The developer path of the backup scripts changed.** Before, they ran
  `docker compose -p <project>` in the repository root, which also loads a developer's
  `docker-compose.override.yml`. They now always pass `-f docker-compose.yml`, which stops
  that. *Fix:* pass `-f` (and `--env-file`, `override.yml`) only when `LAKEHOUSE_DIR` is set,
  so the unset case is exactly the old command. Test both cases in `test_backup_scripts.py`.

**Open: SHOULD-FIX 1–3.**

### Review S4a round 2 (planner, 2026-11-01, at `cd4aba65`)

- **SHOULD-FIX 1 is fixed** (`e11abbf6`). The path rule is checked when the manifest is made
  and again before a file is fetched and before it is written, on both the Python and the Rust
  side.
- **SHOULD-FIX 2 is fixed** (`4b6ca96c`). Two `files` entries whose last component is the same
  are refused when the manifest is made.
- **SHOULD-FIX 3 is fixed** (`4e61a122`). With `LAKEHOUSE_DIR` unset, the scripts run exactly
  the old `docker compose -p <project>` in the repository root. The developer ran the new
  tests against the old scripts, and they failed there.

**Open: no BLOCKER, no SHOULD-FIX.** Merge waits for #332's CI, including `Release · dry run`
and `install-proof`, which now install the shipped files.

### Review S4b round 1 (planner, 2026-11-02, at `1f9095c0`)

Read: `update.rs`, `rollback.rs`, `runner.rs`, `pause.rs` and `status.rs`, plus the outline of
`self_update.rs` and the `update-proof` job. The developer's machine (bluefin) went
unreachable under load before the handoff was written. The commits are all pushed: `42fa7b6b`
(T4), `ac095fc2` (T5), `11a43698` (T6), `d625454a` (T7) and `1f9095c0` (a fix).

- **BLOCKER 1. A failure in step 4 or step 5 leaves the install stopped and paused.**
  `update::run` sets `MAINTENANCE_PAUSED` and stops the writers (step 4), then backs up
  (step 5). If `stop_writers` or `backup` fails, for example on a full disk, it returns the
  error with the writers still stopped and the pause still set. No file has been swapped yet, so
  the old release is intact. *Fix:* on a failure in step 4 or 5, start the stack again on the
  OLD files (`up -d --pull never`), wait for ready on the old version, clear the pause, and
  report that the update stopped before it changed anything. Add unit tests with the fake
  runner, one for a failed stop and one for a failed backup: the stack is up on the old
  version, the pause is cleared, and the error says so. D17 named only "from step 6 on", which
  was the planner's miss.
- **Missing: the handoff of S4b.** The fix's developer writes it: the commits, the counts on
  the final commit, and what was not verified.

**Open: BLOCKER 1.** Merge waits for the fix and for the PR's dry run, including
`update-proof`.

### Review S4b round 2 (planner, 2026-11-02, at `6feeeec7`)

- **BLOCKER 1 is fixed** (`db95e794`). A failure in step 4 or 5 starts the old files again,
  waits for ready on the old version, clears the pause, and says the update stopped before it
  changed anything. A failed restart is said plainly. The three tests failed on the code
  before the fix.
- **BLOCKER 2 (found by the developer's handoff). The pause outlives every update.** The
  Dagster services read `MAINTENANCE_PAUSED` when they are created. Step 6's `up -d` creates
  them with the pause set, and step 8 clears it in `.env` only. Nothing recreates them, so after
  a successful update, and after a rollback or the BLOCKER 1 recovery, the expiry and
  orphan-file jobs stay skipped until something else recreates those services. Storage then
  grows silently. *Fix:* after clearing the pause, run `up -d --pull never` again, which
  recreates only the services whose environment changed. Do it in every path that clears the
  pause: success, rollback, and the step 4/5 recovery. Test with the fake runner that the clear
  is followed by an `up`, in each path.
- **SHOULD-FIX 3. The `backup::tests` flake** (`ETXTBSY`, about 1 run in 20, older than this
  fix). A flaky test is a red CI waiting to happen. *Fix:* write the fake script, close it, then
  run it with `sh <script>` rather than executing the file, or use the candidate the handoff
  names. Then run the module 50 times in a loop and quote the count.

**Open: BLOCKER 2, SHOULD-FIX 3.**

### Review S4b round 3 (planner, 2026-11-02, at `0267f79e`)

- **BLOCKER 2 is fixed** (`e1a37beb`). `pause::clear_and_recreate` owns clearing the pause and
  recreating the services, and all three paths use it. Five tests read `.env` at every call. Nine
  tests failed with the fix reverted.
- **SHOULD-FIX 3 is fixed** (`fe77e06c`). The fake tool is a committed executable fixture,
  linked into the install, so the inode that runs is never written. 50 of 50 runs passed, against
  49 of 50 before. `sh <script>` was rightly not used, because the shipped scripts are bash.
- **BLOCKER 3 (found by the developer's handoff). A release install runs no Dagster.**
  - `docker::compose_up` passes no `--profile`, and nothing sets `COMPOSE_PROFILES`. So an
    installed stack starts only the default profile, while the 16 Dagster services carry
    `profiles: ["dagster"]`.
  - An installed Lakehouse therefore runs no pipelines: no upload loads, no connector runs, and
    no maintenance jobs.
  - The S3 install proof checks only `/health/ready` and the console, so it could not see this.
  - It is a gap in the S3 and S4 plans, not in the code those plans asked for.

  **Decision D22 (planner):**
  - A release install runs the `dagster` profile.
  - On a fresh install, `lakehousectl` writes `COMPOSE_PROFILES=dagster` into the install's
    `.env`, and `update` adds it when an older install's `.env` has no `COMPOSE_PROFILES`.
    Profiles an operator has set are kept as they are.
  - The operator adds `trino` or `seaweedfs` there.
  - `.env.example` is not changed, so a developer's default stays the same.

  *Fix:*
  - Implement it in `secrets.rs` and `update.rs` (or wherever the `.env` is written), with
    tests.
  - `install-proof` and `update-proof` assert that `dagster-daemon` and `dagster-code-location`
    are running after the install, after the update, and after the rollback.
  - `status` lists them.
  - Cite `REL-2 S4b review round 3 BLOCKER 3` and D22.

**Open: BLOCKER 3.**
