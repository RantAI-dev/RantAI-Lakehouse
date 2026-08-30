# CI/CD

This repo's CI is split into four workflows under `.github/workflows/`:

- **`ci.yml`** — fast-feedback correctness: frontend lint/typecheck/test/build,
  the parity-corpus secret-shape check, and Rust `fmt` / `clippy` / `build` /
  `test` (the last as a matrix over `stable` and the declared MSRV).
- **`security.yml`** — `cargo audit`, `cargo deny check all`, a working-tree
  `gitleaks` scan, GitHub's `dependency-review-action` on PRs, and a
  full-git-history `history-scan` job, whose result is currently green
  (see below for the honest, non-obvious story behind that).
- **`docker.yml`** — builds `rust/Dockerfile.api`, boots the container, and
  asserts `/health` returns 200. A GHCR push job exists but is gated on tag
  pushes and is currently inert (no registry login configured).
- **`coverage.yml`** — `cargo llvm-cov` (lcov, uploaded as a workflow
  artifact) and a CycloneDX SBOM per crate (also uploaded as an artifact,
  for Phase 6's release attachment).

## MSRV

`rust/Cargo.toml`'s `rust-version` is `1.88`, not the edition-2024 floor of
`1.85`. This was verified empirically, not assumed: building `--locked`
against 1.85.0 fails because `testcontainers`/`testcontainers-modules`
require rustc 1.88, `time`/`time-core`/`time-macros` require 1.88.0,
`etcetera` requires 1.87.0, and `ferroid` requires 1.85.1. Building against
1.88.0 succeeds. `ci.yml`'s `test` job runs a `stable` / `1.88.0` matrix so a
future dependency bump that raises the real floor again fails CI honestly
instead of silently drifting past the declared MSRV.

`rust-toolchain.toml` pins `1.96.1` — intentionally newer than the MSRV. That
file is what CI's `fmt`/`clippy`/`build` jobs and local dev actually build
with; the MSRV matrix leg in `test` exists specifically to catch MSRV
regressions that the day-to-day toolchain wouldn't.

## `history-scan`: honest status, not the result this phase expected

A prior (Phase 1) audit reported a real LLM API key leaked in git history,
on 2 commits reachable from both `main` and `feat/rust-backend`.
`security.yml`'s `gitleaks` job scans the **working tree only** (`--no-git`)
so it guards against new leaks; `history-scan` is a separate job that scans
**full git history** (`gitleaks detect` with no `--no-git`, `fetch-depth: 0`,
all branches fetched) specifically to catch a leak that predates the current
tree.

**What was actually verified, not assumed:** running `gitleaks detect`
(default ruleset, no history-hiding config) against the real full history of
this repo — 141 commits, both locally and in the `history-scan` CI job
itself — reports **no findings**. This was checked multiple ways before
accepting it: default rules across all 141 commits reachable from every ref,
a targeted search of every commit's added lines for JWT-shaped strings,
provider API key prefixes (`sk-`, `gsk_`, `AIzaSy`, etc.), and any file ever
added under a secret/credential/env-like name. None turned up a leaked LLM
key beyond two already-identified synthetic test fixtures (see
`.gitleaks.toml`).

This does **not** prove the Phase 1 finding was wrong — it proves this
specific tool, with its default rules, does not reproduce it against the
history as currently fetched. Plausible explanations that remain open:
the leaked value doesn't match gitleaks' default regex/entropy rules (e.g.
an unusual provider key format), the key was already scrubbed by a rewrite
that predates this clone, or the original finding used a different
method/scope. This is flagged here rather than either (a) quietly declaring
the finding resolved, or (b) hard-coding the job to fail regardless of its
actual result — both would be dishonest in one direction or the other.

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

## Recommended branch protection (cannot be set by this pass — apply manually)

Settings → Branches → Add rule for `main` (and, if PRs into
`feat/rust-backend` become the norm before it merges, that branch too):

- **Require a pull request before merging**, with at least 1 approving
  review; dismiss stale approvals on new commits.
- **Require status checks to pass before merging**, and require branches to
  be up to date. Required checks:
  - `Frontend · Lint · Typecheck · Test · Build` (`verify`, ci.yml)
  - `Parity corpus · no leaked credentials` (ci.yml)
  - `Rust · fmt` (ci.yml)
  - `Rust · clippy` (ci.yml)
  - `Rust · build` (ci.yml)
  - `Rust · test (stable)` and `Rust · test (1.88.0)` (ci.yml, both matrix
    legs)
  - `cargo audit (advisories)` (security.yml)
  - `cargo deny check (all)` (security.yml)
  - `gitleaks (working tree)` (security.yml)
  - `Build lakehouse-api image · smoke test /health` (docker.yml)
  - **Do not** require `history-scan` — see above; its result must not
    silently gate merges either way. Leave it running and visible, not
    blocking.
  - **Do not** require `Dependency review (PR only)` — this org is on
    GitHub's free plan, which doesn't include GitHub Advanced Security, and
    `dependency-review-action` hard-requires it for private repositories
    (verified: the job fails at `actions/dependency-review-action@v4` with
    "Dependency review is not supported on this repository... ensure
    Dependency graph is enabled along with GitHub Advanced Security").
    Getting this job to actually pass requires either upgrading the org's
    billing plan to include GHAS, or making the repository public — both
    are decisions outside this CI pass's scope. Until one of those happens,
    this job stays present (so the gap doesn't get silently forgotten) and
    permanently excluded from required checks.
- **Do not allow force pushes** to the protected branch.
- **Do not allow deletions** of the protected branch.
- Consider **requiring signed commits** and **requiring linear history**
  once the team's workflow is settled; neither is load-bearing for this
  phase.
- Restrict who can push directly (no direct pushes bypassing PRs), including
  for repo admins if the team wants that strict a guarantee.

This has to be done in the GitHub UI (or via `gh api
repos/:owner/:repo/branches/main/protection`) by someone with admin rights on
the repo — it is not something a workflow file can configure for itself.
