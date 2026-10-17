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

### T1 audit: Rust 1.97.0–1.99.1 release notes and lints

Workspace lints in `rust/Cargo.toml`:
- `[workspace.lints.rust]`: `future-incompatible = "warn"` (deprecated alias for `future_incompatible`), `nonstandard_style = { level = "deny", priority = -1 }`, `missing_docs = "warn"`.
- `[workspace.lints.clippy]`: `all = { level = "deny", priority = -2 }`, `pedantic = { level = "warn", priority = -1 }`, plus specific denies for `redundant_clone`, `needless_collect`, `large_enum_variant`, `unwrap_used`, `expect_used`.

#### Release notes compatibility audit table

| Release | Section | Note / Change | Assessment & Grep Evidence |
| --- | --- | --- | --- |
| 1.97.0 | Compiler / Compatibility Notes | Symbol mangling `v0` enabled by default (`-Csymbol-mangling-version=v0`); legacy mangling restricted | Does not affect us: repo does not inspect mangled symbols or require legacy mangling. `grep -rn "symbol-mangling" rust/` -> empty. (Source: https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/) |
| 1.97.0 | Compatibility Notes | Deref coercions disallowed in `pin!` macro | Does not affect us: `pin!` is not used anywhere in workspace. `grep -rn 'pin!' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Compatibility Notes | Future-incompatibility warning for `{float}` inference (`f32: From<{float}>`) | Does not affect us: floating point usages are explicit (`f64`). (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Compatibility Notes | Pattern matching & structural equality changes (disallow const pattern matching conflicting with manual PartialEq) | Does not affect us: no manual `PartialEq` impls paired with const pattern matches. `grep -rn 'impl.*PartialEq for' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Compatibility Notes | `#[non_exhaustive]` enum discriminant reading on match | Does not affect us: no closures capture single-variant non-exhaustive enums across match arms. (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Compatibility Notes | Match ergonomics 2024 `mut ref` and `mut ref mut` in struct pattern field shorthand feature-gated | Does not affect us: no `mut ref` patterns exist. `grep -rn 'mut ref ' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Compatibility Notes / Libraries | `derive(PartialOrd)` fast path with `Ord` | Does not affect us: no manual `Ord` impls exist in workspace. `grep -rn 'impl.*Ord for' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Compatibility Notes | `semicolon_in_expressions_from_non_local_macros` lint emitted on non-local macros | Does not affect us: macro invocations in expression position are properly structured. (Source: https://github.com/rust-lang/rust/releases/tag/1.97.0) |
| 1.97.0 | Platform Support | `nvptx64-nvidia-cuda` dropped support for older architectures | Does not affect us: workspace builds for `x86_64-unknown-linux-gnu` / `x86_64-unknown-linux-musl`. (Source: https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/) |
| 1.97.0 / 1.98.0 | Cargo | `lints.rust.future-incompatible` deprecated in favor of `lints.rust.future_incompatible` | Affects us at `rust/Cargo.toml:79` and all workspace crate `Cargo.toml` files; fixed in T2/T3. (Source: Cargo 1.98 manifest diagnostic) |
| 1.98.0 | Compatibility Notes | Trait object lifetime bounds resolution when fully elided | Does not affect us: trait object lifetime bounds are explicit (`dyn ... + Send + Sync + 'static`). (Source: https://github.com/rust-lang/rust/releases/tag/1.98.0) |
| 1.98.0 | Compatibility Notes | Ambiguous glob imports converted to hard errors (`ambiguous_glob_imports`) | Does not affect us: glob imports are limited to test modules (`use super::*;`) without name collisions. `grep -rn 'use .*::\*;' rust/` checked. (Source: https://github.com/rust-lang/rust/releases/tag/1.98.0) |
| 1.98.0 | Compatibility Notes | Equality where-bounds syntax deprecation (`Type = Type`, `Type == Type` no longer permitted) | Does not affect us: no equality syntax in where-clauses. `grep -rn 'where.*==' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.98.0) |
| 1.98.0 | Compatibility Notes | Transmute size checking with `#[repr(...)]` | Does not affect us: `std::mem::transmute` is not used in the codebase. `grep -rn 'transmute' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.98.0) |
| 1.98.0 | Compatibility Notes / Libraries | Windows TLS destructors switched to Fiber Local Storage (FLS) | Does not affect us: target is Linux x86_64. (Source: https://github.com/rust-lang/rust/releases/tag/1.98.0) |
| 1.98.0 | Compatibility Notes / Compiler | Deny-by-default `invalid_runtime_symbol_definitions` lint | Does not affect us: workspace does not define runtime library symbols. (Source: https://github.com/rust-lang/rust/releases/tag/1.98.0) |
| 1.99.0 | Compatibility Notes / Libraries | Recommendation against round-trip unleaking after `Box::leak` | Does not affect us: `lakehouse-test-support/src/lib.rs:69,109` leaks tokio runtime & Postgres testcontainer for process lifetime and never calls `Box::from_raw`. `grep -rn 'Box::from_raw' rust/` -> empty. (Source: https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/) |
| 1.99.0 | Compatibility Notes | Corrected layout of `#[repr(Int)]` enums with uninhabited ZST | Does not affect us: no `#[repr(Int)]` enums in workspace. `grep -rn '#\[repr(' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.99.0) |
| 1.99.0 | Compatibility Notes | Prevent unsize-coercing into `Pin<Foo>` where `Foo: !Deref` | Does not affect us: `Pin` unsize-coercions are not used. (Source: https://github.com/rust-lang/rust/releases/tag/1.99.0) |
| 1.99.0 | Compatibility Notes | Fast-path for `derive(PartialOrd)` when deriving `Ord` | Does not affect us: no manual `Ord` implementations exist in workspace. `grep -rn 'impl.*Ord for' rust/` -> empty. (Source: https://github.com/rust-lang/rust/releases/tag/1.99.0) |
| 1.99.1 | Regression / Compatibility | LLVM loop-idiom `wcslen` missing symbol on targets without standard C library | Does not affect us: targets Linux glibc/musl, and 1.99.1 includes the compiler-builtins fallback. (Source: https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/) |
| 1.99.1 | Regression / Compatibility | FFI `bool` zero-extension fix on x86_64 | Does not affect us: no custom extern "C" functions returning bool. (Source: https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/) |
| 1.99.1 | Regression / Compatibility | `rust-lld` on macOS rpath fix | Does not affect us: builds and runs on Linux x86_64. (Source: https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/) |

#### New default-on rustc and clippy lints affecting the workspace

Since `rust/Cargo.toml` enables `clippy::all` (deny) and `clippy::pedantic` (warn), the following new lints triggered under 1.99.1:
1. `clippy::question_mark` (`clippy::all`):
   - Affects `crates/lakehouse-api/src/connector_probe.rs:1426:12` (block can use `?`). Fixed in T3.
2. `clippy::result_large_err` (`clippy::all`):
   - Affects `crates/lakehouse-api/src/routes/ai/mod.rs:431:6` (Err variant `Response` is large). Fixed in T3.
   - Affects `crates/lakehouse-api/src/routes/pipelines.rs:2814:64` and `3715:6` (Err variant `Response` is large). Fixed in T3.
3. `clippy::chunks_exact_to_as_chunks` (`clippy::all`):
   - Affects `crates/lakehouse-api/src/upload_parse.rs:203:18` (constant chunk size 2). Fixed in T3.
4. `clippy::assert_is_empty` (`clippy::pedantic`):
   - Affects `assert!(x.is_empty())` test assertions across `crates/lakehouse-api`, `crates/lakehouse-dagster`, `crates/lakehouse-clickhouse`, `crates/lakehouse-store`. Fixed in T3.
5. `clippy::unused_async_trait_impl` (`clippy::pedantic`):
   - Affects `crates/lakehouse-api/src/rustfs_client.rs:213:9` (async trait fn with no `.await`). Fixed in T3.
6. Cargo `future_incompatible` key rename warning:
   - Affects `rust/Cargo.toml` and crate manifests (`future-incompatible` -> `future_incompatible`). Fixed in T2/T3.


## 9. Review

(planner)
