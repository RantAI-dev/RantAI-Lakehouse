# `REL-9` The Rust workspace builds on the latest stable Rust

| | |
| --- | --- |
| Backlog | `REL-9` in [BACKLOG.md](../BACKLOG.md) |
| Module | Delivery |
| Size | M |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The workspace pins Rust 1.96.1 (`rust/rust-toolchain.toml`) and promises
to compile on 1.88 (`rust-version` in `rust/Cargo.toml`, the CI job
`Rust · MSRV check (1.88.0)`). `sqlx` 0.9, which `SEC-41` needs, declares
1.94.0 as its minimum, so the 1.88 promise cannot hold. Rather than move
the floor to whatever one dependency needs, the product owner chose to
move both numbers to the latest stable release and to make proving that
safe its own item.

## Decisions (product owner, 2026-10-17)

| # | Decision |
| --- | --- |
| D1 | The toolchain pin and the declared minimum both move to the latest stable Rust: 1.99.1 at the time of writing (released 2026-10-13). `rust-version = "1.99"`; the MSRV job checks 1.99.0. |
| D2 | Moving is not "change the number": the item proves the code is safe on the new compiler. Every compatibility note in the 1.97, 1.98 and 1.99 release notes is checked against this code, and every new warning or lint is fixed, not allowed, unless an allow carries a checkable reason. |
| D3 | The security planner owns it, at P1, before `SEC-41`. |

## Target

| Capability | Today | Target |
| --- | --- | --- |
| Toolchain used by CI, local builds and the API image | 1.96.1 | 1.99.1 everywhere: `rust-toolchain.toml`, `rust/Dockerfile`, the compose test-runner image, docs |
| Declared minimum and its CI check | 1.88 | 1.99, checked by the MSRV job on 1.99.0 |
| Warnings | `clippy -D warnings` clean on 1.96.1 | Clean on 1.99.1, with no new blanket allow |
| Behaviour | — | Unchanged: the full verification block and every CI gate green |

## Acceptance checklist

- `REL-9-AC1` No file in the repository names 1.96 or 1.88 as the toolchain or minimum any more (grep quoted in the PR), except history (`CHANGELOG.md`, ADRs, `docs/plans/*-RESULT.md`).
- `REL-9-AC2` The plan lists each compatibility note from the 1.97, 1.98 and 1.99 release notes with "affects us at file:line" or "does not affect us, because …".
- `REL-9-AC3` The full Rust verification block passes on 1.99.1, and every CI check that passes on `main` passes on the PR.
- `REL-9-AC4` The API image builds and passes its `/health` smoke test on `rust:1.99.1-slim`.

## Not included

Upgrading crates (that is `SEC-41` for `sqlx`, and `cargo update` is out of
scope here). Changing the edition. Nightly features.
