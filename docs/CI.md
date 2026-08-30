# CI/CD

This repo's CI is split into four workflows under `.github/workflows/`:

- **`ci.yml`** — fast-feedback correctness: frontend lint/typecheck/test/build,
  the parity-corpus secret-shape check, and Rust `fmt` / `clippy` / `build` /
  `test` (the last as a matrix over `stable` and the declared MSRV).
- **`security.yml`** — `cargo audit`, `cargo deny check all`, a working-tree
  `gitleaks` scan, GitHub's `dependency-review-action` on PRs, and a
  deliberately-red `history-scan` job (see below).
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

## `history-scan`: known-red by design

A Phase 1 audit found a real LLM API key leaked in git history, on commits
reachable from both `main` and `feat/rust-backend`. `security.yml`'s
`gitleaks` job scans the **working tree only** (`--no-git`) so it guards
against new leaks; `history-scan` is a separate job that scans full history
and is **expected to fail**.

It is a separate job — not `continue-on-error` bolted onto a trusted job —
specifically so the redness stays visible in the Actions UI instead of being
swallowed. Resolving it for real requires:

1. Rotating the exposed key at the provider.
2. Rewriting history (`git filter-repo` or BFG) to purge the blob from every
   affected commit.
3. Force-pushing every affected ref, which invalidates every existing clone
   and any open PR based on the old history.

That is a destructive, cross-cutting, and irreversible-for-clones operation.
It is deliberately not done as a side effect of a CI-hardening pass — it
needs a human to explicitly decide to take it on, coordinate with anyone
with a local clone, and execute it. Until then, `history-scan` stays red as
an honest record that the debt exists, and it is **excluded from required
status checks** (see below) so it doesn't block merges while remaining
visible on every run.

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
  - `Dependency review (PR only)` (security.yml, PR-triggered)
  - `Build lakehouse-api image · smoke test /health` (docker.yml)
  - **Do not** require `gitleaks (full git history) — KNOWN RED` — see
    above. Leave it running and visible, just not blocking.
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
