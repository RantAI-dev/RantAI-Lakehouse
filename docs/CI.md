# CI/CD

This repo's CI is split into four workflows under `.github/workflows/`:

- **`ci.yml`** — change-scoped fast feedback and acceptance testing. Begins with
  a change scope detector (`changes`) and unconditional repo lints (`repo-lints`),
  runs selective frontend/Dagster/Rust/acceptance jobs based on modified paths,
  and gates merges with a single summary gate (`ci-required`).
- **`security.yml`** — `cargo audit`, `cargo deny check all`, a working-tree
  `gitleaks` scan, GitHub's `dependency-review-action` on PRs, and a
  full-git-history `history-scan` job, which is currently **RED for a real
  reason** (see below) and is deliberately excluded from required checks.
- **`docker.yml`** — builds `rust/Dockerfile`, boots the container, and
  asserts `/health` returns 200. A GHCR push job exists but is gated on tag
  pushes and is currently inert (no registry login configured).
- **`coverage.yml`** — `cargo llvm-cov` (lcov, uploaded as a workflow
  artifact) and a CycloneDX SBOM per crate (also uploaded as an artifact,
  for Phase 6's release attachment). Runs on `push` to `main` and manual dispatch
  only to avoid spending ~30 runner minutes per PR run when no PR checks consume
  the artifact.

## Change Scopes and the Single Gate (`ci-required`)

To reduce CI feedback latency on targeted changes (such as docs-only or frontend-only PRs),
`ci.yml` classifies changes in the `changes` job into six scopes:

| Scope | Triggered By |
|---|---|
| `rust` | `rust/**`, `src/lib/dashboard-specs.ts` |
| `frontend` | `src/**`, `public/**`, `package.json`, `bun.lock`, `bunfig.toml`, `tsconfig.json`, `next.config.ts`, `eslint.config.mjs`, `postcss.config.mjs`, `components.json`, `Dockerfile.frontend` |
| `dagster` | `dagster/**` |
| `stack` | `docker-compose.yml`, `ops/**`, `scripts/**`, `.env.example` |
| `docs_only` | True when every changed file is under `docs/**`, `GTM/**`, or is a root `*.md`, `LICENSE` or `NOTICE` |
| `all` | `.github/**`, an unreadable base SHA, or any file unclassified by the rules above (fails safe) |

On a `push` to `main`, all jobs run unconditionally.

Downstream jobs execute according to scope:
- **`repo-lints`**: Runs unconditionally on all pushes and PRs. Folds the four static lints (parity corpus secrets, R11 bare iceberg count, compose init readiness, Dagster intra-package imports) into one fast job.
- **`verify`**: Runs if `frontend` or `all` is true.
- **`dagster-unit-tests`**: Runs if `dagster` or `all` is true.
- **Rust fmt / clippy / build / test / msrv**: Runs if `rust` or `all` is true.
- **Acceptance jobs (`g1-rustfs`, `g2-seaweedfs`, `g3a-dagster`, `g3-maintenance`, `g4-cdc`, `g6-ingest`, `gold-export`, `g8-governance`)**: Run if `rust`, `dagster`, `stack`, or `all` is true.

At the end of the pipeline, **`ci-required`** runs with `if: always()`, checks every job result against the detected scope, and fails if any required job failed, cancelled, or was skipped when the scope indicated it should have run. Both the scope detection script (`scripts/ci/detect_change_scope.sh`) and the gate script (`scripts/ci/required_gate.sh`) have accompanying self-tests that run in CI before the scripts are executed.

## MSRV

`rust/Cargo.toml`'s `rust-version` is `1.88`, not the edition-2024 floor of
`1.85`. This was verified empirically, not assumed: building `--locked`
against 1.85.0 fails because `testcontainers`/`testcontainers-modules`
require rustc 1.88, `time`/`time-core`/`time-macros` require 1.88.0,
`etcetera` requires 1.87.0, and `ferroid` requires 1.85.1. Building against
1.88.0 succeeds.

In CI, the `msrv` job runs a dedicated `cargo check --workspace --all-targets --locked`
under toolchain `1.88.0` to verify compatibility without duplicating test run
times, while the full test suite runs under `stable`.

`rust-toolchain.toml` pins `1.96.1` — intentionally newer than the MSRV. That
file is what CI's `fmt`/`clippy`/`build` jobs and local dev actually build
with; the MSRV check job exists specifically to catch MSRV
regressions that the day-to-day toolchain wouldn't.

## `history-scan`: currently RED, and correctly so

A Phase 1 audit found a real internal LLM API key leaked in git history, on 2 commits
reachable from both `main` and `feat/rust-backend`, together with two
internal LAN hostnames. The exact values are deliberately not reproduced
here — see the rule definitions in `.gitleaks.toml`.

**gitleaks' default ruleset does not match either pattern.** Scanned with
defaults, `history-scan` came back green — a false negative. That is worse
than no scan at all: it tells a reviewer history is clean when it is not.

`.gitleaks.toml` therefore adds two custom rules (`rantai-llm-node-api-key`,
`rantai-internal-lan-host`) matching the key prefix and host range. With
them, the scan reports the truth:

```
Finding:     https://...:8443
RuleID:      rantai-internal-lan-host
Commit:      8cdd285 (chore: save local dev progress)

Finding:     <prefix>-<32-hex>
RuleID:      rantai-llm-node-api-key
Commit:      8cdd285 (chore: save local dev progress)
Commit:      b067f96 (chore: update config)
```

The key has been revoked upstream, so the exposure is dormant, but the
secret remains in the reachable DAG until an explicit rewrite (`git filter-repo`
or `bfg`) is performed. That rewrite would rewrite every commit SHA in the
repo and force every collaborator to re-clone or rebase; the team chose to
defer the rewrite until the next scheduled maintenance window.

Until then, `history-scan` **must stay red** to tell the truth.
`security.yml` marks it non-blocking by **not** including it in `main`'s
required status checks. A future contributor who fixes the repo history
will see this job turn green on its own, without having to touch CI config.

Do not remove the custom rules to make the job green. Do not add `|| true`
to the gitleaks invocation. Do not delete the job. The red check is a
reminder of pending technical debt, not a CI bug.

## Dependency review: now works, verified on a real PR

`dependency-review-action` previously failed with:

```
Error: Dependency Review is not supported on this repository.
Please ensure that the repository is public or has GitHub Advanced
Security enabled.
```

The repo is public, so GitHub Advanced Security features (including
Dependency Review) are free and available without a paid license, but the
feature was not enabled in repository settings.

It has since been enabled: `dependency-review-action` runs on every pull
request targeting `main`, parsing package manifest changes (`bun.lock`,
`Cargo.lock`) and failing the PR if a newly introduced dependency has a
known advisory with severity above the configured threshold.

Because the action requires a PR event payload (`github.event.pull_request`),
it is skipped on `push: branches: [main]` runs. That is normal and expected.

## Docker smoke test: `/health` contract

`docker.yml` builds `rust/Dockerfile` via `docker buildx`, starts the
container with minimal required environment variables (`POSTGRES_URL`,
`CLICKHOUSE_URL`, `LAKEKEEPER_BASE_URI`, `JWT_SECRET`), waits up to 60 seconds
for the container to become healthy, and executes:

```sh
curl -fsS http://localhost:8080/health
```

The `/health` endpoint is unauthenticated and returns `{"status":"ok"}`
when the axum server is ready to accept traffic. It does not probe database
connectivity (those checks belong to the readiness probe, not the liveness
smoke test).

If the container crashes on boot (e.g. missing environment variable, dynamic
linker failure, panic during router initialization), `docker inspect` dumps
the container logs to the CI transcript before the job exits with failure.

## Recommended branch protection

Branch protection cannot be fully managed by contributors unless the
acting account has org-admin rights. Rather than fight that, here is the
exact command an owner can run themselves, and the reasoning behind each
choice, so it can be applied (or adjusted) in one step instead of clicking
through the UI.

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
  -f 'required_status_checks[checks][][context]=Build lakehouse-api image · smoke test /health' \
  -F 'enforce_admins=false' \
  -F 'required_pull_request_reviews=null' \
  -F 'restrictions=null' \
  -F 'allow_force_pushes=false' \
  -F 'allow_deletions=false'
```

The check names above reflect `ci-required` (which consolidates and validates all scope-dependent tests in `ci.yml`), `Repo lints`, security scans, and the Docker smoke test.

Why each choice:

- **Required checks are the honest-green ones only.** `ci-required` (enforcing
  all scope-required CI jobs), `Repo lints`, `cargo audit`, `cargo deny`,
  `gitleaks (working tree)`, and the Docker smoke test are the jobs that
  are expected to actually pass on a healthy `main`.
- **`gitleaks (full git history)` is deliberately NOT in this list.** It is
  expected to stay red (see "Known exposure" in
  [SECURITY.md](../SECURITY.md) and the `history-scan`/history section
  above). Requiring it would either block every future merge forever, or
  pressure someone into silencing a real finding — neither is acceptable.
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
