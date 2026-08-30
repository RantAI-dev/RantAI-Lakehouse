# Contributing to RantAI Lakehouse

Thanks for your interest in contributing. This document covers dev setup,
how to run tests, commit conventions, and what to expect from a PR review.

## Project layout

- `src/` — Next.js 16 / React 19 / Tailwind v4 frontend (Bun runtime).
- `rust/` — Rust (axum) backend workspace, `rust/crates/lakehouse-*`
  (11 crates). See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the
  module map and request lifecycle.

## Prerequisites

- [Bun](https://bun.sh) `>= 1.3.0`
- Rust toolchain matching `rust/rust-toolchain.toml` (installed
  automatically by `rustup` when you run any `cargo` command inside `rust/`)
- Docker (for Postgres/ClickHouse/Dagster locally — see
  [README.md](README.md#quickstart) for the full stack)

## Dev setup

```bash
# Frontend
bun install

# Backend
cd rust
cargo build
```

See [README.md](README.md) for environment variables and how to bring up
the full stack (Postgres, ClickHouse, Dagster) with Docker.

## Running tests

Frontend:

```bash
bun run typecheck   # tsc --noEmit
bun run lint        # eslint
bun run test         # bun test src/lib
bun --bun next build # production build
```

Backend (from `rust/`):

```bash
cargo test --all-features
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo deny check licenses
```

`cargo fmt --check` and `cargo clippy -- -D warnings` are **enforced in CI**
— a PR that fails either will not be merged. Please run them locally before
pushing. `cargo deny check licenses` guards the dependency license set
(permissive-only, see [rust/deny.toml](rust/deny.toml)); a new dependency
that pulls in a copyleft license will fail this check, by design.

## Commit convention

This repository uses [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<optional scope>): <description>

[optional body]

[optional footer(s)]
```

Common types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `chore`,
`ci`, `build`, `security`. A breaking change is marked with `!` after the
type/scope (`feat!: ...`) or a `BREAKING CHANGE:` footer.

Examples from this repo's own history:

```
fix(auth): permission-gate /api/identity/* instead of auth-only (D1)
feat(alerts): threshold alerts + scheduled digests (webhook + email)
feat!: cutover to Rust backend, delete TypeScript API routes
```

## Pull request expectations

- Keep PRs focused — one logical change per PR is easier to review than a
  bundle of unrelated fixes.
- Include tests for new behavior and bug fixes where practical.
- Update relevant docs (`README.md`, `docs/ARCHITECTURE.md`,
  `CHANGELOG.md`'s `[Unreleased]` section) alongside the code change.
- Make sure the verification gate above passes locally before requesting
  review — CI runs the same checks and will block merge on failure.
- Describe *why* the change is needed, not just what changed; link related
  issues.
- Do not weaken a lint, skip a test, or lower a threshold to make CI pass —
  fix the underlying issue instead.

## Reporting security issues

Please do **not** open a public issue for security vulnerabilities — see
[SECURITY.md](SECURITY.md) for the disclosure process.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By
participating, you're expected to uphold it.
