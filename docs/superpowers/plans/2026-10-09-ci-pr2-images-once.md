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

Developer: Claude Sonnet 5.5. Branch `ci/pr2-images-once`, cut from `main` at
`a78ad62`, plan commit `f9eb7cf`. No PR opened (not my role); no workflow has
run. Everything below was run by me on this machine (6 cores, 7 GB, shared) and
is quoted from the real output.

### Commits (one per task, plan order)

| Task | Commit | Subject |
| --- | --- | --- |
| T1 | `8a4fea1` | build(api-image): compile dependencies in their own layer with cargo-chef |
| T2 | `e26f15f` | build(compose): give the API and Dagster images stable names |
| T3 | `1291952` | ci: add an images job that builds the API and Dagster images once |
| T4 | `4a42add` | ci: acceptance jobs load the shared images and start with --no-build |
| T5 | `780ec99` | ci: ci-required gates on the images and image-smoke jobs |
| T6 | `ae3bd91` | ci(docker): run the Docker workflow on tags only |
| T7 | `78af076` | ci: pull requests restore rust-cache but no longer save it |
| T8 | `49e98e1` | docs(ci): describe the shared images, the cache rules and the new gate |

### Deviations from the plan, and mismatches found (read these first)

1. **T3 and T4 commits do not pass `test_required_gate.sh`.** The plan puts the
   new jobs in T3/T4 and adds them to `ci-required` only in T5, but the
   self-test asserts every job in `ci.yml` is in `ci-required`'s `needs`. I
   followed the plan's order and checked it at `1291952`: `25 passed, 1 failed`
   (the consistency check). The T3/T4 commit bodies say so; T5 (`780ec99`)
   fixes it. If you want every commit green, fold the `needs`/`env` lines of T5
   into T3 and the gate script into T4; I did not, because the plan fixes the
   task boundaries.
2. **The T2 test name in the plan is a helper, not a test.**
   `cargo test -p lakehouse-store demo_connector_compose_properties` runs 0
   tests (`demo_connector_compose_properties` is a `fn` at
   `cdc.rs:1130`; `include_str!` of `docker-compose.yml` is there). The test
   that calls it is `demo_connector_properties_match_the_checked_in_compose_file`
   (`cdc.rs:1164`); I ran that.
3. **T2 sub-step 1's build did not compile anything.** `up -d --build
   lakehouse-api` finished in 27 s: the tools, planner and `cook` steps were
   BuildKit cache hits from my own earlier benchmark builds in the default
   builder, and the final `cargo build` ran against the warm cache mount and
   printed `Finished ... target(s) in 1.30s` with no `Compiling` line (sub-step
   2's API build printed no `Compiling` line either). The from-scratch
   default-argument build is the T1 "A1" run below (734.7 s, fresh
   builder).

### Commands run and results

**Per-commit scoped checks**

```
bash scripts/ci/tests/test_required_gate.sh        -> Results: 37 passed, 0 failed   (HEAD 49e98e1)
bash scripts/ci/tests/test_detect_change_scope.sh  -> Results: 32 passed, 0 failed   (HEAD 49e98e1)
python3 ops/lint/check_compose_init_readiness.py   -> exit 0, no output
sg docker -c "docker compose --profile '*' config --quiet" -> exit 0
cd rust && CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target \
  cargo test -p lakehouse-store demo_connector_properties_match_the_checked_in_compose_file
  -> test cdc::tests::demo_connector_properties_match_the_checked_in_compose_file ... ok
     test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 153 filtered out
actionlint 1.7.12 (pip, venv in /tmp) on ci.yml and docker.yml -> exit 0
python3 yaml.safe_load on both workflows -> parses
git status --short (HEAD 49e98e1) -> empty
```

`shellcheck` is not installed, so actionlint did not check the shell in `run:`
steps. The gate self-test went from 26 cases to 37 (11 new). To check they
test the change, I removed the two new `check_job` lines from a temp copy of
the gate: 8 of the 11 fail (`29 passed, 8 failed`); the other 3 are the
"consumers skipped" cases (which are red through the existing consumer checks
anyway) and the docs-only case that must stay green.

No Rust source changed, so no workspace cargo run.

**T1: Dockerfile.** All builds from a temp copy of the context
(`/tmp/pr2bench/ctx*`, outside the repo). Old Dockerfile = `git show
a78ad62:rust/Dockerfile`. One-line change = a comment appended to
`crates/lakehouse-api/src/lib.rs`.

| Build | Result |
| --- | --- |
| Old Dockerfile, cold, default builder | `rc=0 wall=656.6s` |
| Rebuild after a one-line change, old Dockerfile (**before**) | 279.0 s, 274.9 s, 275.2 s |
| First new-Dockerfile build into the warm mount (installs cargo-chef, first cook) | 396.3 s |
| Rebuild after a one-line change, new Dockerfile (**after**) | 275.9 s, 273.2 s |
| A1: new Dockerfile, default args, **fresh** `docker-container` builder | `rc=0 wall=734.71s` |
| A2: same builder, `--build-arg CARGO_TARGET_DIR=/build/target-layer --cache-to type=local,dest=...,mode=max` | `rc=0 wall=688.69s` |
| B: **fresh** builder, one-line change, `--build-arg CARGO_TARGET_DIR=/build/target-layer --cache-from type=local,src=...` | `rc=0 wall=343.53s` |

The before/after rebuilds alternated old/new/old/new/old back to back
(`alt.sh`), same context, same machine, one heavy build at a time. After (mean
274.6 s) is not slower than before (mean 276.4 s). In the new rebuild log the
tools layers, the planner copy of the manifests and the `cook` step print
`CACHED`; only `COPY crates`, `chef prepare` (0.2 s) and the final build re-run.

Build B (quoted from its log; the layer cache came from another builder):

```
#14 [tools 2/4] WORKDIR /build                                  #14 CACHED
#15 [tools 3/4] RUN apt-get update && apt-get install ...       #15 CACHED
#16 [tools 4/4] RUN ... cargo install sqlx-cli ... cargo-chef   #16 CACHED
#21 [builder 2/6] RUN ... cargo chef cook --release --locked ... #21 CACHED
#25 [builder 6/6] RUN ... cargo build --release --locked -p lakehouse-api ...
#25 13.86    Compiling lakehouse-core v0.1.0 (/build/crates/lakehouse-core)
 ... (13 "Compiling" lines in total, all lakehouse-*; no third-party crate)
#25 22.55    Compiling lakehouse-api v0.1.0 (/build/crates/lakehouse-api)
#25 298.7     Finished `release` profile [optimized] target(s) in 4m 58s
```

Two things B shows that the plan did not predict. The final `cargo build`
re-downloaded the registry crates (about 13 s: `Downloading crates ...` before
the first `Compiling`) because the registry cache mount is not restored by a
layer cache. The `type=local` cache export of the API image is 953 MB on disk
(998,426,098 bytes of blobs): the `api-image` scope on `main` will take about
that much of the Actions cache (9.9 GB of 10 GB used at the time of writing),
which is the risk parent plan section 9 names; the `rust-cache` change in T7
frees roughly 980 MB per PR ref. I did not measure the real `type=gha` size.

By-hand `docker.yml` steps against the default-args image from A1:
`docker run -d --name ... -p 28090:8080 <image>` then the `/health` loop:
attempts 1–15 returned `000`, attempt 16 returned `200` (about 32 s, while
build A2 was running on the same machine). The loop in `ci.yml` allows 30
attempts of 2 s. I did not investigate why the first 30 s were slow.

**T2: real `docker compose up`** (clean projects of my own, fresh names, env
file outside the repo `--env-file /tmp/pr2bench/proof.env` with the CI-style
bootstrap credentials and host ports 23000–29011, all checked free; `down -v`
run first, and after each project). Run from this worktree at `HEAD=49e98e1`,
tree clean.

1. `docker compose -p pr2proof1 ... up -d --build lakehouse-api` -> `rc=0 wall=27s`
   (cached, see deviation 3); `curl http://localhost:28080/health` -> `attempt 1: 200`.
   `docker image ls` -> `rantai-lakehouse-api:local` and the fixture
   `pr2proof1-oidc-mock:latest`.
2. `docker compose -p pr2proof2 ... --profile dagster up -d --build lakehouse-api
   dagster-code-location dagster-webserver dagster-daemon` -> `rc=0 wall=186s`.
   `compose ps`: the three Dagster services all show image
   `rantai-lakehouse-dagster:local`, `lakehouse-api` shows
   `rantai-lakehouse-api:local`. `docker image ls | grep -E 'rantai-lakehouse|pr2proof2'`:
   `rantai-lakehouse-dagster:local`, `rantai-lakehouse-api:local`,
   `pr2proof2-oidc-mock:latest`; the count of `rantai-lakehouse-(api|dagster)`
   images is 2. `/health` -> 200.
3. `scripts/compose.sh -p pr2proof2 ... --profile dagster up -d --build <same four services>`
   -> `rc=0 wall=195s`. `HEAD=49e98e1fda539bd44714778b66061a11034edd35`;
   `docker inspect` `GIT_SHA` is `49e98e1fda539bd44714778b66061a11034edd35` in
   `pr2proof2-lakehouse-api-1`, `pr2proof2-dagster-code-location-1`,
   `-dagster-webserver-1` and `-dagster-daemon-1`.
4. After `down -v` on `pr2proof2` and `docker rmi rantai-lakehouse-api:local
   rantai-lakehouse-dagster:local` (both removed; `docker image ls` then lists no
   `rantai-lakehouse*`): `docker compose -p pr2proof3 ... up -d --no-build
   lakehouse-api` **fails, exit 1**. First on the fixture
   (`No such image: pr2proof3-oidc-mock:latest`: decision 4, which is why the CI
   jobs build `oidc-mock` first). With the fixture built, it fails on the shared image:
   `service:lakehouse-api:1 Error response from daemon: No such image: rantai-lakehouse-api:local`,
   `exit=1`.

Extra, not in the plan: I emulated one CI consumer locally. Image from build B
(`CARGO_TARGET_DIR=/build/target-layer`) tagged `rantai-lakehouse-api:ci`,
`docker save | zstd -T0 -3` -> 62,195,772 bytes (docker-save format, not the
buildx `type=docker` tar, so the CI size may differ), `docker rmi`,
`zstd -dc | docker load` (`Loaded image: rantai-lakehouse-api:ci`),
`LAKEHOUSE_API_IMAGE=rantai-lakehouse-api:ci`, `compose build oidc-mock`, then
`up -d --no-build lakehouse-api` -> exit 0 and `/health` 200 on attempt 2.

Cleanup: projects `pr2proof1`, `pr2proof2`, `pr2proof3`, `pr2proofci` were
brought down with `-v` (0 containers, volumes and networks with that prefix
remain); my `pr2proof-*` image tags, the `rantai-lakehouse-*` tags and the two
`docker-container` buildx builders were removed by exact name. I did not prune
anything and did not touch any other project, image or volume.

**Action versions.** `checkout@v7`, `docker/setup-buildx-action@v4`,
`docker/build-push-action@v7` are what the workflows already use.
`actions/upload-artifact@v7` is already used by `coverage.yml`; latest is
`v7.0.2` (2026-10-07). `actions/download-artifact`: no workflow uses it; I took
the newest major, `v8` (`v8.0.2`, 2026-10-07). Checked with
`git ls-remote --tags --refs https://github.com/actions/download-artifact.git`
(highest majors `v6 v7 v8`) and `gh api repos/actions/download-artifact/releases`
(latest `v8.0.2`); same for `upload-artifact` (`v5 v6 v7`).

### Not verified (and why)

CI only can prove these; there is no PR, so no workflow ran.

- The `images` job on a runner: that `docker/build-push-action@v7` accepts an
  empty `cache-to` on PRs, that `type=docker,dest=` works with the buildx the
  runner gets, that `zstd` is present on `ubuntu-latest` (assumed), the real
  `type=gha` cache size and hit rate, artifact sizes and transfer times, and
  the "5 minutes or less on a warm cache" criterion.
- Each acceptance job with `--no-build` (G3a, G3, G4, G6 both projects, Gold,
  G8) and `image-smoke` on a runner; `download-artifact@v8` with
  `pattern`/`merge-multiple`; `docker load` of a buildx `type=docker` archive.
  Locally I proved only the compose side (`--no-build` with an image of that
  name, fixtures built first) for the API service.
- The read-only token ("a fork PR passes"): `permissions: contents: read` is in
  the file, but no run used it.
- `docker.yml` on a tag, including `outputs: type=cacheonly` and reading the
  `api-image` scope from a tag ref.
- All timings in section 7 of the parent plan and the "deliberately broken
  commit" checks under "Every PR".
- Shell steps in the workflows were not shellchecked (`shellcheck` absent).
- The `gold-export` and `g8-governance` jobs download only the API artifact and
  never start a Dagster service; I read the compose dependency closure (a
  Python walk of `depends_on`) rather than ran those jobs. The closure of each
  job contains only `lakehouse-api`, `oidc-mock` and (G3a/G3/G4/G6) the Dagster
  services as `build:` services, plus `g6-test-runner` and `rest-stub-g6` for
  the G6 matrix project.

Section 3.3 cause 1 of `CI-SPEED-PLAN.md` is not falsified by anything I
measured, so it is unchanged. Final `git status --short`,
`git log --oneline origin/main..HEAD` and `git ls-remote --heads origin
ci/pr2-images-once` can only be produced after the push, so they are in my
final message rather than in this file.

## Review

(Planner.)
