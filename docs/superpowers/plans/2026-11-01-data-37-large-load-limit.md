# DATA-37: a separate limit on large loads at once

Backlog `DATA-37` (Task, Data, P2). Base: `main` at `5661a043`. Planner: Claude Opus. Developer:
another agent (`AGENTS.md`).

## 0. Why

`DATA-33` measured about 2.2 GiB of job memory per 2 GiB load (`plans/DATA-17-RESULT.md`).
The only installation-wide cap is `UPLOAD_MAX_CONCURRENT`, 16 loads of any size. Sixteen large
loads would ask for about 35 GiB on a host whose minimum is 16 GiB, and the host would run out
of memory.

## 1. Decisions (product owner, 2026-11-01)

- **D1. A large load is a load whose files total more than 512 MiB**, counting the
  `size_bytes` of the load's primary upload plus every upload it carries (`loaded_with`). The
  threshold is a constant with its reason at the site. It is not a setting.
- **D2. At most 2 large loads run at once across the installation** (product owner,
  2026-11-01). This replaces the proposed 4: two are about 4.4 GiB, which leaves room for the
  stack on the 16 GiB minimum host, and the stack's own idle memory is not measured. The
  number is configurable as `UPLOAD_MAX_CONCURRENT_LARGE`, default 2, parsed like the other
  two limits (`parse_limit`). 0 or a non-number fails the API's start.
- **D3. The large limit is checked with the other two, in the same place and under the same
  lock.** The places are `under_limits` / `mark_group_within_limits`
  (`lakehouse-store/src/uploads.rs`, `:872`, `:1007`), under the advisory lock
  `file_upload:ingest_limit`. Two starts at the same moment therefore cannot both take the last
  large place. A small load never counts against it and is never refused by it. A large load
  still counts against the per-user and installation limits as well.
- **D4. A refusal is a 429 with `Retry-After`, as `SEC-17` answers today, with its own
  message.** It says that the installation already runs the maximum number of large loads,
  that the load can be started again when one finishes, and names the setting. The person can
  then tell this refusal from the per-user one.
- **D5. Not queued.** A refused large load is not held and started later. It is refused, as
  the other limits are.

## 2. Anchors (verified at `5661a043`)

- `rust/crates/lakehouse-store/src/uploads.rs`: `LoadLimits` `:827`, `loads_under_limits`
  `:861`, `under_limits` `:872` (the count query), `mark_ingesting_within_limits` `:969`,
  `mark_group_within_limits` `:1007` (the lock, the check, the claim).
- `rust/crates/lakehouse-api/src/routes/uploads.rs`: `load_limits` `:2735`,
  `check_within_limits_and_column_cap` `:2754`, and `too_many_uploads()` (the 429).
- `rust/crates/lakehouse-api/src/config.rs:1650-1651` (the two limits parsed), its tests
  `:2863-2892`, and `ConfigError::InvalidUploadLimit`.
- `docker-compose.yml:286-287`, `.env.example:51-52`.
- `docs/core/features/upload-limits-and-safe-csv.md` decisions table (`:68-69`).
- `ops/g9/large_upload.py` (`--concurrent N`, and a 429 recorded as a result, `:540`).

## 3. Tasks (one commit each, in order)

- **T1. The store.**
  - `LoadLimits` gains `large: i64`.
  - `under_limits` also counts the running loads that are large. A load is a leading
    `ingesting` row (`loaded_with IS NULL`) whose own `size_bytes` plus that of its followers
    is more than the threshold. The count is under the same `LOAD_COUNTS_FOR_HOURS` window.
    When the load being started is large, that count must be under `limits.large`.
  - The new load's size is the primary's `size_bytes` plus that of `also`, so `under_limits`
    takes `also`.
  - `MarkOutcome` gains `LargeLimitReached`, so the route can say which limit refused it.
  - `sqlx::test` tests:
    - two large loads run, and a third is refused;
    - a small load is not refused while two large ones run;
    - a load of several files is large by its total;
    - a stale row older than the window does not count.
  - *Accept:* `cargo test -p lakehouse-store uploads` and `cargo clippy -p lakehouse-store
    --all-targets -- -D warnings`.
- **T2. The API and the configuration.**
  - `UPLOAD_MAX_CONCURRENT_LARGE`, default 2, in `Config`, with a config test for the default,
    for a value, and for a 0 or a non-number refusal.
  - `load_limits` passes it.
  - The route answers `LargeLimitReached` with its own 429 message (D4), checked by the early
    read as well as by the locked mark.
  - Add `UPLOAD_MAX_CONCURRENT_LARGE: ${UPLOAD_MAX_CONCURRENT_LARGE:-}` to the API's
    environment in `docker-compose.yml`, and a commented line with its reason in
    `.env.example`.
  - A route test for the message.
  - *Accept:* `cargo test -p lakehouse-api uploads` and `cargo test -p lakehouse-api config`,
    plus clippy for the crate.
- **T3. The page and the proof.**
  - Add a decision row to `upload-limits-and-safe-csv.md` (the large limit, 2,
    `UPLOAD_MAX_CONCURRENT_LARGE`, `DATA-37`).
  - Read `ops/g9/large_upload.py`. If it runs more than 2 large loads at once, its expectation
    must say that the 3rd and later are refused with the large-load message. Do not raise the
    limit to make it pass (`AGENTS.md` rule 2). Report what you changed and why.
  - *Accept:* `python3 -m pytest ops/g9 -q` if the folder has unit tests, and
    `docker compose --profile '*' config --quiet`.

## 4. Out of scope

- Queuing refused loads (D5).
- A limit derived from the host's memory.
- Measuring the stack's idle memory, which would be needed before raising the default.

## 5. Handoff (developer appends below)

### Handoff (developer, 2026-11-01)

One commit per task, in plan order, each pushed as it was made:

| Commit | Task | What it holds |
| --- | --- | --- |
| `d41cebd8` | T1 | The store: `LARGE_LOAD_BYTES` (D1), `LoadLimits::large`, `LimitOutcome`, `MarkOutcome::LargeLimitReached`, the check under the same lock (D3). |
| `68757f99` | T2 | The API and the configuration: `UPLOAD_MAX_CONCURRENT_LARGE` (D2), the refusal with its own sentence (D4), compose and `.env.example`. |
| `fb39a253` | T3 | The feature page's decision row and the G9 harness's expectation. |
| this commit | — | This handoff. |

**Commands run (all from `rust/`, through `/home/shiro/rantai/bin/cargo-locked`; the
machine is memory-capped and shared, so every cargo command was scoped to the
crate it touched).**

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean (run again on the final commit) |
| `cargo clippy -p lakehouse-store --all-targets -- -D warnings` | clean |
| `cargo clippy -p lakehouse-api --all-targets -- -D warnings` | clean |
| `cargo test -p lakehouse-store` | 38 test binaries, 840 passed, 0 failed (`--test uploads` alone: 63 passed, of which 4 are the new `DATA-37` ones; `--test upload_folders`: 14 passed) |
| `cargo test -p lakehouse-api --lib` | 2880 passed, 1 failed (an environmental failure, see below), 18 ignored |
| `cargo test -p lakehouse-api --lib large_load_limit` | 3 passed (the two config tests and the refusal's sentence) |
| `cargo test -p lakehouse-api --lib config` | 160 passed (the filter `config` also matches tests of other modules) |
| `cargo test -p lakehouse-api --test upload_routes` | 175 passed |
| `cargo test -p lakehouse-api --test upload_folder_routes` | 7 passed |
| `cargo test -p lakehouse-api --test connector_upload_table` | 7 passed |
| `cargo test -p lakehouse-api --test hostile_uploads` | 0 passed, 2 ignored (both are ignored on `main` too) |
| `python3 -m pytest ops/g9 -q` | 29 passed (pytest, requests, pyarrow and fastavro in a throwaway venv at `/tmp/opencode/g9-venv`; the system python has no pytest) |
| `python3 ops/lint/check_intra_package_imports.py` | OK |
| `python3 ops/lint/check_bare_iceberg_count.py` | OK |
| `docker compose --profile '*' config --quiet` | OK (with throwaway values for `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`, `LAKEKEEPER_ENCRYPTION_KEY` and `CONNECTOR_SECRETS_KEY`; no `.env` was written) |

**What the developer changed in the API for T1, and why.** `LoadLimits` gained a
field, so the API's `load_limits()` could not compile unchanged. In T1 it passed
`large: i64::MAX` — "no load is refused by the large limit yet" — with a comment
that says the setting arrives in T2; T2 replaced it with
`state.config.upload_max_concurrent_large`. The store's `loads_under_limits`
now takes the other files of the load (`also`) and answers a `LimitOutcome`
instead of a `bool`, because D4 wants the refusal to say WHICH limit refused it,
and the route has to say the same thing at the early read and at the locked
mark.

**`ops/g9/large_upload.py`: what changed and why.** The harness loads one file
at a time by default (`--concurrent 1`), but `--concurrent N` starts N loads of
the same file together, and its CSV and JSON Lines files are 2,048 MiB by
default, over the 512 MiB of the threshold. With the new limit, the third and
later loads of such a phase are refused with the large-load sentence. The
harness's expectation now says so (its docstring, point 4, and the `--concurrent`
help): the third and later loads are refused with the large-load message, the
refusal is recorded as a result, and the run ends with a non-zero status. The
harness's CODE is unchanged: a refused load was already a result to record and
not an error to retry (that is what it already did for the `SEC-17` limits), and
the limit was NOT raised to make a run pass (`AGENTS.md` rule 2). The docstring
also says that a phase whose file is under the threshold (a Parquet file of
194 MiB by default) is not large and is not refused by this limit.

**Not verified, with the reason.**

- The full-workspace block of `AGENTS.md` (`cargo clippy --workspace
  --all-targets --all-features` and `cargo test --workspace`) was **not run**:
  the rules for this machine forbid a full-workspace run (it is memory-capped
  and another developer is building on it at the same time). The crates this
  change touches were run in full instead: `lakehouse-store` (all 38 test
  binaries) and `lakehouse-api` (its lib binary and the four upload test
  binaries). The other crates of the workspace are untouched by the diff.
- `bun run typecheck`, `bun run lint` and `bun run test` were **not run**:
  no TypeScript file changed (the "only build what changed" rule).
- A real `docker compose up` of the affected profile was **not run**: the plan's
  accept for T3 is `docker compose --profile '*' config --quiet`, which passed,
  and the host is shared (another developer's containers are up, one of them
  publishing the API's default ClickHouse port 18123, so starting a stack here
  would collide). The compose change is one environment-variable passthrough of
  the same shape as the two limits above it.
- The G9 harness was **not run against a live stack** (it needs a running
  console and a 2 GiB file); only its unit tests were run.
- `lakehouse-api`'s `routes::gold::tests::unchanged_since_last_export_reason_fails_closed_on_read_error`
  is **red on this machine for a reason outside this change**: the test sets
  `CLICKHOUSE_URL`, a name the configuration does not read, and relies on the
  default `ch_url` (`http://localhost:18123`) being unreachable. Another
  developer's ClickHouse container publishes exactly that port on this host
  (`curl 'http://localhost:18123/?query=SELECT%201'` answers `1`), so the
  engine the test expects to be unreachable is reachable and the query
  succeeds. The diff touches neither `routes/gold.rs` nor the default of
  `ch_url`, so this is red on `main` on this host as well.

**Docker.** The store's `sqlx::test` runs use the shared Postgres testcontainer
of `lakehouse-test-support` (`org.rantai.lakehouse-test-support=
postgres-16-shm1g`, started with `ReuseDirective::Always`); it is meant to
outlive the test binary and was left running. No container was started by this
work that was not already the shared one.

## 6. Review (planner appends below)
