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
| `rust` | `rust/**`, `src/lib/dashboard-specs.ts`, `docker-compose.yml` |
| `frontend` | `src/**`, `public/**`, `package.json`, `bun.lock`, `bunfig.toml`, `tsconfig.json`, `next.config.ts`, `eslint.config.mjs`, `postcss.config.mjs`, `components.json`, `Dockerfile.frontend` |
| `dagster` | `dagster/**` |
| `stack` | `docker-compose.yml`, `ops/**` (except `ops/fixtures/**`), `scripts/**` (except `scripts/ci/**`), `.env.example` |
| `docs_only` | True when every changed file is under `docs/**`, `GTM/**`, or is a root `*.md`, `LICENSE` or `NOTICE` |
| `all` | `.github/**`, `ops/fixtures/**`, `scripts/ci/**`, an unreadable base SHA, or any file unclassified by the rules above (fails safe) |

On a `push` to `main`, all jobs run unconditionally.

Downstream jobs execute according to scope:
- **`repo-lints`**: Runs unconditionally on all pushes and PRs. Folds the four static lints (parity corpus secrets, R11 bare iceberg count, compose init readiness, Dagster intra-package imports) into one fast job.
- **`verify`**: Runs if `frontend` or `all` is true.
- **`dagster-unit-tests`**: Runs if `dagster` or `all` is true.
- **Rust fmt / clippy / build / test / msrv**: Runs if `rust` or `all` is true.
- **Acceptance jobs (`g1-rustfs`, `g2-seaweedfs`, `g3a-dagster`, `g3-maintenance`, `g4-cdc`, `g6-ingest`, `gold-export`, `g8-governance`, `g8-time-travel`)**: Run if `rust`, `dagster`, `stack`, or `all` is true.

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
with; the MSRV check job exists specifically to catch MSRV regressions that the
day-to-day toolchain wouldn't.

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
$ gitleaks detect --config .gitleaks.toml
leaks found: 6
  rantai-llm-node-api-key: 1
  rantai-internal-lan-host: 5
```

**This job is expected to fail until the history is rewritten.** Clearing it
requires `git filter-repo` (or equivalent) plus a force-push, which rewrites
published history — a human decision that has deliberately not been taken.
The leaked key should also be rotated at its source, independently of any
rewrite; removing it from git does not un-leak it.

The working-tree `gitleaks` job uses the same config and **is** green, because
the key is no longer present in any tracked file (the file that held it was
deleted during the Rust port). `.env.local` does contain live values and is
flagged on a local scan, but it is gitignored and never checked out in CI.

`history-scan` stays a **separate job**, not folded into the working-tree
`gitleaks` job and not wrapped in `continue-on-error`, precisely so that
whichever way it goes (red or green) is independently visible in the
Actions UI on every run, rather than being averaged into another job's
status. It is **excluded from required status checks** (see below) so that
if a future run does turn red — a real regression, a rule update, or new
evidence — that alone cannot silently block a merge; a human needs to look
at it.

If the Phase 1 leak is confirmed for real (e.g. by locating the exact commit
by other means), resolving it requires:

1. Rotating the exposed key at the provider.
2. Rewriting history (`git filter-repo` or BFG) to purge the blob from every
   affected commit.
3. Force-pushing every affected ref, which invalidates every existing clone
   and any open PR based on the old history.

That is a destructive, cross-cutting, and irreversible-for-clones operation,
deliberately not undertaken as a side effect of a CI-hardening pass — it
needs a human to explicitly decide to take it on and coordinate with anyone
holding a local clone.

## Docker

`docker.yml` builds the image and runs it standalone (no Postgres/ClickHouse
containers), because `lakehouse-api` is designed to boot and serve its
DB-independent routes — including `/health` — without either dependency
(`entrypoint.api.sh` skips migrations when `DATABASE_URL` is unset; see
`lakehouse_store::connect_lazy`'s doc comment). That's a real smoke test of
the container's own boot path, not a substitute for `docker compose up`
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

## Recommended branch protection (attempted, blocked — apply manually)

An attempt was made to set this via `gh api` during the public-release
hardening pass; it was blocked by a permission classifier even though the
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

The check names above reflect the real `name:` values from
`.github/workflows/{ci,security,docker}.yml` as of this writing — not
guessed. `ci-required` consolidates and enforces all scope-dependent tests
in `ci.yml`. If a workflow's job names change, this list needs to be updated to
match, or `strict` mode will block merges on a check that no longer reports.

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
- Consider **requiring signed commits** and **linear history** once the
  team's workflow is settled; neither is load-bearing for this phase.

This has to be applied by someone with admin rights on the repo — it is not
something a workflow file can configure for itself, and (as noted above) it
could not be applied programmatically during this pass either.
