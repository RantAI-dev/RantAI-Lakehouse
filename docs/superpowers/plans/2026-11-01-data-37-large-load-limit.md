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

### Review round 1 (planner, 2026-11-01, at `3b0f137e`)

- **T1 matches D1 and D3.** The large count is a third check inside `under_limits`. It runs on
  the connection that holds the `file_upload:ingest_limit` lock, so two starts cannot both take
  the last large place. A load is large by the primary's `size_bytes` plus its `also`, and a
  running load by its leader plus its followers (`loaded_with`), within the same
  `LOAD_COUNTS_FOR_HOURS` window. A small load returns before the large count is read, so it is
  never refused by it.
- **T2 matches D2 and D4.** `UPLOAD_MAX_CONCURRENT_LARGE` is parsed by `parse_limit`
  (default 2, and 0 refuses the start). Both the early read and the locked mark answer
  `too_many_large_loads()`, a 429 with `Retry-After` and its own sentence naming the setting.
  Compose passes it with `${…:-}`.
- **T3.** The G9 harness's expectation is corrected, not the limit (rule 2). That is the right
  call.
- **One API test failed locally for an environmental reason:** another stack's ClickHouse held
  the default port. The diff does not touch it, and CI's workspace run is the judge.

**Open: no BLOCKER, no SHOULD-FIX.** Merge waits for the PR's CI.

### Incident: the review commit reverted the code (planner, 2026-11-01)

The planner's review commit `f8cd1db2` carried this plan's review **and the branch's code as it
stood before T1–T3**. It therefore undid every code change of `d41cebd8`, `68757f99` and
`fb39a253`, and #319 merged that state: `main` at `c70b9e2d` has the history of DATA-37 but none
of its code.

Cause: the planner's worktree and the developer's worktree had the same branch checked out, and
git worktrees share their branch refs. The developer's `git checkout -B` moved the branch under
the planner's worktree, whose index still held the old files, so the planner's docs-only commit
recorded them.

Fix (branch `fix/data-37-restore-code`): every non-plan file is checked out from `3b0f137e`, the
reviewed head, byte for byte (`git diff --cached 3b0f137e -- <files>` is empty). No other commit
touched these files between the base `5661a043` and `main`. The planner's worktrees are now
detached, and the planner diffs the staged paths before every commit.
