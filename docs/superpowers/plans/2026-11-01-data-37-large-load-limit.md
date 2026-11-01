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

## 6. Review (planner appends below)
