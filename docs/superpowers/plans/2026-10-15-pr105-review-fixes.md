# PR #105 review fixes (SEC-12 withdrawal race, orchestrator error class)

Planner and reviewer: Claude Opus. Developers: Antigravity (Gemini 3.1 Pro)
for slice A, OpenCode (MiniMax M3.1) for slice B. Base: `fix/sec-phase-0`
at `0e318c18` (main merged in, catch-up 5). Anchors verified at `0e318c18`.

Source: the planner reviews on PR #105 (2026-10-12, 2026-10-14,
2026-10-15). The product owner withdrew the process BLOCKER on 2026-10-15.
What is left is the SHOULD-FIX list of the 2026-10-15 review; this plan
covers the code items. Items 3, 4 and 6 are docs and are done by the
planner (section 5).

## 1. Decisions already made

- **D1 (SHOULD-FIX 1, embed withdrawal race).** Embed access state leaves
  the `console.bi_board` row. A dashboard save (`save_board_patch`, a
  full-row `INSERT` into a `ReplacingMergeTree`) can then never write an
  old `embed_access` back. The state moves to a new append-only table,
  `console.bi_board_embed_event`, written only by `INSERT` and read by
  aggregation:
  - `revoked_before` = `max(revoked_before)` over the board's
    `revoke_all` events. A later, smaller value cannot move it back.
  - withdrawn tokens = `jti` with `max(exp)` over `withdraw` events whose
    `exp + tolerance` has not passed (the tolerance `withdraw_token` uses
    today). Union, so two withdrawals racing both stay.
  - origins = the `origins_json` of the newest `origins` event
    (`argMax` on a server-side `DateTime64(6)` `at`). Last write wins is
    correct for a replace-the-list action; it is not a revocation.
  - The cap `MAX_WITHDRAWN_TOKENS` is still checked before inserting a
    `withdraw` event (count of unexpired distinct `jti`). The check can be
    raced by two requests; the cap is a size bound, not a security
    property, and the comment says so.
  - The three `embed_*` columns this PR added to `console.bi_board` (the
    `ALTER TABLE … ADD COLUMN` calls in `ensure_bi_table`, `BOARD_COLS`,
    `row_to_board`, `upsert_board`) are removed. They never reached
    `main`, so no data moves. A dev database that already has them keeps
    them unused; their `DEFAULT`s make an `INSERT` without them valid.
  - `Board.embed_access` stays the type callers use (`routes/embed.rs`,
    `routes/dashboard.rs`). `get_board` and `get_board_by_token` fill it
    with one extra query; `list_boards` and other list reads leave it
    `EmbedAccess::default()` (verified: every reader of `embed_access`,
    `routes/embed.rs:139/211/224` and `routes/dashboard.rs:1436`, gets the
    board from `get_board` or `get_board_by_token`).
  - `EmbedAccess`'s pure logic (`is_withdrawn`, `revoke_all`,
    `withdraw_token`, `validate_origins`) stays in `embed_access.rs`; the
    store builds `EmbedAccess` from the aggregate.
- **D2 (SHOULD-FIX 5, error class).** `orchestrator_unreachable`
  (`upstream_error.rs:181`) answers `503` + `code: orchestrator_unreachable`
  (which the gates retry) only for a failure that really means "retry":
  `DgError::Transport`, and a non-2xx HTTP status of 500 or more. A GraphQL
  `errors` array, a body that does not parse, or a 4xx is not "unreachable":
  it is reported through the same `report(...)` path with the
  `ORCHESTRATOR_UNREACHABLE` pair's first message ("The orchestrator did
  not accept the request.") as a `502` with no retry `code`. To tell a
  status apart without parsing text, `lakehouse-dagster` gets a typed
  variant `DgError::Status(u16)` used where it now builds
  `DgError::Server(format!("Dagster HTTP {}", …))` (`lib.rs:1157`), with
  the same `Display` ("Dagster HTTP {n}"). Every exhaustive `match` on
  `DgError` in the workspace is updated; any place that matched the
  `"Dagster HTTP"` text is changed to match the variant.
- **D3 (SHOULD-FIX 2).** `src/proxy.ts:26` keeps its bare `fetch`. It runs
  in Next's server-side proxy (middleware) against `RUST_API_URL`;
  `apiFetch` is the browser client (same-origin `/api`, cookies). Add the
  why-comment at the site, as `AGENTS.md` requires for going around a
  choke point.
- **D4 (SHOULD-FIX 6).** `POST /api/pipelines/{id}/trigger` keeps `503` +
  `orchestrator_job_not_loaded` for `PipelineNotFoundError`. A pipeline id
  that is unknown or out of scope is already refused before any `Dagster`
  call (`authored_pipelines::in_scope`, `authored_launch_target`), so a
  not-found from `Dagster` after that is a job we know of that is not
  loaded: a code location reloading, or a broken one. Neither is the
  caller's 404. No code change; the planner records it on the feature
  page.

## 2. Anchors (at `0e318c18`)

- `rust/crates/lakehouse-bi/src/store.rs`: `ensure_bi_table` embed
  `ALTER`s 552–576; `BOARD_COLS` 607; `row_to_board` embed fill 662;
  `upsert_board` 766–794; `save_board_patch` 796–851 (embed carry at
  836); `BoardPatch.embed_access` 873; `set_board_embed_origins` 1083;
  `revoke_all_embed_tokens` 1117; `withdraw_embed_token` 1149; test
  `row_to_board_reads_the_embed_access_columns_and_does_not_serialise_them`
  2913; the `bi_board` columns test near 2579.
- `rust/crates/lakehouse-bi/src/embed_access.rs`: `EmbedAccess` 55,
  `from_stored` 73, `revoke_all` 113, `withdraw_token` 130,
  `MAX_WITHDRAWN_TOKENS` 40.
- Callers: `routes/embed.rs` 139, 215–225, 302; `routes/dashboard.rs`
  681, 695, 1436–1444.
- `rust/crates/lakehouse-dagster/src/lib.rs`: `DgError` 157–190,
  `Server(format!("Dagster HTTP …"))` 1157.
- `rust/crates/lakehouse-api/src/upstream_error.rs`: `orchestrator_unreachable`
  175–190; callers `routes/connectors.rs` 3323, 3405.
- `src/proxy.ts` 23–35.

## 3. Tasks

### Slice A — Antigravity (branch `fix/pr105-embed-events`, off `0e318c18`)

**A1. Embed access in an append-only table (D1).** One commit,
`fix(bi): keep embed access out of the board row (PR #105 review SHOULD-FIX 1)`.
- Create `console.bi_board_embed_event` in `ensure_bi_table` with a
  why-comment (the race, the aggregation rules). Columns at least:
  `board_id String, kind LowCardinality(String), revoked_before UInt64
  DEFAULT 0, jti String DEFAULT '', exp UInt64 DEFAULT 0, origins_json
  String DEFAULT '[]', at DateTime64(6) DEFAULT now64(6)`; `MergeTree`
  `ORDER BY (board_id, at)`. Values bound / `SqlLiteral` as the file
  already does; `format!` only for constant identifiers.
- Remove the three `bi_board` embed columns from the code (D1).
- `set_board_embed_origins`, `revoke_all_embed_tokens`,
  `withdraw_embed_token` insert one event each and no longer read or write
  the board row (they still check the board exists and keep their
  signatures, return values and `# Errors`).
- `save_board_patch`/`BoardPatch`/`upsert_board` lose `embed_access`.
- A board deleted and its id reused is not a case (ids are generated);
  do not build cleanup of events.
- **Accept:** tests (wiremock or whatever the file's existing ClickHouse
  test pattern is; grep before inventing one) that show:
  (a) a dashboard save's SQL contains none of the embed columns;
  (b) the aggregate read maps rows to `EmbedAccess` (max `revoked_before`,
  union of `jti`, newest origins, expired `jti` dropped);
  (c) each of the three writers issues an `INSERT` into the event table
  and no `INSERT` into `bi_board`.
  The old `row_to_board_reads_the_embed_access_columns…` test is replaced,
  with the reason in the commit body (rule 2).
  `cargo fmt --check`, `cargo clippy -p lakehouse-bi -p lakehouse-api
  --all-targets -- -D warnings`, `cargo test -p lakehouse-bi`,
  `cargo test -p lakehouse-api --lib embed`, `cargo test -p lakehouse-api
  --lib dashboard`, `cargo test -p lakehouse-api --test embed_tokens`
  all green.

### Slice B — OpenCode MiniMax M3.1 (branch `fix/pr105-orchestrator-class`, off `0e318c18`)

**B1. `DgError::Status` and the narrower retry class (D2).** One commit,
`fix(api): retry code only for an unreachable orchestrator (PR #105 review SHOULD-FIX 5)`.
- Add `DgError::Status(u16)` with a doc comment; build it at `lib.rs:1157`
  (and any other non-2xx site in that file).
- `orchestrator_unreachable`: `Transport` and `Status(s) if s >= 500` →
  the current `503` + code. Everything else → `502`, fixed message, no
  `code`, raw text to the log only. Update its doc comment (rule 1: verify
  each claim against the code).
- **Accept:** unit tests in `upstream_error.rs` for `Transport` → 503 +
  code, `Status(502)` → 503 + code, `Status(400)` → 502 no code,
  `Server("…GraphQL…")` → 502 no code and the raw text not in the body.
  `cargo test -p lakehouse-dagster`, `cargo test -p lakehouse-api --lib
  upstream_error`, `cargo test -p lakehouse-api --lib connectors`,
  `cargo test -p lakehouse-api --test sec11_guard`, clippy for both
  crates, `cargo fmt --check`.

**B2. Why-comment at `src/proxy.ts` (D3).** One commit,
`docs(proxy): say why the embed frame check uses fetch (PR #105 review SHOULD-FIX 2)`.
- **Accept:** `bun run typecheck && bun run lint`.

## 4. Rules for both developers

- Work only in your own worktree and branch, named above. Do not push to
  `fix/sec-phase-0` or `main`, do not open or merge a PR. Push your branch
  to `origin`.
- `export CARGO_TARGET_DIR=/home/uaa/rantai/.cargo-target` for every cargo
  command (shared cache; the other developer builds there too, so a
  "Blocking waiting for file lock" is normal: wait). Never `cargo clean`.
- Do not run `cargo test --workspace` or workspace-wide clippy: this
  machine (11 GB) falls over under it. CI runs it. Write it as *not
  verified* in the handoff.
- If this plan is wrong against the code, stop and write the mismatch in
  your handoff; do not improvise scope.
- Commit body cites the finding (`PR #105 review SHOULD-FIX n`). Comments in
  English. `git status` before each commit; nothing generated staged.
- Append your handoff to section 6 of this file **in your branch**:
  commits, exact commands with pass/fail counts, what you did not verify.

## 5. Planner's docs (on `fix/sec-phase-0`)

- SHOULD-FIX 3: close `SEC-6` and `SEC-29` in `docs/core/BACKLOG.md` with
  this PR (the code has zero such lines; `sec11_guard` enforces it).
- SHOULD-FIX 4: note the `run_sql` refusal-before-`EXPLAIN` hardening on
  the `SEC-9` Done row.
- SHOULD-FIX 6: the `503` decision on `docs/core/features/no-raw-upstream-errors.md`.
- SHOULD-FIX 1: the race and its fix on `docs/core/features/safer-embed-tokens.md`.

## 6. Handoff

### Slice B — developer (OpenCode MiniMax M3.1), 2026-10-15

Branch `fix/pr105-orchestrator-class`, base `e17a8281`:

- `66fc42f3` — `fix(api): retry code only for an unreachable orchestrator (PR #105 review SHOULD-FIX 5)` (B1; 5 files, +237/−34)
- `ba8d63a2` — `docs(proxy): say why the embed frame check uses fetch (PR #105 review SHOULD-FIX 2)` (B2; `src/proxy.ts` only, +8)
- `29f31e98` — `docs(dagster): name Status in launch_run_with_config's Errors` (doc only, part of B1)
- `89231890`, `1f4766fc`, `3e2eee8d` — the three commits carrying this section; the first was pushed by an interrupted run of this slice and the other two consolidate it into the single section below.

**Plan vs code.** No scope mismatch. The anchors held at `e17a8281`
(`DgError` 157, `Server(format!("Dagster HTTP …"))` 1157,
`orchestrator_unreachable` 181), and the file's two further non-2xx sites
(2323, 2393) fall under the plan's "any other non-2xx site in that file".
Two things the plan did not spell out, both recorded below: the new
`ApiError::BadGateway` variant (accepted by the planner) and the
`routes/connectors.rs` doc comment that named the old class.

**B1, what changed.** `lakehouse-dagster` gains `DgError::Status(u16)`
(`Display` unchanged, `"Dagster HTTP {n}"`), built at all three non-2xx
sites, so a caller can tell a `5xx` from a `4xx` without parsing text.
`upstream_error::orchestrator_unreachable` keeps `Transport` and
`Status(s) if s >= 500` on `503` + `code: orchestrator_unreachable` +
`Retry-After`, and sends `Status(4xx)` and `Server(..)` to a `502` with the
context's `failed` sentence, no `code`, no `Retry-After`, raw text to the log
only. `UpstreamFailure::into_bad_gateway` sits beside `into_not_ready`.
`routes/connectors.rs`'s `ingest_run_request` doc is updated to state the
split (it still called a protocol failure "unreachable").

**One file the plan does not name: `lakehouse-core`.** No `502` existed
anywhere in the workspace (`ApiError::status()` covered
400/401/403/404/409/422/429/500/503) and D2 asks for one, so
`ApiError::BadGateway(String)` is added beside `Self::Unavailable` with the
unreachable/unusable split documented at the variant, plus its `status()` →
502 arm and a test. `ApiError::status()` is the only exhaustive `match` on
the enum in the workspace (grep over `rust/crates`); the renderer
`lakehouse-api/src/error.rs` picks `code` with an `if let` + `_ =>`, so no
route status table moves. Accepted by the planner.

**Tests added.** Four in `upstream_error.rs` (Transport → 503 + code + a
`WARN` log line, `Status(502)` → 503 + code, `Status(400)` → 502 without a
code, a GraphQL `Server(..)` → 502 whose planted marker is in the log and
not in the body), one in `lakehouse-dagster` (a `503` answer is
`DgError::Status(503)` with the same `Display`), one in `lakehouse-core`
(`BadGateway` → 502).

**Commands run, with the counts they printed.** Rust, from `rust/`:

| command | result |
| --- | --- |
| `cargo fmt --check` | exit 0, no diff |
| `cargo clippy -p lakehouse-dagster -p lakehouse-api --all-targets -- -D warnings` | exit 0, no warnings |
| `cargo test -p lakehouse-dagster` | `69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test -p lakehouse-api --lib upstream_error` | `21 passed; 0 failed; 0 ignored; 0 measured; 1677 filtered out` (17 pre-existing + the 4 above) |
| `cargo test -p lakehouse-api --lib connectors` | `75 passed; 0 failed; 0 ignored; 0 measured; 1623 filtered out` |
| `cargo test -p lakehouse-api --lib pipelines` | `157 passed; 0 failed; 0 ignored; 0 measured; 1541 filtered out` |
| `cargo test -p lakehouse-api --test sec11_guard` | `4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test -p lakehouse-core` (extra: the crate B1 edits) | `48 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test -p lakehouse-api --test ingest_run -- --test-threads=2` (extra) | `22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` — includes `ingest_run_with_dagster_down_is_a_503_with_an_unreachable_code`, the end-to-end proof the `Transport` → 503 + code arm is unmoved |
| `cargo test -p lakehouse-api --test upstream_errors -- --test-threads=2` (extra) | `6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |

Without `--test-threads`, `ingest_run` fails all 22 on this 11 GB machine
(each test starts a Postgres container); that is a resource limit of the
box, not a code failure — one test alone passes.

TypeScript, from the worktree root: `bun run typecheck` exit 0;
`bun run lint` exit 0 — `✖ 6 problems (0 errors, 6 warnings)`, all
pre-existing and in files this slice does not touch.

**Deviation: the toolchain.** The shell this slice ran in had no Rust
toolchain at all (`cargo: command not found`, no `~/.cargo`, no `~/.rustup`),
so the pinned toolchain of `rust/rust-toolchain.toml` (1.96.1 + rustfmt +
clippy, minimal profile) was installed with rustup before anything could be
run. That is a change to the machine, not to the repository.

**Deviation: `CARGO_TARGET_DIR`.** The plan's shared
`/home/uaa/rantai/.cargo-target` is not safe while two worktrees build the
same crates, and this slice hit exactly that. Against the shared dir,
`cargo test -p lakehouse-api --lib upstream_error` first failed to compile
B1's own tests against the other worktree's crates (`E0599: no variant ...
named Status` / `... named BadGateway`), and the next run of the same
command reported `17 passed` — 17 is the pre-change count and the four new
tests were absent from the binary it ran; that binary's dep-info names
`/home/uaa/rantai/wt-pr105-embed/rust/migrations/...`, i.e. the slice-A
worktree. A unit's `-C metadata` and extra-filename are identical across
the two worktrees, so their builds overwrite each other's artifacts and
cargo's mtime freshness check can accept the wrong one: a false green of
exactly the kind rule 7 exists to catch. Every number above therefore comes
from a private target dir, `CARGO_TARGET_DIR=/home/uaa/rantai/.cargo-target-orch-pr105`,
built from this worktree only. Nothing in the shared directory was deleted
and `cargo clean` was never run, so the other developer's cache is
untouched. The reviewer should re-run its own verification somewhere other
than the shared dir while two worktrees are live; a per-branch target dir
(or a note in `AGENTS.md`) may be worth the planner's attention.

**Re-verified on the final tree** (code identical to `29f31e98`, after that
commit): every command in the table above was run once more against the
plan's *shared* `/home/uaa/rantai/.cargo-target` and printed the same counts
— `69`, `21`, `75`, `157`, `4`, `cargo fmt --check` clean, `bun run typecheck`
exit 0, `bun run lint` 0 errors/6 warnings. `--lib upstream_error` is the
discriminating one: `21` (17 pre-existing + this slice's 4) is this branch's
binary, where the slice-A worktree's prints `17`. In that same shared dir
the first clippy run failed `E0599` on code that had passed there minutes
earlier and the immediate re-run was clean — the overwrite described above,
seen from the other side, and the reason the shared dir is not a place to
take a green from.

**Consequences worth a look.** A `Dagster` answer that is neither a
transport failure nor a `5xx` is now a `502`, and
`ops/g6/g6_ingest_matrix_test.py` raises at once on anything that is not
`503` + one of `RETRYABLE_ORCHESTRATOR_CODES` — that is the gate's own rule
and D2's point, but such an answer no longer consumes the gate's 120 s wait
window. The transient in `docs/superpowers/plans/2026-10-14-ci-green.md`
(F3: a launch answering `Dagster HTTP 500` while the code location reloads)
is a `5xx`, so it keeps the retry code. No gate needed a change.

**Not verified, and why.**

- `cargo test --workspace` and `cargo clippy --workspace`: the plan forbids
  them on this 11 GB machine while two developers build; CI runs them.
- `bun run test`, the `ops/g*` gates and `docker compose config`: not run —
  this slice changes no TypeScript behaviour and no compose, and the gates
  need the whole stack up.
- The `502` is proven by unit tests over constructed `DgError` values, not
  against a live `Dagster` answering a `4xx` or a GraphQL error; no Dagster
  or ClickHouse was run here.
- Slice A (embed events) is not in this branch.
- The doc pages in section 5, and
  `docs/core/features/no-raw-upstream-errors.md:52` (which still presents
  `orchestrator_unreachable` as the answer for every orchestrator failure),
  are the planner's and were left untouched.

### Slice A — developer (Antigravity), 2026-10-15

- Implemented Task A1: moved embed access state from `console.bi_board` row to append-only `console.bi_board_embed_event` table read by aggregation (`PR #105 review SHOULD-FIX 1`).
- Commit: `3d2263c7` (`fix(bi): keep embed access out of the board row (PR #105 review SHOULD-FIX 1)`)
- Commands verified:
  - `cargo fmt --check`: passed (clean)
  - `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings`: passed (clean)
  - `cargo test -p lakehouse-bi`: 96 passed, 0 failed, 1 doc-test passed
  - `cargo test -p lakehouse-api --lib embed`: 8 passed, 0 failed
  - `cargo test -p lakehouse-api --lib dashboard`: 22 passed, 0 failed
  - `cargo test -p lakehouse-api --test embed_tokens`: 21 passed, 0 failed
- Not verified:
  - `cargo test --workspace` or workspace-wide clippy (skipped per rule to conserve memory on 11 GB host; CI runs workspace checks).

### Slice A on the current base — developer (Claude Sonnet), 2026-10-17

Merge of `origin/fix/pr105-embed-events` into this branch (base `48f3295b`),
conflicts in `store.rs` and this file resolved by hand (see the merge commit).

- Checks run, in the foreground, `CARGO_TARGET_DIR` shared, jobs 2:
  - `cargo fmt --check`: clean
  - `cargo clippy -p lakehouse-bi -p lakehouse-api -p lakehouse-dagster -p lakehouse-core --all-targets -- -D warnings`: clean
  - `cargo test -p lakehouse-bi`: 427 passed, 3 ignored; doc-test 1 passed
  - `cargo test -p lakehouse-api --lib -- embed dashboard upstream_error connectors`: 223 passed
  - `--test embed_tokens`: 21 passed; `--test sec11_guard`: 4 passed; `--test upstream_errors`: 6 passed
  - `bun run typecheck`: no errors
- **Plan assumption falsified.** D1 says the embed columns "never reached
  `main`, so no data moves". They did: `5d8f61e1` (SEC-12) is on `main` and
  creates `embed_revoked_before`, `embed_revoked_jti_json`,
  `embed_origins_json`. A deployment that ran it holds withdrawals there;
  after this change they are no longer read, so a withdrawn token would be
  accepted again. A one-time backfill of those columns into the event table
  (guarded by `system.columns`) is needed before this ships if `main` has
  been deployed. Not built in this entry; built afterwards as R-A1 (below).
- Not verified: `route_auth`, the full workspace test run, `bun run lint`
  and `bun run test`, anything against a live ClickHouse.

### R-A1 fix — developer (Claude Sonnet), 2026-10-17

- Commit: `fix(bi): copy the embed state stored on the board row into the event table (PR #105 review R-A1 BLOCKER)`;
  `copy_legacy_embed_state` in `store.rs`, called at the end of `ensure_bi_table`.
- Tests added (5, in `store.rs`): copy keeps jti/cut-off/origins effective;
  second run inserts nothing; no legacy columns runs no copy; copied
  origins event is epoch-stamped (the `argMax` read half is pinned by a
  source check, not run); unparseable list revokes everything.
- DDL-count test 22 to 23: the `system.columns` check request.
- Commands (foreground, shared target, jobs 2): `cargo fmt --check` clean;
  clippy on bi/api/dagster/core `--all-targets -D warnings` clean;
  `cargo test -p lakehouse-bi` 432 passed, 3 ignored, doc-test 1;
  `-p lakehouse-api --lib -- embed dashboard upstream_error connectors` 223;
  `--test embed_tokens` 21; `--test sec11_guard` 4; `--test upstream_errors` 6;
  `--test route_auth` (threads 2) 42.
- Not verified: against a live ClickHouse. The `NOT IN (... at = epoch)`
  idempotence and the `FINAL` read are SQL the mock stands in for (the mock
  emulates the idempotence); the real statements are unrun. No full
  workspace run, `bun run lint/test`.

### R-A3 / R-A4 fix — developer (Claude Sonnet), 2026-10-17

- Commit: `fix(bi): copy a board's legacy embed state in one INSERT and include trashed boards (PR #105 review R-A3/R-A4 SHOULD-FIX)`.
- R-A3: one multi-row `INSERT` per board (`legacy_embed_insert`). R-A4: no
  `is_deleted` condition in the copy's `SELECT`.
- Tests: the one-`INSERT` assertion replaces the earlier three-`INSERT` ones;
  new `the_legacy_embed_copy_does_not_skip_boards_in_the_trash` (checks the
  query text; the mock does not filter).
- Commands (foreground, shared target, jobs 2): `cargo fmt --check` clean;
  clippy bi/api/dagster/core `--all-targets -D warnings` clean;
  `cargo test -p lakehouse-bi` 433 passed, 3 ignored, doc-test 1;
  `-p lakehouse-api --lib -- embed dashboard upstream_error connectors` 223;
  `--test embed_tokens` 21; `sec11_guard` 4; `upstream_errors` 6;
  `route_auth` (threads 2) 42.
- Not verified: that ClickHouse writes a multi-row `INSERT` as one block
  (the planner's premise; not run here), full workspace run, `bun run lint/test`.

## 7. Review

### Review — slice A merged onto `main`, 2026-10-17, planner

- **R-A1 BLOCKER — withdrawals already stored on `main` stop being read.**
  The plan said the board row's embed columns never reached `main`; they
  did (SEC-12, `5d8f61e1`: `embed_revoked_before`, `embed_revoked_jti_json`,
  `embed_origins_json`, checked in `origin/main` `store.rs`). After the
  merge nothing reads them, so on a deployed database a withdrawn token is
  accepted again and a board's allowed origins are lost. That is the
  failure D1 exists to prevent, so the fix is in scope. Decision: a
  one-time copy into `console.bi_board_embed_event`, run by
  `ensure_bi_table` after the event table is created:
  - Only when `system.columns` shows the legacy columns on
    `console.bi_board` (a fresh database has none; skip without error).
  - Read each board's current row (`FINAL`, not deleted) and insert a
    `revoke_all` event when `embed_revoked_before > 0`, a `withdraw` event
    per `{jti, exp}` in `embed_revoked_jti_json`, and an `origins` event
    when `embed_origins_json` is not `[]`.
  - Every copied event gets `at` = the Unix epoch, so any real event
    written since wins over it (origins are read latest-first).
  - Idempotent: a second run inserts nothing (skip a board that already
    has an event at the epoch). Values bound, never formatted in.
  - The legacy columns are left in place and not written; dropping them is
    not this PR's (a migration header-style comment at the site says why).
  - Accept: a mocked legacy database with one withdrawn jti, a cut-off and
    two origins → after `ensure_bi_table` the jti is refused, a token older
    than the cut-off is refused and the origins are served; a second run
    inserts nothing; a fresh database (no legacy columns) runs no copy; a
    real `origins` event newer than the copy wins.
- **R-A2** — the three clippy fixes in `embed_tokens.rs` may stay in the
  merge commit; they are mechanical.

### Review — R-A1 fix (`5679107b`), 2026-10-17, planner

Re-run by the planner against the dev `ClickHouse` (26.8) in a scratch
database with the same table shapes: the copy's `SELECT` picks the legacy
board, returns nothing once an epoch event exists (idempotent), and
`argMax(origins_json, at)` returns a real event's origins even when the copy
is inserted after it. That half of the handoff's *not verified* is now
verified; the `FINAL` read on the real `bi_board` is not.

- **R-A3 SHOULD-FIX — a copy cut off half-way is never finished.** The
  board's first `INSERT` already writes the epoch marker, so if the process
  stops after it, the board's other events (a withdrawn jti) are never
  copied and that token is accepted again. Write each board's events in
  **one** multi-row `INSERT` (all columns listed, one `VALUES` tuple per
  event): `ClickHouse` inserts one block whole or not at all. Accept: the
  test that a board with a cut-off, a jti and origins issues one `INSERT`.
- **R-A4 SHOULD-FIX — boards in the trash are skipped.** `is_deleted = 0`
  leaves out phase 3's trashed boards (`is_deleted = 2`), which can be
  restored; a restored board would lose its withdrawals. Drop the
  `is_deleted` condition: an event for a deleted board is never read and
  costs nothing. Accept: a test with a trashed legacy board.

### Review — PR #201 before merge, 2026-10-21, planner (round F)

Reviewed at `3831f7b5` merged locally with `main` (`c8f6d395`; the branch is
107 commits behind and merges cleanly). **No BLOCKER.** R-A1, R-A3 and R-A4
are fixed in code: the embed state is written only by `INSERT` into
`console.bi_board_embed_event` and every reader (token check, `/api/embed/frame`,
`embed-info`) aggregates it; the legacy copy is idempotent, includes trashed
boards, is skipped on a fresh database and fails closed (`ensure_bi_table`
errors and retries). Slice B returns `orchestrator_unreachable` only for a
transport error or a 5xx, and merges with `main`'s `ApiError::Gone` without a
conflict. Before merge, fix these (one commit each, cite
`PR #201 review F SHOULD-FIX n` at the site and in the commit body):

1. **One clock tolerance.** `lakehouse-bi/src/store.rs` ~1585 keeps its own
   `DEFAULT_CLOCK_TOLERANCE_SECS = 60` and `get_board_embed_access` (~1640,
   ~1663) drops withdrawn tokens after `exp + 60`, while the verifier's
   tolerance is `lakehouse_embed::CLOCK_TOLERANCE_SECONDS`. If that is ever
   raised, a withdrawn token that the verifier still accepts drops off the
   withdrawn list and works again. Drop expired entries only when counting
   toward the cap, not in the list `is_withdrawn` reads (or take the tolerance
   from `lakehouse_embed`); a test with a tolerance above 60 that fails today.
2. **Rule 4:** `store.rs` ~1768–1780 re-implements `EmbedAccess::withdraw_token`'s
   unexpired-then-cap guard; call it (or delete the now test-only methods).
3. The `#[cfg(test)]` swap of the production `OnceCell` for a `RwLock<bool>`
   (`store.rs` ~559–566, ~588) means
   `ensure_bi_table_runs_ddl_at_most_once_per_process` no longer tests the
   production cache, and its doc ("only test … allowed to exercise
   `ensure_bi_table`", ~4335) is false now. Make the test exercise the real
   cache, or say plainly what it pins.
4. Stale comment, `lakehouse-api/src/routes/embed.rs` ~136: withdrawal is read
   by aggregating the event table, not "a `FINAL` read" of the board.
5. `src/proxy.ts`: the last sentence reads as if a visitor with no session
   always gets `frame-ancestors 'none'`; embed viewers have no session and get
   the board's allowed sites. Say the answer does not depend on a session.
6. `# Errors` docs in `lakehouse-dagster/src/lib.rs` (~1149, ~2229, ~2270,
   ~2344, ~2383, ~2420) still name only `Transport` and `Server`; the non-2xx
   sites return `DgError::Status`.
7. `revoke_all_embed_tokens` returns `now` (~1744) but documents "the stored
   instant"; the cut-off in force is the max over all `revoke_all` events.
   Return that max (the response's `embedRevokedBefore` must be what is
   enforced).

Also, one line each (no code): the legacy unparseable list becoming
`revoke_all(u64::MAX)` is permanent, so say "permanently" where `from_stored`
said "until saved again"; and the deploy note on the feature page — stop old
API instances before starting this build (an old instance's withdrawal after
the copy is never copied).
