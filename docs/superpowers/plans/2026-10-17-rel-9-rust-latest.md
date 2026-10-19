# REL-9: the Rust workspace on the latest stable Rust (1.99.1)

Spec: `docs/core/specs/rel-9.md`. Owner decisions D1–D3 there (2026-10-17).
Base: `main` at `383010b1`. Branch: `chore/rel-9-rust-1-99`. Worktree:
`/home/shiro/rantai/wt/rel-9`. Developer rules:
`/home/shiro/rantai/review-logs/dev-rules.md`.

## 1. Goal

Toolchain pin and declared minimum move to 1.99.1 / 1.99, and the code is
shown safe on that compiler. Behaviour does not change. This unblocks
`SEC-41` (sqlx 0.9 needs 1.94), which unblocks `SEC-14` part B (#192).

## 2. Decisions

- D1: `rust/rust-toolchain.toml` `channel = "1.99.1"`; `rust/Cargo.toml`
  `rust-version = "1.99"`; the CI MSRV job checks `1.99.0`.
- D2: fix warnings, don't allow them. A new `#[allow]` or `#[expect]`
  needs `reason = "…"` that a reader can check, and the plan's handoff
  lists every one. No workspace-level `[lints]` relaxations.
- D3: no crate upgrades (`cargo update` is out of scope). `Cargo.lock`
  changes only if 1.99's cargo rewrites the lock format by itself; if so,
  say so in the commit and show the diff is format-only.

## 3. Anchors (verified at `383010b1`)

| What | Where |
| --- | --- |
| Toolchain pin | `rust/rust-toolchain.toml:8` (`channel = "1.96.1"`), comment lines 1–7 |
| Declared minimum | `rust/Cargo.toml:12` (`rust-version = "1.88"`), comment lines 9–11 |
| API image | `rust/Dockerfile:23` (`FROM rust:1.96.1-slim AS tools`), comments ~10–12 and ~97 |
| G1 test runner image | `docker-compose.yml:1341` (`image: rust:1.96.1-slim`) |
| MSRV job | `.github/workflows/ci.yml:347–375` (name, `toolchain: "1.88.0"`, cache key `msrv-1.88.0`) |
| Required-gate comment | `scripts/ci/required_gate.sh:24` |
| Docs | `README.md:201–207`, `docs/CI.md:143–154`, `docs/OPERATIONS.md:19–24` |
| History, leave alone | `CHANGELOG.md` old entries, `docs/adr/*`, `docs/plans/*-RESULT.md` |

Other workflows use `dtolnay/rust-toolchain@stable`, and `rust-toolchain.toml`
overrides that inside `rust/`; check that this is still true (§7).

## 4. Tasks (one commit each, in order)

**T1. Release-note audit (plan only, no code).** Read the "Compatibility
Notes" (and "Language", "Compiler", "Libraries", "Cargo") sections of the
Rust 1.97.0, 1.98.0 and 1.99.0 release notes (and 1.99.1's). Append a
table to §8 under "T1 audit": one row per note, with either "affects us
at file:line, fixed in T3" or "does not affect us, because …" (grep
evidence). Also list new default-on rustc lints and new clippy lints in
`clippy::all`/`pedantic` if the workspace enables pedantic. Check the
workspace `[lints]` in `rust/Cargo.toml` to see which groups are on.
Acceptance: every note has a row, and each row cites a source URL or the
release-notes section.

**T2. Bump the numbers.** These files: `rust-toolchain.toml`,
`Cargo.toml` `rust-version`, `rust/Dockerfile`, `docker-compose.yml:1341`,
the `ci.yml` MSRV job (name, toolchain, cache key) and
`required_gate.sh:24`. Rewrite each "why" comment so it holds for the new
numbers. The old comments explained why the minimum was 1.88. The new
reason is the owner's decision (spec D1), and `SEC-41` needs at least 1.94.
Acceptance: `rustup run 1.99.1 cargo --version`; `cargo fmt --check`.
`rg -n '1\.96|1\.88'` over the non-history files in §3 returns nothing.

**T3. Fix what 1.99 reports.** Run `cargo clippy -j 4 --workspace
--all-targets --all-features -- -D warnings` on 1.99.1 and fix every
warning and error at its site. Fix the "affects us" rows from T1 here
too. If there are many, split into one commit per crate. Each fix cites
`REL-9` in the commit body. Acceptance: clippy exit 0. List each
`allow`/`expect` you added, with its reason, in the handoff.

**T4. Docs and changelog.** Update `README.md`, `docs/CI.md` and
`docs/OPERATIONS.md` (§3 lines) to the new numbers and the reason. Add a
`CHANGELOG.md` bullet under `## [Unreleased]` in the file's style.
Acceptance: `rg -n '1\.96|1\.88'` over the repo shows only history files.
Paste the output into the handoff.

## 5. Verification before handoff (foreground or detached-and-polled, on the final commit)

Disk first: `df -h /`. Stop below 40 GB free and tell the planner. A
new compiler rebuilds everything in the shared target. When no cargo is
running, `rm -rf /home/shiro/rantai/cargo-target/debug/incremental` is
safe.

```
cd rust && cargo fmt --check
cargo clippy -j 4 --workspace --all-targets --all-features -- -D warnings
cargo deny check
RUST_TEST_THREADS=2 cargo test -j 4 --workspace --no-fail-fast   # under sg docker, detached, polled
rustup run 1.99.0 cargo check -j 4 --workspace --locked          # what the MSRV job runs
docker build -f rust/Dockerfile -t lakehouse-api:rel9 .           # then the /health smoke as CI does it
docker compose --profile '*' config --quiet
```

Any test failure: rerun that binary alone with `RUST_TEST_THREADS=1`
before calling it a regression, and quote both runs. The known `.env`
failure (`state::tests::connector_secret_resolver_admits_…`) is not
yours.

## 6. Pull request

One PR, opened by the planner: `chore(rust): toolchain and minimum to
1.99.1 (REL-9)`.

## 7. Verify, do not assume

- Make sure the `@stable` workflows really build with the pinned
  toolchain in `rust/`: a CI log line showing `rustc 1.99.1` in a job
  that uses `@stable` is enough after the PR runs. The planner checks
  this on the PR.
- If 1.99 needs a crate upgrade to compile (a dependency that breaks on
  the new compiler), stop and report: crate upgrades are out of scope.

## 8. Handoff

(developer appends here)

## 9. Review

(planner)
