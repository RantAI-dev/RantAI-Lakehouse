# SEC-42: the `zip` 8.6.0 advisories, ignored with a checked reason

Backlog: `SEC-42` (Task, P1). Base: `main` at the merge of #195. Branch:
`fix/sec-42-zip-advisories`. Worktree: `/home/shiro/rantai/wt/sec-42`.
Developer rules: `/home/shiro/rantai/review-logs/dev-rules.md`.

## 1. Facts (verified by the planner, 2026-10-17)

- `cargo audit` and `cargo deny check` are red on `main` and every PR:
  `zip` 8.6.0 has `RUSTSEC-2026-0367` (SUID/SGID bits kept on extract),
  `-0368` (`ZipArchive::extract` writes symlinks outside the destination),
  `-0369` (`extract` applies permissions through symlinks). All are fixed
  in `zip` 9.0.0 and all are in `extract`, which writes an archive to disk.
- `zip` arrives only through `calamine` 0.36.1 (`cargo tree -i zip@8.6.0`:
  `calamine` <- `lakehouse-api`). The newest `calamine` (0.36.1) requires
  `zip ^8.6`; no release accepts `zip` 9.
- `calamine` 0.36.1's source never calls `extract` (it reads entries with
  `by_name`); the workspace has no direct `zip` dependency and no
  `extract` call.

## 2. Decision

Ignore the three IDs, in both lists, each with the reason above and the
condition for removing it ("drop when `calamine` accepts `zip` >= 9.0.0;
then `cargo update -p zip`"). No crate changes. Do not fork or patch
`calamine`.

## 3. Tasks (one commit each)

**T1.** `rust/deny.toml` `[advisories]`: add the three IDs to `ignore`
with a comment block in the file's existing style (what, why not
reachable, `cargo tree` path, when to remove). Acceptance: `cargo deny
check advisories` passes locally (no `cargo build` needed).

**T2.** `.github/workflows/security.yml`: the `cargo audit` step adds
`--ignore` for the three IDs, with a comment pointing to `rust/deny.toml`
as the existing comment does. Acceptance: `cargo audit --ignore
RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0367 --ignore RUSTSEC-2026-0368
--ignore RUSTSEC-2026-0369` exits 0 in `rust/` (install `cargo-audit` if
missing, outside the heavy lock is fine: it does not compile the
workspace).

**T3.** `CHANGELOG.md` bullet under `## [Unreleased]` in the file's style,
and re-verify the `calamine` facts in §1 yourself (quote the `grep`
output for `extract` in its source and the `cargo tree -i` output) in the
handoff.

## 4. Handoff

### Handoff (2026-10-17, round 1)

#### 1. Commits
- `61c10810d869` docs(changelog): document SEC-42 zip advisories exception
- `9a41d644bb39` ci(security): ignore unreachable zip advisories in cargo audit (SEC-42)
- `e297bd7d216f` fix(security): ignore unreachable zip advisories in deny.toml (SEC-42)
- `4f7041f354e2` docs(plan): SEC-42 zip advisories

#### 2. Re-check of calamine facts (§1)
- `cargo tree --manifest-path rust/Cargo.toml -i zip`:
  ```
  zip v8.6.0
  └── calamine v0.36.1
      └── lakehouse-api v0.1.0 (/home/shiro/rantai/wt/sec-42/rust/crates/lakehouse-api)
  ```
  (exit code 0; confirms `zip` 8.6.0 is only introduced transitively through `calamine` 0.36.1 via `lakehouse-api`).
- `calamine` 0.36.1 `Cargo.toml` dependency specification:
  ```toml
  [dependencies.zip]
  version = "8.6"
  features = ["deflate"]
  ```
  (requires `zip ^8.6`, incompatible with `zip` 9.0.0; no newer calamine release exists).
- `grep -rn "extract" ~/.cargo/registry/src/.../calamine-0.36.1/src/`:
  ```
  calamine-0.36.1/src/attrs.rs:5://! Zero-allocation XML attribute extraction utilities.
  calamine-0.36.1/src/lib.rs:1798:/// tables have been loaded the following methods can be used to extract and
  calamine-0.36.1/src/xls.rs:1127:/// Decode XF (extract only ifmt - Format identifier)
  calamine-0.36.1/src/xlsx/cells_reader.rs:552:/// Reads a cell value using pre-extracted `s` and `t` attributes
  calamine-0.36.1/src/xlsx/cells_reader.rs:623:/// Parse a `<v>` cell value from raw bytes with pre-extracted
  calamine-0.36.1/src/xlsx/mod.rs:1489:    /// the following methods can be used to extract and work with individual
  ```
  (exit code 0; zero calls to `extract`; archive entries are read in-memory using `zip.by_name(...)`).
- Workspace scan:
  No direct dependency on `zip` (`grep -rn "zip" rust/Cargo.toml rust/crates/*/Cargo.toml` shows only a comment in `lakehouse-api/Cargo.toml`). No archive extraction calls in workspace crates (`extract` matches are axum extractors, SQL `extract(epoch ...)`, or pipeline step keys).

#### 3. Verification commands and real outputs
- `cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check advisories`
  Exit code: 0
  Output:
  ```
  warning[yanked]: detected yanked crate (try `cargo update -p chacha20`)
     ┌─ /home/shiro/rantai/wt/sec-42/rust/Cargo.lock:70:1
     │
  70 │ chacha20 0.10.1 registry+https://github.com/rust-lang/crates.io-index
     │ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ yanked version
     │
     ├ chacha20 v0.10.1
       └── rand v0.10.3
           ├── ferroid v2.0.0
           │   └── testcontainers v0.27.3
           │       ├── (dev) lakehouse-api v0.1.0
           │       ├── lakehouse-test-support v0.1.0
           │       │   ├── (dev) lakehouse-api v0.1.0 (*)
           │       │   ├── (dev) lakehouse-auth v0.1.0
           │       │   │   └── lakehouse-api v0.1.0 (*)
           │       │   └── (dev) lakehouse-store v0.1.0
           │       │       ├── lakehouse-api v0.1.0 (*)
           │       │       └── lakehouse-auth v0.1.0 (*)
           │       └── testcontainers-modules v0.15.0
           │           ├── (dev) lakehouse-api v0.1.0 (*)
           │           └── lakehouse-test-support v0.1.0 (*)
           ├── lakehouse-alerts v0.1.0
           │   └── lakehouse-api v0.1.0 (*)
           ├── lakehouse-api v0.1.0 (*)
           ├── lakehouse-auth v0.1.0 (*)
           ├── lakehouse-bi v0.1.0
           │   ├── lakehouse-alerts v0.1.0 (*)
           │   └── lakehouse-api v0.1.0 (*)
           └── object_store v0.14.2
               ├── lakehouse-api v0.1.0 (*)
               └── lakehouse-iceberg v0.1.0
                   └── lakehouse-api v0.1.0 (*)

  advisories ok
  ```
- `cargo audit --file rust/Cargo.lock --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0367 --ignore RUSTSEC-2026-0368 --ignore RUSTSEC-2026-0369`
  Exit code: 0
  Output:
  ```
      Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
        Loaded 1334 security advisories (from /home/shiro/.cargo/advisory-db)
      Updating crates.io index
      Scanning /home/shiro/rantai/wt/sec-42/rust/Cargo.lock for vulnerabilities (584 crate dependencies)
  Crate:     paste
  Version:   1.0.15
  Warning:   unmaintained
  Title:     paste - no longer maintained
  Date:      2024-10-07
  ID:        RUSTSEC-2024-0436
  URL:       https://rustsec.org/advisories/RUSTSEC-2024-0436
  Dependency tree:
  paste 1.0.15
  └── parquet 58.4.0
      ├── lakehouse-iceberg 0.1.0
      │   └── lakehouse-api 0.1.0
      ├── lakehouse-api 0.1.0
      └── iceberg 0.10.1
          ├── lakehouse-iceberg 0.1.0
          ├── lakehouse-api 0.1.0
          └── iceberg-catalog-rest 0.10.1
              └── lakehouse-iceberg 0.1.0

  Crate:     chacha20
  Version:   0.10.1
  Warning:   yanked
  Dependency tree:
  chacha20 0.10.1
  └── rand 0.10.3
      ├── quinn-proto 0.11.19
      │   └── quinn 0.11.11
      │       ├── reqwest 0.13.4
      │       │   ├── testcontainers 0.27.3
      │       │   │   ├── testcontainers-modules 0.15.0
      │       │   │   │   ├── lakehouse-test-support 0.1.0
      │       │   │   │   │   ├── lakehouse-store 0.1.0
      │       │   │   │   │   │   ├── lakehouse-auth 0.1.0
      │       │   │   │   │   │   │   └── lakehouse-api 0.1.0
      │       │   │   │   │   │   └── lakehouse-api 0.1.0
      │       │   │   │   │   ├── lakehouse-auth 0.1.0
      │       │   │   │   │   └── lakehouse-api 0.1.0
      │       │   │   │   └── lakehouse-api 0.1.0
      │       │   │   ├── lakehouse-test-support 0.1.0
      │       │   └── lakehouse-api 0.1.0
      │       │   ├── object_store 0.14.2
      │       │   │   ├── lakehouse-iceberg 0.1.0
      │       │   │   │   └── lakehouse-api 0.1.0
      │       │   │   └── lakehouse-api 0.1.0
      │       │   └── lakehouse-api 0.1.0
      │       └── reqwest 0.12.28
      │           ├── lakehouse-trino 0.1.0
      │           │   └── lakehouse-api 0.1.0
      │           ├── lakehouse-notify 0.1.0
      │           │   ├── lakehouse-api 0.1.0
      │           │   └── lakehouse-alerts 0.1.0
      │           │       └── lakehouse-api 0.1.0
      │           ├── lakehouse-llm 0.1.0
      │           │   └── lakehouse-api 0.1.0
      │           ├── lakehouse-iceberg 0.1.0
      │           ├── lakehouse-dagster 0.1.0
      │           │   └── lakehouse-api 0.1.0
      │           ├── lakehouse-clickhouse 0.1.0
      │           │   ├── lakehouse-bi 0.1.0
      │           │   │   ├── lakehouse-api 0.1.0
      │           │   │   └── lakehouse-alerts 0.1.0
      │           │   ├── lakehouse-api 0.1.0
      │           │   └── lakehouse-alerts 0.1.0
      │           ├── lakehouse-auth 0.1.0
      │           ├── lakehouse-api 0.1.0
      │           ├── lakehouse-alerts 0.1.0
      │           ├── iceberg-catalog-rest 0.10.1
      │           │   └── lakehouse-iceberg 0.1.0
      │           └── iceberg 0.10.1
      │               ├── lakehouse-iceberg 0.1.0
      │               ├── lakehouse-api 0.1.0
      │               └── iceberg-catalog-rest 0.10.1
      ├── object_store 0.14.2
      ├── lakehouse-bi 0.1.0
      ├── lakehouse-auth 0.1.0
      ├── lakehouse-api 0.1.0
      ├── lakehouse-alerts 0.1.0
      └── ferroid 2.0.0
          └── testcontainers 0.27.3

  warning: 2 allowed warnings found
  ```
- `docker run --rm -v /home/shiro/rantai/wt/sec-42:/repo -w /repo zricethezav/gitleaks:v8.30.1 dir /repo --config /repo/.gitleaks.toml --no-banner --redact`
  Exit code: 0
  Output: `no leaks found` (run before each push).

#### 4. Skipped / not verified
- *Not verified:* `cargo test --workspace`, `cargo clippy`, `cargo fmt`: No Rust code files were modified; only `rust/deny.toml`, `.github/workflows/security.yml`, `CHANGELOG.md`, and the plan were modified. Full compilation avoided per plan: "No workspace compile is needed: `cargo deny check advisories` and `cargo audit` only read `Cargo.lock`".
- *Not verified:* `bun test`, `bun run typecheck`, pytest, docker compose: No TypeScript, Python, or Docker compose files were modified.

#### 5. Repository status
```
$ git status --short

$ git log --oneline --abbrev=12 origin/main..HEAD | head
61c10810d869 docs(changelog): document SEC-42 zip advisories exception
9a41d644bb39 ci(security): ignore unreachable zip advisories in cargo audit (SEC-42)
e297bd7d216f fix(security): ignore unreachable zip advisories in deny.toml (SEC-42)
4f7041f354e2 docs(plan): SEC-42 zip advisories

$ git ls-remote --heads origin fix/sec-42-zip-advisories
61c10810d869	refs/heads/fix/sec-42-zip-advisories
```

## 5. Review

(planner)

Reviewed 2026-10-17 at `cb2a0f64` (planner): the diff matches §2–§3.
Planner's own run in `rust/`: `cargo deny check advisories` → "advisories
ok"; `cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0367
--ignore RUSTSEC-2026-0368 --ignore RUSTSEC-2026-0369` → exit 0 (2
allowed warnings, the existing unmaintained ones). The handoff pasted a
full commit hash; the planner shortened it to 12 characters. No open
BLOCKER.
