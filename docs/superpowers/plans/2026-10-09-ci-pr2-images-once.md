# CI speed PR 2: build each image once per run

Parent plan: `docs/plans/CI-SPEED-PLAN.md`, sections 4.2, 6 (row 2), 7
("PR 2" and "Every PR"), 8 and 9. This file turns that section into tasks.
Where the two disagree, this file wins and says why.

- Branch: `ci/pr2-images-once`, cut from `main` at `a78ad62`.
- Anchors below were verified at `a78ad62`. Re-check them before editing;
  if one has moved, find the new line. If one is *wrong* (the code does not
  do what this plan says it does), stop and report.
- Roles: `AGENTS.md`. The developer writes the code and the handoff. The
  planner reviews, opens the PR, measures CI and merges.

## Decisions already made

Do not reopen these. If one cannot work, stop and report the mismatch.

1. **Two shared images, built once.** An `images` job in `ci.yml` builds
   the API image and the Dagster image, and uploads each as a workflow
   artifact (`docker save | zstd`, one-day retention). Consumers download,
   `docker load`, and start the stack with `up -d --no-build`.
2. **Compose image names.** `lakehouse-api` gets
   `image: ${LAKEHOUSE_API_IMAGE:-rantai-lakehouse-api:local}`. The five
   services built from `./dagster` get
   `image: ${LAKEHOUSE_DAGSTER_IMAGE:-rantai-lakehouse-dagster:local}`.
   Every `build:` block stays. CI sets both variables to `…:ci` tags in the
   job `env:`.
3. **The five Dagster-built services must have identical `build:` blocks.**
   `dagster-code-location`, `dagster-webserver` and `dagster-daemon` pass
   `args: GIT_SHA`; `g6-test-runner` (`docker-compose.yml:3076`) and
   `backup-job` (`:1925`) do not. Once all five share one image name, a
   build of either of those two would retag the shared image with
   `GIT_SHA=unknown`, and the console's source view would answer 409
   (see the comment at `docker-compose.yml:1677`). So add the same `args:`
   to those two, with a comment saying why.
4. **Fixture images are not shared.** `oidc-mock` (`:759`) and
   `rest-stub-g6` (`:2963`) are tiny, have `build:` and no `image:`, and
   stay that way. Because `--no-build` would fail on them, each acceptance
   job runs an explicit `docker compose … build oidc-mock` (plus
   `rest-stub-g6` in the G6 matrix project) with the same `-p`, `-f` and
   `--profile` flags as its `up`, immediately before the `up`.
5. **`g1-rustfs` and `g2-seaweedfs` do not consume the artifacts.** They
   never built the API image (`g1-test-runner` is `rust:1.96.1-slim`,
   `docker-compose.yml:1228`). Leave both jobs untouched. The parent plan's
   "six acceptance jobs" is the right count: G3a, G3, G4, G6, Gold, G8.
6. **`cargo-chef`, with the local build cache kept.** The Dockerfile today
   puts `target/` in a cache mount, which is what makes a local one-line
   rebuild fast and what makes the layer useless to CI's layer cache. Keep
   both behaviours with one build argument:
   - `ARG CARGO_TARGET_DIR=/build/target`, exported as `ENV`, declared in
     the builder stage before the first `cargo` call that compiles the
     workspace.
   - The cache mount stays at the fixed path `/build/target` on both the
     `cook` step and the final `cargo build` step. Locally the default
     points cargo at the mount, exactly as today.
   - CI passes `CARGO_TARGET_DIR=/build/target-layer`. That path is not
     under a mount, so `cook`'s output lands in the image layer and the
     `gha` cache can restore it.
   - Copy the binary out with `cp "$CARGO_TARGET_DIR/release/lakehouse-api"`.
7. **Tools are pinned and installed above every `COPY` of repo files.**
   `cargo install cargo-chef --version <exact> --locked` beside the existing
   `sqlx-cli` install. No `latest` tags, no third-party base image.
8. **Layer cache is written from `main` only.** `cache-from: type=gha` on
   every run, `cache-to: type=gha,mode=max` only when
   `github.event_name == 'push'`. Separate `scope` per image (`api-image`,
   `dagster-image`). Reason: the repo's Actions cache is at 9.9 GB of the
   10 GB cap today (measured 2026-10-09), and a PR-scoped cache can only be
   read by that PR.
9. **Stop PR runs from saving `rust-cache`.** Measured today: each PR
   stores its own 980 MB `build` cache and more, next to `main`'s. Add
   `save-if: ${{ github.ref == 'refs/heads/main' }}` to every
   `Swatinem/rust-cache` step in `ci.yml`, and remove the cache step from
   the `fmt` job (`cargo fmt --check` compiles nothing; its cache is
   107 MB per ref of nothing). This is the mitigation the parent plan
   names in 4.2 and section 9; the trigger for it has already happened.
10. **Read-only token for the whole workflow.** Add top-level
    `permissions: contents: read` to `ci.yml`, with a comment. This is how
    the "a fork PR passes" criterion is proven: every run then uses the
    token a fork PR gets.
11. **`GIT_SHA` stays unset in CI** (parent plan 3.4).
12. **Out of scope:** `security.yml`, `coverage.yml`,
    `deploy-staging.yml`, anything under `rust/crates/`, the `test` job's
    commands (that is PR 3), GHCR publishing, merging acceptance jobs.

## Anchors

| What | Where (`a78ad62`) |
| --- | --- |
| API build block, no `image:` | `docker-compose.yml:80-104` |
| Dagster-built services | `docker-compose.yml:1674`, `1925`, `1953`, `1991`, `3076` |
| Fixture builds | `docker-compose.yml:759`, `2963` |
| `up -d --build` call sites | `.github/workflows/ci.yml:435`, `482`, `552`, `621`, `675`, `721`, `760` |
| Gate job `needs` and env | `.github/workflows/ci.yml:777-833` |
| Gate script and self-test | `scripts/ci/required_gate.sh`, `scripts/ci/tests/test_required_gate.sh` (the self-test asserts every job is in `needs` and has a result variable) |
| Smoke test to move | `.github/workflows/docker.yml:19-80` |
| Dockerfile build stage | `rust/Dockerfile:14-47` |
| Staging deploy uses `up -d --build` | `.github/workflows/deploy-staging.yml:75` |

## Tasks

One task per commit, in this order. Each commit runs only the checks for
what it touched (`AGENTS.md`, "Keep build time down").

### T1. `rust/Dockerfile`: dependency layer with `cargo-chef`

Stages: a tools base (apt packages, `sqlx-cli`, `cargo-chef`); a planner
that copies `Cargo.toml`, `Cargo.lock` and `crates/` and runs
`cargo chef prepare`; a builder that copies only `recipe.json`, runs
`cargo chef cook --release --locked -p lakehouse-api`, then copies the
sources and `migrations/` and runs the existing `cargo build`. Decision 6
governs the target directory.

The runtime stage must not change in behaviour: same binary path, `sqlx`
binary, `migrations`, `specs`, `pipeline_src` copy, `PIPELINE_PACKAGE`,
`GIT_SHA`, entrypoint. Keep every comment that is still true; rewrite the
ones that are not (the "CI's GHA layer cache does not persist cache
mounts" paragraph changes meaning).

Acceptance:
- `docker build` of the image succeeds from a clean builder with default
  arguments and with `--build-arg CARGO_TARGET_DIR=/build/target-layer`.
- With the `target-layer` argument and a local layer cache
  (`--cache-to/--cache-from type=local`), a second build after a one-line
  change in `crates/lakehouse-api/src/` shows the `cook` step as `CACHED`
  and compiles workspace crates only. Quote the build log lines.
- With default arguments, time a rebuild after a one-line change **before**
  this commit (on `a78ad62`) and **after** it, back to back, same machine.
  After must be no slower. Record both numbers. If it is slower, stop and
  report.
- The container boots standalone and `/health` returns 200 (the
  `docker.yml` steps, run by hand).

### T2. `docker-compose.yml`: image keys

Decisions 2 and 3. Comment each `image:` key once at the API service and
once at `dagster-code-location`, and point the other four at that comment.

Acceptance:
- `docker compose --profile '*' config --quiet` passes.
- `python3 ops/lint/check_compose_init_readiness.py` passes.
- `cd rust && cargo test -p lakehouse-store demo_connector_compose_properties`
  passes (`cdc.rs` embeds `docker-compose.yml` with `include_str!`).
- **Real `up`, clean project** (`AGENTS.md` rule 8). With a fresh project
  name and `down -v` first, using free host ports:
  1. `docker compose -p <fresh> up -d --build lakehouse-api` builds, and
     `/health` answers 200 on the published port.
  2. `docker compose -p <fresh> --profile dagster up -d --build
     lakehouse-api dagster-code-location dagster-webserver dagster-daemon`
     comes up; `docker image ls` shows one API image and one Dagster image
     under the default names.
  3. `scripts/compose.sh … up -d --build` of the same services, then
     `docker inspect` shows `GIT_SHA` equal to `git rev-parse HEAD` in both
     the API container and `dagster-code-location`.
  4. With both images removed, `up -d --no-build lakehouse-api` **fails**
     and names the missing image.
  Tear each project down with `down -v`. Other agents use this machine:
  never touch a project you did not create.

### T3. `ci.yml`: the `images` job

Same `if:` as the acceptance jobs. `docker/setup-buildx-action`, then one
`docker/build-push-action` per image with `outputs: type=docker,dest=…`,
the tags from decision 2, the caches from decision 8, and for the API
image `build-contexts: pipeline_src=dagster/dispar_orchestrate` and
`build-args: CARGO_TARGET_DIR=/build/target-layer`. Compress with `zstd`,
upload with `actions/upload-artifact`, `retention-days: 1`,
`compression-level: 0`. Print each archive's size in the log.

Add `permissions: contents: read` at the top of the workflow (decision
10).

### T4. `ci.yml`: consumers

For G3a, G3, G4, G6 (both projects), Gold and G8: `needs: [changes,
images]`, keep the `if:`, add the image variables to the job `env:`,
download and `docker load` only the artifacts that job uses (Gold and G8
need the API image only), build fixtures (decision 4), and replace
`up -d --build` with `up -d --no-build`. The test-runner commands do not
change by a character.

Because a job with `needs: images` is skipped when `images` fails, check
that `required_gate.sh` still turns red in that case (it should: a
required job that is `skipped` is an error) and add a self-test for it.

Add an `image-smoke` job: same `if:`, `needs: [changes, images]`, loads
the API artifact and runs the `/health` steps moved from `docker.yml`
with their comments.

### T5. Gate

Add `images` and `image-smoke` to `ci-required`'s `needs`, its `env:`, the
header and body of `required_gate.sh` (both required when acceptance is),
and the self-tests.

Acceptance: `bash scripts/ci/tests/test_required_gate.sh` and
`bash scripts/ci/tests/test_detect_change_scope.sh` pass; quote the
counts.

### T6. `docker.yml`: tag trigger only

Remove the `push: branches` and `pull_request` triggers. The build job
stays for tags, reading `cache-from: type=gha,scope=api-image` and writing
no cache. Rewrite the comments to match.

### T7. `rust-cache` on PRs

Decision 9.

### T8. `docs/CI.md`

Describe the workflow as it now stands: the `images` job, artifacts,
`--no-build`, where the smoke test lives, what `docker.yml` still does,
the cache rules and why. Fix the branch-protection command: the check
`Build lakehouse-api image · smoke test /health` no longer reports on
PRs, so it must leave the required list (`ci-required` covers the smoke
job). Mark `docs/plans/CI-SPEED-PLAN.md` status as "PR 1 merged (#88),
PR 2 in review" and correct section 3.3 cause 1 only if this work
falsified it.

## Verification before handoff

No Rust source changes, so no workspace cargo run; only the one
`lakehouse-store` test in T2.

```bash
bash scripts/ci/tests/test_required_gate.sh
bash scripts/ci/tests/test_detect_change_scope.sh
python3 ops/lint/check_compose_init_readiness.py
sg docker -c "docker compose --profile '*' config --quiet"
# plus the T1 builds and the T2 real `up`, quoted
git status
```

Local notes: this shell is not in the `docker` group; wrap docker commands
in `sg docker -c '…'`. Cargo uses
`CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target`. The machine has 6
cores and 7 GB of memory; run one heavy build at a time.

## What the developer cannot verify, and must say so

CI itself. The branch is pushed but the developer does not open a PR, so
no workflow runs. Write "not verified" for: the `images` job on a
runner, artifact size and transfer time, each acceptance job with
`--no-build`, the read-only token, and all timings in section 7 of the
parent plan. The planner measures those on the PR and records them under
Review.

## Handoff

(Developer: append here. Commits, exact commands with their output
counts, what was skipped or not verified and why, then the output of
`git status --short`, `git log --oneline origin/main..HEAD` and
`git ls-remote --heads origin ci/pr2-images-once`.)

## Review

(Planner.)
