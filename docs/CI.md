# CI/CD

This repo's CI is split into four workflows under `.github/workflows/`:

- **`ci.yml`** — fast-feedback correctness: frontend lint/typecheck/test/build,
  the parity-corpus secret-shape check, and Rust `fmt` / `clippy` / `build` /
  `test` (the last as a matrix over `stable` and the declared MSRV).
- **`security.yml`** — `cargo audit`, `cargo deny check all`, a working-tree
  `gitleaks` scan, GitHub's `dependency-review-action` on PRs, and a
  full-git-history `history-scan` job, which is currently **RED for a real
  reason** (see below) and is deliberately excluded from required checks.
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
  -f 'required_status_checks[checks][][context]=Frontend · Lint · Typecheck · Test · Build' \
  -f 'required_status_checks[checks][][context]=Parity corpus · no leaked credentials' \
  -f 'required_status_checks[checks][][context]=Rust · fmt' \
  -f 'required_status_checks[checks][][context]=Rust · clippy' \
  -f 'required_status_checks[checks][][context]=Rust · build' \
  -f 'required_status_checks[checks][][context]=Rust · test (stable)' \
  -f 'required_status_checks[checks][][context]=Rust · test (1.88.0)' \
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

The check names above are the real `name:` values from
`.github/workflows/{ci,security,docker}.yml` as of this writing — not
guessed. If a workflow's job names change, this list needs to be updated to
match, or `strict` mode will block merges on a check that no longer reports.

Why each choice:

- **Required checks are the honest-green ones only.** `Frontend...Build`,
  `Parity corpus...`, all `Rust ·` jobs, `cargo audit`, `cargo deny`,
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