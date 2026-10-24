# SEC-51: the JWT library's crypto backend no longer pulls in `crypto-bigint` 0.5

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-24 |
| Backlog | `SEC-51` (P1), Administration & Security |
| Base | `origin/main` at `c3468ed5` (Rust 1.99.1 since `REL-9`); anchors read there |
| Branch | `fix/sec-51-jwt-backend`, worktree `/home/shiro/rantai/wt-sec51` |

## Why

`cargo audit` and `cargo deny` are red on `main` since 2026-10-24:
`RUSTSEC-2026-0387`, timing variability in `crypto-bigint` 0.5.5, patched
only in 0.7.6. It reaches us through `jsonwebtoken` 11.1.0's `rust_crypto`
backend (`p256`/`p384` 0.13 -> `ecdsa` -> `elliptic-curve` 0.13, which pins
`crypto-bigint` ^0.5), so no bump fixes it. We sign with EC private keys
(`rust/crates/lakehouse-api/src/connector_probe/saas.rs`,
`EncodingKey::from_ec_pem`), so an ignore is not acceptable. While the
checks are red, no pull request that touches dependencies may merge.

In `jsonwebtoken` 11.1.0, `p256`, `p384` (and `rsa`) are optional and come
only with the `rust_crypto` feature; `aws_lc_rs` uses `aws-lc-rs`, which is
already compiled into this workspace (`object_store`, `rustls`).

## Decisions (planner)

1. `rust/Cargo.toml`: `jsonwebtoken` features become
   `["aws_lc_rs", "use_pem"]`. Rewrite the comment above it: why
   `aws_lc_rs` now (this advisory, no new C dependency because `aws-lc-rs`
   is already built), and that exactly one backend must be selected.
   Update the comment in `rust/crates/lakehouse-api/Cargo.toml` (around
   line 36) that says `rust_crypto`.
2. `cargo update` only what the feature change needs (`cargo update -p
   jsonwebtoken --precise 11.1.0` or simply build: the lockfile drops the
   unused crates). No other crate moves. The diff of `Cargo.lock` must show
   only removals (`p256`, `p384`, `ecdsa`, `elliptic-curve`,
   `crypto-bigint` 0.5, and whatever only they pulled in). If any crate is
   added or upgraded, stop and report.
3. `crypto-bigint` must be gone from `Cargo.lock` entirely
   (`grep -c 'name = "crypto-bigint"' rust/Cargo.lock` = 0). `rsa` stays
   (also used by `sqlx-mysql`), so the `RUSTSEC-2023-0071` ignores in
   `rust/deny.toml` and `.github/workflows/security.yml` stay.
4. Behaviour must not change: RS256/ES256 signing in `saas.rs` and the
   GCP secret manager, and OIDC verification (`lakehouse-auth`), work
   exactly as before. If `aws_lc_rs` refuses a key format or algorithm we
   use, stop and report.

## Tasks

- **T1.** Decisions 1-3, one commit. Check: the `Cargo.lock` diff is only
  removals (quote `git diff --stat` and the removed package names);
  `grep` from decision 3 is 0; `cargo deny check advisories` (or
  `cargo audit` with the workflow's ignore list) passes locally if the tool
  is installed, else *not verified*.
- **T2.** Proof. `cargo test -p lakehouse-auth` (OIDC sign/verify),
  `cargo test -p lakehouse-api` filtered to `saas`, `secret_manager` and
  `oidc`, then the workspace clippy and `cargo test --workspace`. A
  `CHANGELOG.md` line. Mark nothing Done in `BACKLOG.md`.

## Out of scope

Any other dependency bump, `SEC-41` (sqlx), removing `rsa`.

## Handoff

### Developer: T1 committed, T2 blocked by the disk guard

Worktree `/home/shiro/rantai/wt-sec51`, branch `fix/sec-51-jwt-backend`,
base `origin/main` at `c3468ed5`. The two edits the previous (lost) session
left uncommitted in `rust/Cargo.toml` and
`rust/crates/lakehouse-api/Cargo.toml` were kept as they were.

**T1 — commit `3cb827a`** (`build(deps): switch jsonwebtoken to the
aws-lc-rs backend (SEC-51)`; decisions 1-3):

- `cargo update -p jsonwebtoken --precise 11.1.0` (no compile) wrote the
  new lockfile.
- `git diff --stat rust/Cargo.lock`: `1 file changed, 10 insertions(+),
  198 deletions(-)`.
- Removed packages (16): `base16ct`, `crypto-bigint` 0.5.5,
  `curve25519-dalek`, `curve25519-dalek-derive`, `ecdsa`, `ed25519`,
  `ed25519-dalek`, `elliptic-curve`, `ff`, `fiat-crypto`, `group`,
  `p256`, `p384`, `primeorder`, `rfc6979`, `sec1`.
- `grep -c 'name = "crypto-bigint"' rust/Cargo.lock` -> `0`.
- `cargo deny check advisories` -> `advisories ok`, exit 0. It also prints
  `warning[advisory-not-detected]` for the existing `RUSTSEC-2026-0381`
  ignore at `rust/deny.toml:74`; that is the config's own warning, not this
  diff (nothing in `deny.toml` changed).
- `cargo fmt --check` -> OK.
- `/home/shiro/rantai/bin/cargo-locked check -j 2 -p lakehouse-auth
  -p lakehouse-api --all-targets` -> exit 0, `Finished ... in 3m 48s`.

**Decision 2 mismatch — needs the planner.** The lock diff is not
removals-only. `aws-lc-rs`'s default features (`ring-io`,
`ring-sig-verify`) became active with `jsonwebtoken`'s `aws_lc_rs` feature
(upstream enables `aws-lc-rs` with its defaults and offers no switch), so
the lock gains exactly one package entry:

```text
[[package]]
name = "untrusted"
version = "0.7.1"
```

no dependencies, 4 lines. No crate changes version; the rest of the diff is
dependency-edge movement (`jsonwebtoken`, `aws-lc-rs`, `generic-array`
dropping `zeroize`) or the `untrusted 0.9.0` disambiguation for
`ring`/`rustls`. Avoiding it would need a `[patch]` or an upstream
`default-features = false`, both outside this plan; I did not edit decision
2 and did not add anything else.

**T2 — what ran before the stop:**

- `/home/shiro/rantai/bin/cargo-locked test -j 2 -p lakehouse-auth` ->
  exit 0, 8 test binaries, **133 passed, 0 failed, 1 ignored** (the 63 s
  binary is `oidc.rs`'s wiremock `/jwks` + `/token` round trip).
- `cargo test -j 2 -p lakehouse-api saas` was still compiling
  `lakehouse-api` when the stop happened; no test from it ran.

**Not verified, and why:** the `lakehouse-api` runs filtered to `saas`,
`secret_manager` and `oidc`, the workspace clippy
(`-j 2 --workspace --all-targets --all-features -- -D warnings`) and
`cargo test -j 2 --workspace` were not run. `df -h /` fell from 129 GB
free at the start to 2.3 GB free (100 % used) during the session, while a
concurrent full review run was using the same machine; the machine rule
stops work below 15 GB free, so I terminated my own background chain and
stopped. I did not touch any other worktree, process, container, image or
volume, and ran no `cargo clean`. The T2 `CHANGELOG.md` line is written in
the worktree but **uncommitted** for the same reason: it sits first under
`## [Unreleased]` / `### Security`, says the lock loses 16 package entries
and gains `untrusted` 0.7.1, and names the algorithms the existing tests
cover.

Restart once free space is back: run the three `lakehouse-api` filters,
then the workspace clippy and `cargo test --workspace`, then commit the
`CHANGELOG.md` line as T2. `BACKLOG.md` was not touched.

## Review

(planner appends here)

### Review round 1 (planner, 2026-10-24)

The decision-2 mismatch is accepted: the one added entry, `untrusted`
0.7.1, comes from `aws-lc-rs`'s default features that `jsonwebtoken`
enables and offers no switch for; it has no dependencies, its licence is
ISC (allowed in `rust/deny.toml`), and no crate changes version. The other
change in `Cargo.lock` is sixteen removals, `crypto-bigint` among them
(`grep` = 0). `rsa` stays, so the `RUSTSEC-2023-0071` ignores stay. No
`BLOCKER`. The developer's T2 proof stopped at the disk guard; the
planner's full run below replaces it.
