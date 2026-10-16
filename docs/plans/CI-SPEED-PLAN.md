# CI Speed Plan and Spec

Status: PR 1 merged (#88), PR 2 in review.
Baseline measured on `origin/main` at `715d87b`, 2026-10-08.

## 1. Goal

Cut the time from `git push` to a trustworthy CI verdict, without paid
runners and without running fewer tests.

| Scenario | Today (measured) | Target |
|---|---|---|
| Rust PR, all checks green | 16–22 min, 33 min worst case | 10–12 min after PR 3, 8–10 min after PR 5 |
| Frontend-only PR | 16–22 min | 2 min or less |
| Docs-only PR | 16–22 min | 1 min or less |
| Runner-minutes per push (CI + Docker + Coverage) | about 137 | 55 or less |

The targets are estimates. They come from the step timings in section 3
and from published benchmarks, and have not been tested on this repo.
Every PR in this plan must report its own measured numbers (section 8).

## 2. Constraints

These are fixed. Do not work around them.

1. **No paid runners.** Only `ubuntu-latest` and the other free
   GitHub-hosted runners. The repo is public, so these are free and have
   4 vCPUs.
2. **No PR job may run on the self-hosted runner** (`rep-staging-vm`,
   labels `self-hosted, staging`). This is a public repo and a fork PR
   could run arbitrary code on that machine.
3. **No publishing to GHCR.** `docker.yml` and `docs/CI.md` record that
   publishing images is a deliberate, separate decision. Images are
   shared between jobs as workflow artifacts instead.
4. **No test may stop running.** Every test that runs on a Rust PR today
   must still run on a Rust PR. Path filters may skip a job only when the
   diff cannot affect it.
5. **Fork PRs must work.** Fork PRs get a read-only token. Anything that
   needs a write token must not be on the PR path.
6. **Local development must not regress.** `docker compose up --build`,
   `scripts/compose.sh`, `cargo test` and the staging deploy must keep
   working and must not get slower.
7. **Leave `security.yml` alone.** Its `history-scan` job is red on
   purpose; `docs/CI.md` explains why.
8. **Follow the repo's conventions.** Workflows here carry a comment on
   each non-obvious choice explaining why it exists. New and changed jobs
   need the same. Commit messages use conventional commits, such as
   `ci: build the API image once per run`.

## 3. Baseline

### 3.1 Wall-clock per CI run

Five consecutive runs of the `CI` workflow: 15m49s, 21m59s, 17m48s,
17m18s, 32m41s. The slowest single job is about 17.5 minutes, so the two
longer runs spent time waiting for free runners. Each push starts about
26 jobs across four workflows.

### 3.2 Slow jobs (run 37757594281)

| Job | Total | Slow step | Step time |
|---|---|---|---|
| Rust · G6 ingest matrix | 944s | `up -d --build` | 681s |
| Rust · test (stable) | 818s | `cargo test --workspace --locked` | 774s |
| Rust · Gold export | 801s | `up -d --build lakehouse-api` | 725s |
| Rust · test (1.88.0) | 794s | same as stable | about 750s |
| Rust · G3 Bronze maintenance | 742s | `up -d --build` | 646s |
| Rust · G4 Debezium CDC | 675s | `up -d --build` | 532s |
| Rust · G3a Dagster + dlt | 617s | `up -d --build` | about 520s |
| Rust · G8 governance masking | 580s | `up -d --build lakehouse-api` | 520s |
| Docker workflow · build and smoke test | about 750s | Build image | 702s |
| Coverage workflow · `cargo llvm-cov` | about 750s | `cargo llvm-cov` | 699s |
| Frontend | 134s | install 9, lint 23, typecheck 13, test 43, build 37 | |
| Rust · clippy | 92s | | |
| Rust · build | 86s | | |

### 3.3 Causes

1. **The `lakehouse-api` image is built from scratch seven times per
   push.** Six acceptance jobs (G3a, G3, G4, G6, Gold, G8) and the Docker
   workflow each run a release build of about 515 crates. The
   `--mount=type=cache` mounts in `rust/Dockerfile` do not persist on
   GitHub runners, and the `cargo build` layer is invalidated by any
   source change. `sqlx-cli` is also compiled from source in each build.
   The Dagster image is rebuilt in four of those jobs too.
2. **The test job is serial.** Of its 774s, 259s is compiling 14
   workspace crates and linking 79 test binaries (dependencies are
   restored from cache). The rest is running those binaries one after
   another; their own reported durations sum to 498s.
3. **`lakehouse-api` unit tests run twice.** `src/main.rs` declares the
   same 30 modules with `mod` that `src/lib.rs` declares, so the tests
   compile into both targets: `lib.rs` 95s, `main.rs` 98s.
4. **The suite runs three times per PR.** Once on stable, once on the
   1.88.0 MSRV toolchain, and once more under `cargo llvm-cov`.
5. **Nothing is skipped by path.** A docs-only or frontend-only PR runs
   every Rust and acceptance job.

### 3.4 Facts the implementer needs

- `main` has no branch protection and no rulesets. No check is currently
  "required" by GitHub.
- `lakehouse-test-support` starts one labelled Postgres container with
  `ReuseDirective::Always` from a `#[ctor]`, and every test process
  reuses it. `#[sqlx::test]` gives each test its own database.
- The `lakehouse-api` service in `docker-compose.yml` has a `build:`
  block and no `image:` key. Compose therefore names the image after the
  project, and each CI job uses a different project name (`g4ci`,
  `goldci`, and so on).
- The API image takes a second build context, `pipeline_src`, from
  `dagster/dispar_orchestrate`. A change under that directory changes
  the API image.
- CI calls plain `docker compose`, so `GIT_SHA` is unset and both images
  are built with `GIT_SHA=unknown`. Keep it unset in CI: `ARG GIT_SHA`
  sits at the top of `dagster/Dockerfile`, so setting it would
  invalidate every layer.
- The Rust test job installs Bun because `lakehouse-bi`'s
  `specs_match_typescript` test reads `src/lib/dashboard-specs.ts`. A
  change to that file must count as a Rust change.
- The repo is busy: about 130 commits landed on `main` in the week
  before this plan. Keep PRs small and rebase often.

## 4. Design

### 4.1 Change-scope detection and a single gate job

Add a first job, `changes`, that classifies the diff and exposes boolean
outputs. Every other job runs or skips on those outputs. Add a last job,
`ci-required`, that runs with `if: always()`, needs every other job, and
fails unless each job either succeeded or was skipped for a reason the
scope outputs justify.

Do not use workflow-level `paths:` filters. A filtered workflow reports
nothing, which leaves a required check pending forever.

`RantAI-dev/RantAIClaw` already has this pattern working. Reuse its
shape: `scripts/ci/detect_change_scope.sh`, `scripts/ci/required_gate.sh`,
and a self-test script for each that runs before the script is trusted.

Scope outputs and what sets them:

| Output | Set by changes under |
|---|---|
| `rust` | `rust/**`, `src/lib/dashboard-specs.ts`, `docker-compose.yml` |
| `frontend` | `src/**`, `public/**`, `package.json`, `bun.lock`, `bunfig.toml`, `tsconfig.json`, `next.config.ts`, `eslint.config.mjs`, `postcss.config.mjs`, `components.json`, `Dockerfile.frontend` |
| `dagster` | `dagster/**` |
| `stack` | `docker-compose.yml`, `ops/**` (except `ops/fixtures/**`), `scripts/**` (except `scripts/ci/**`), `.env.example` |
| `docs_only` | true when every changed file is under `docs/**`, `GTM/**`, or is a root `*.md`, `LICENSE` or `NOTICE` |
| `all` | `.github/**`, `ops/fixtures/**`, `scripts/ci/**`, an unreadable base SHA, or any file the rules above do not classify |

`all` forces every job to run. An unclassified file must set `all`; the
script fails safe, never open.

Which outputs each job group runs on:

| Job group | Runs when |
|---|---|
| Repo lints (parity corpus, R11, compose init, Dagster imports) | always |
| Frontend | `frontend` or `all` |
| Rust fmt, clippy, build, test, MSRV | `rust` or `all` |
| Dagster unit tests | `dagster` or `all` |
| Image build, all acceptance jobs, image smoke test | `rust`, `dagster`, `stack` or `all` |

On `push` to `main`, run everything regardless of scope.

### 4.2 Build each image once per run

Add an `images` job that builds the `lakehouse-api` image and the Dagster
image once, and hands them to the jobs that need them as artifacts.

- Build with `docker/build-push-action` or `docker buildx bake`, with
  `cache-from` and `cache-to` set to `type=gha,mode=max` and a separate
  `scope` per image.
- Export each image with `outputs: type=docker,dest=...`, compress it,
  and upload it with `actions/upload-artifact` at one-day retention.
- Each acceptance job declares `needs: [changes, images]`, downloads the
  artifacts, runs `docker load`, then starts the stack without building.
- Give the `lakehouse-api` service and the Dagster-built services an
  `image:` key that reads an environment variable with a local default,
  for example `image: ${LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}`.
  Keep the `build:` blocks, so local `up --build` behaves as before.
- Replace `up -d --build` with `up -d --no-build` in the acceptance
  jobs. A missing image must fail the job, not trigger a silent rebuild.
- Move the `/health` smoke test from `docker.yml` into `ci.yml` as a job
  that consumes the same artifact. Leave `docker.yml` with its tag
  trigger only.

Restructure `rust/Dockerfile` so the dependency build is a cacheable
layer:

- Use `cargo-chef` (`prepare`, then `cook --release`) so the 500
  dependency crates are compiled in a layer that changes only when
  `Cargo.lock` or a `Cargo.toml` changes.
- Install `cargo-chef` and `sqlx-cli` from prebuilt binaries, or keep
  their install layer above every `COPY` of repo files, so they are not
  recompiled.
- Keep the existing explanatory comments that still apply, and keep the
  final runtime stage byte-for-byte equivalent in behaviour: same
  binary path, `sqlx` binary, migrations, `specs`, `pipeline_src` copy,
  `GIT_SHA` handling and entrypoint.

The repo's GitHub Actions cache is capped at 10 GB and `rust-cache`
already uses several gigabytes. If the image cache is evicted often
(visible as a cold `images` job on `main`), reduce what `rust-cache`
stores before considering anything else.

### 4.3 Faster Rust test job

Do these in order, measuring after each.

1. **Stop running `lakehouse-api` unit tests twice.** First confirm that
   the test names in the `main.rs` binary are a subset of those in the
   `lib.rs` one. If they are, set `test = false` on the `[[bin]]` target.
   If the binary has tests of its own, keep those and say so in the PR.
2. **Run tests with `cargo-nextest`.** Install it with
   `taiki-e/install-action`. Run `cargo nextest run --workspace --locked`
   and then `cargo test --workspace --doc --locked`, because nextest does
   not run doc tests.
3. **Start Postgres before the tests.** nextest starts many test
   processes at once, and they could race to create the shared
   container. Start it in a step before the test run, using the same
   image, tag and label that `lakehouse-test-support` looks for, so every
   process finds it already running.
4. **Trim debug info in CI only.** Set
   `CARGO_PROFILE_DEV_DEBUG=line-tables-only` in the job's `env`. Do not
   change the profiles in `Cargo.toml`.
5. **Turn the MSRV job into a compile check.** Replace the 1.88.0 matrix
   entry with a job that runs
   `cargo check --workspace --all-targets --locked` on 1.88.0.

Fallback if nextest is unreliable here (RantAIClaw abandoned it after
runs were killed or hit a 30-minute cap): keep `cargo test`, and split it
into two or three jobs by crate, with `lakehouse-api` on its own.

### 4.4 Remove duplicate work

- Run `coverage.yml`'s `cargo llvm-cov` job on `push` to `main` and on
  `workflow_dispatch` only. Nothing reads its output on PRs; it uploads
  an artifact and no check consumes it.
- Fold the four tiny lint jobs (parity corpus, R11, compose init
  readiness, Dagster intra-package imports) into one job with one step
  each. Each takes 5–8 seconds and occupies a runner slot.

### 4.5 Frontend job

- Run lint and typecheck concurrently with the build, inside the same
  job. Do not add job slots.
- Cache `.next/cache` between runs, keyed on `bun.lock` and the source
  hash with a restore-key fallback.
- `bun check`: add it only if a stable Bun release ships it, and only as
  a non-blocking step beside `tsc --noEmit`. It was announced for Bun
  1.4.3 and was canary-only when this plan was written. Do not move CI
  to a canary Bun. The expected saving is about 10 seconds, so this is
  low priority.

### 4.6 Optional follow-ups

Do these only after PR 1–4 are merged and measured.

- **Start the stack while the image builds.** Let acceptance jobs pull
  and start their third-party containers before `images` finishes, then
  add the API. Expected saving is 1–2 minutes on the critical path.
- **Merge `lakehouse-api`'s 33 integration test files into one binary.**
  This cuts link time and disk use. It moves many files in an active
  area, so coordinate it and land it as its own PR.

## 5. Out of scope, needs an owner decision

Do not implement these. Record them in the PR description as open items.

| Item | Why it is deferred |
|---|---|
| Merge queue with a fast PR tier | Needs branch protection, which `main` does not have. It also moves acceptance failures from the PR to merge time. |
| Staging deploy reusing the CI-built image | Needs images published to a registry, which constraint 3 forbids. |
| Staging deploy waiting for CI | `deploy-staging.yml` starts on push without waiting for CI. Changing that changes release behaviour. |
| Merging acceptance jobs (G3a with G3, Gold with G8) | Their `.env` files differ, so sharing a stack could mask a failure. |
| `bun check` as the blocking type gate | Not in a stable Bun release yet, and it reports TypeScript 7 errors while the repo is on TypeScript 5. |

## 6. Delivery

One PR per row, in this order. Each PR must be mergeable and useful on
its own. Branch from a fresh `origin/main` each time.

| PR | Contents | Sections | Expected effect |
|---|---|---|---|
| 1 | Scope detection, gate job, lint job merge, MSRV as `cargo check`, coverage off PRs | 4.1, 4.3 step 5, 4.4 | Docs-only 1 min, frontend-only 2–3 min, about 25 fewer runner-minutes |
| 2 | `images` job, Dockerfile with `cargo-chef`, compose `image:` keys, acceptance jobs consume artifacts, smoke test moved | 4.2 | Acceptance jobs drop from 10–16 min to 5–8 min |
| 3 | Test dedupe, nextest, Postgres pre-start, CI debug-info setting | 4.3 steps 1–4 | Test job drops from 13–16 min to about 6 min |
| 4 | Frontend job | 4.5 | Frontend job drops from 134s to about 70s |
| 5 | Optional follow-ups | 4.6 | Rust PR reaches 8–10 min |

Update `docs/CI.md` in each PR so it describes the workflow as it then
stands.

## 7. Acceptance criteria

### PR 1

- A PR touching only `docs/**` finishes with `ci-required` green in
  1 minute or less, and no Rust, frontend or acceptance job runs.
- A PR touching only `src/**` (not `dashboard-specs.ts`) runs the
  frontend job and the repo lints, and no Rust or acceptance job.
- A PR touching `src/lib/dashboard-specs.ts` runs the Rust test job.
- A PR touching `.github/**`, or a file no rule classifies, runs every
  job.
- A push to `main` runs every job.
- `ci-required` goes red when any job it needs fails or is cancelled,
  and when a job is skipped that the scope says should have run.
- Both scripts have self-tests, and the self-tests run in CI before the
  scripts are used.
- The 1.88.0 job runs `cargo check --workspace --all-targets --locked`
  and passes.

### PR 2

- The `lakehouse-api` release build runs once per workflow run. No
  acceptance job log contains a `cargo build`.
- With no change to `Cargo.lock`, the `images` job finishes in 5 minutes
  or less on a warm cache.
- Every acceptance job passes with the same test runner command as
  before.
- A fork PR passes. Verify with a real fork PR, or by running the
  workflow with a read-only `GITHUB_TOKEN`.
- Locally, `docker compose up -d --build lakehouse-api` on a clean
  checkout builds and serves `/health`, and a second build after a
  one-line source change is no slower than it was before this PR.
- The `/health` smoke test still runs on every Rust PR.

### PR 3

- The set of tests executed is unchanged, apart from the duplicated
  `lakehouse-api` binary tests. Show this by listing test names before
  and after (`cargo test -- --list` against `cargo nextest list`) and
  diffing them. The baseline run reported 4,005 passed, including the
  duplicates and doc tests.
- Doc tests still run.
- The job passes on five consecutive runs with no flaky failure. Any
  test that needs serial execution is pinned to a nextest test group
  with a comment naming the shared state it protects; it is not
  retried into passing.
- Exactly one Postgres container exists during the run.
- The test job finishes in 8 minutes or less.

### PR 4

- Lint, typecheck, unit tests and build all still run and all still
  gate the job.
- The frontend job finishes in 90 seconds or less.

### Every PR

- A deliberately broken commit on a throwaway branch shows the affected
  check going red. Do this once for a Rust test, once for a frontend
  type error and once for an acceptance test. Delete the branch
  afterwards.

## 8. Measuring

Record three runs before and three after, in the PR description.

```bash
# Wall-clock per run
gh run list --workflow CI --limit 10 \
  --json databaseId,createdAt,updatedAt,headBranch,conclusion

# Per-job and per-step seconds for one run
gh api "repos/RantAI-dev/RantAI-Lakehouse/actions/runs/<RUN_ID>/jobs?per_page=50" \
  --jq '.jobs[] | "## \(.name) \(((.completed_at|fromdate)-(.started_at|fromdate)))s",
        (.steps[] | select(.completed_at and .started_at)
         | {n: .name, d: ((.completed_at|sub("\\.[0-9]+";"")|fromdate)
                        - (.started_at|sub("\\.[0-9]+";"")|fromdate))}
         | select(.d >= 15) | "   \(.d)s  \(.n)")'
```

Report wall-clock from `createdAt` to `updatedAt`, which includes
queueing, and the duration of the slowest job, which does not. If a
target in section 1 is missed, say so with the numbers; do not adjust
the target.

## 9. Risks

| Risk | Mitigation |
|---|---|
| nextest is unstable on this codebase | Fallback in section 4.3: split `cargo test` by crate |
| Actions cache (10 GB) evicts the image layers | Watch `images` job time on `main`; shrink `rust-cache` contents first |
| Runner disk fills while linking 79 debug test binaries | Trimmed debug info in PR 3; free preinstalled toolchains as RantAIClaw does if it still happens |
| Image artifacts are large, mainly the Dagster image | Compress with zstd; if transfer exceeds 60s per job, rely on the `gha` layer cache for Dagster instead of an artifact |
| A path rule wrongly skips a job | Fail-safe `all` default, script self-tests, and every job runs on `main` |
| Queueing still inflates wall-clock when several PRs push together | Not fully solvable on free runners; fewer jobs and path filters reduce it |
| Conflicts with concurrent work in `ci.yml` and `docker-compose.yml` | Small PRs, rebase before each push |
