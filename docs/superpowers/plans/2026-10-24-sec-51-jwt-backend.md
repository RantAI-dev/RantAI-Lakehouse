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

(developer appends here)

## Review

(planner appends here)
