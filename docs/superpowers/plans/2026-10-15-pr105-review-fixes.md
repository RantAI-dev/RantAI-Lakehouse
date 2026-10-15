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

Base `e17a8281`, branch `fix/pr105-orchestrator-class`. Commits, one per task:

- `66fc42f3` — `fix(api): retry code only for an unreachable orchestrator (PR #105 review SHOULD-FIX 5)` (B1, 5 files, +237/−34)
- `ba8d63a2` — `docs(proxy): say why the embed frame check uses fetch (PR #105 review SHOULD-FIX 2)` (B2, `src/proxy.ts` only, +8)

What B1 does: `DgError::Status(u16)` in `lakehouse-dagster` (built at all
three non-2xx sites — `lib.rs` `list_runs_with_steps_for_job`, `run_logs`,
`execute` — with the unchanged `Display` `"Dagster HTTP {n}"`);
`orchestrator_unreachable` keeps `503` + `code: orchestrator_unreachable` +
`Retry-After` for `Transport` and `Status(n) if n >= 500` only, and answers
everything else through the same `report(...)` path as a `502` with
`ORCHESTRATOR_UNREACHABLE`'s `failed` sentence, no `code`, raw text in the log
only. Route docs in `routes::connectors` that named the old class are
corrected at the same time.

#### One file the plan does not name: `lakehouse-core`

No `502` existed anywhere in the workspace (`ApiError::status()` covered
400/401/403/404/409/422/429/500/503), and D2 asks for one, so
`ApiError::BadGateway(String)` is added beside `Self::Unavailable` with the
unreachable/unusable split documented at the variant, plus its
`status()` → 502 arm and a test. `ApiError::status()` is the only exhaustive
match on the enum in the workspace (checked by grep over `rust/crates`); the
renderer `error.rs` picks `code` with an `if let` + `_ =>`, so nothing else
moves. Per the task note this variant is accepted by the planner.

#### Commands run, with the counts they printed

Rust, from `rust/`, `CARGO_TARGET_DIR` as noted below:

| command | result |
| --- | --- |
| `cargo fmt --check` | exit 0, no diff |
| `cargo clippy -p lakehouse-dagster -p lakehouse-api --all-targets -- -D warnings` | exit 0 (`Finished dev profile in 58.34s`) |
| `cargo test -p lakehouse-dagster` | 69 passed, 0 failed |
| `cargo test -p lakehouse-core` (extra: the crate I touched) | 48 passed, 0 failed |
| `cargo test -p lakehouse-api --lib upstream_error` | 21 passed, 0 failed (17 pre-existing + the 4 the plan lists) |
| `cargo test -p lakehouse-api --lib connectors` | 75 passed, 0 failed |
| `cargo test -p lakehouse-api --lib pipelines` | 157 passed, 0 failed |
| `cargo test -p lakehouse-api --test sec11_guard` | 4 passed, 0 failed |
| `cargo test -p lakehouse-api --test ingest_run -- --test-threads=2` (extra) | 22 passed, 0 failed — includes `ingest_run_with_dagster_down_is_a_503_with_an_unreachable_code`, the end-to-end proof the `Transport` → `503` + code path is intact |
| `cargo test -p lakehouse-api --test upstream_errors -- --test-threads=2` (extra) | 6 passed, 0 failed |

TypeScript, from the worktree root: `bun run typecheck` exit 0;
`bun run lint` exit 0 — `✖ 6 problems (0 errors, 6 warnings)`, all pre-existing
and none in `src/proxy.ts`.

#### Two environment facts the reviewer needs

1. **No Rust toolchain was installed on this machine** when the slice
   started (`cargo: command not found`, no `~/.cargo`, no `~/.rustup`). I
   installed the pinned toolchain of `rust/rust-toolchain.toml`
   (1.96.1 + rustfmt + clippy, minimal profile) with rustup to be able to run
   anything at all. That is a machine change outside the repository, not a
   repo change.
2. **`/home/uaa/rantai/.cargo-target` is shared with the slice-A developer and
   served this worktree artifacts built from THEIR source.** Observed
   concretely: `cargo test -p lakehouse-api --lib upstream_error` there first
   failed with five `E0599` errors (`no variant ... named Status`,
   `... named BadGateway`) and then, on a rerun, reported
   `17 passed` — 17 is the pre-change count, and the four new tests were
   absent from the binary it ran. Every number quoted above therefore comes
   from a private target dir, `CARGO_TARGET_DIR=/home/uaa/rantai/.cargo-target-orch-pr105`,
   built from this worktree only. Nothing in the shared cache was cleaned or
   deleted. Re-running the review's own verification in the shared dir can
   produce a false green for the same reason; it is worth a re-run there after
   slice A is done.

#### Consequences worth a look

- A `Dagster` answer that is not a transport failure or a `5xx` is now a
  `502`, and `ops/g6/g6_ingest_matrix_test.py` raises at once on anything
  that is not `503` + one of `RETRYABLE_ORCHESTRATOR_CODES` — that is the
  gate's own rule, and D2's point, but it does mean such an answer no longer
  consumes the gate's 120 s wait window. The transient in
  `docs/superpowers/plans/2026-10-14-ci-green.md` (F3: a launch answering
  `Dagster HTTP 500` while the code location reloads) is a `5xx`, so it
  keeps the retry code; nothing in the acceptance gates needed a change.
- `docs/core/features/no-raw-upstream-errors.md:52` still describes
  `orchestrator_unreachable` as the answer for both classes; that is the
  planner's SHOULD-FIX 6 (section 5), left untouched here.

#### Not verified, and why

- `cargo clippy --workspace --all-features -- -D warnings` and
  `cargo test --workspace`: forbidden by section 4 of this plan (11 GB
  machine); CI runs them.
- `bun run test`, the `ops/g*` gates, and `docker compose config`: not run —
  this slice touches neither TypeScript behaviour nor compose, and the gates
  need the whole stack up.
- The doc pages in section 5 and `no-raw-upstream-errors.md` are the
  planner's, not verified here.

Branch `fix/pr105-orchestrator-class`, base `e17a8281`. Commits:

- `66fc42f3` `fix(api): retry code only for an unreachable orchestrator (PR #105 review SHOULD-FIX 5)` — B1.
- `ba8d63a2` `docs(proxy): say why the embed frame check uses fetch (PR #105 review SHOULD-FIX 2)` — B2.
- the commit carrying this section.

**B1, what changed.** `lakehouse-dagster` gains `DgError::Status(u16)`
(`Display` unchanged, `"Dagster HTTP {n}"`), built at all three non-2xx
sites (`e17a8281`: 1157, 2323, 2393 — the plan named the first and "any
other"). `upstream_error::orchestrator_unreachable` keeps `Transport` and
`Status(s) if s >= 500` on the 503 + `code: orchestrator_unreachable`, and
sends `Status(4xx)` and `Server(..)` to a 502 with the context's `failed`
sentence, no `code`, no `Retry-After`, raw text to the log only. A 502 did
not exist in the workspace, so `ApiError::BadGateway` is added in
`lakehouse-core` (accepted by the planner) with `UpstreamFailure::
into_bad_gateway` beside `into_not_ready`; `ApiError::status()` is the only
exhaustive `match` on `ApiError` in the workspace and carries the new arm.
`routes/connectors.rs`'s `ingest_run_request` doc comment is updated to
state the split (it still described a "protocol failure" as unreachable).
Unit tests added: four in `upstream_error.rs` (Transport → 503 + code +
`WARN` log, `Status(502)` → 503 + code, `Status(400)` → 502 no code, and a
GraphQL `Server(..)` → 502 whose planted marker appears in the log and not
in the body), one in `lakehouse-dagster` (a `503` answer is
`DgError::Status(503)` with the same `Display`), one in `lakehouse-core`
(`BadGateway` → 502).

**Commands run** (from `rust/`; see the target-dir deviation below; every
number below is quoted from that run on this branch's code):

| command | result |
| --- | --- |
| `cargo fmt --check` | exit 0, no diff |
| `cargo clippy -p lakehouse-dagster -p lakehouse-api -p lakehouse-core --all-targets -- -D warnings` | exit 0, no warnings (lakehouse-core added to the plan's two crates because B1 edits it) |
| `cargo test -p lakehouse-dagster` | `test result: ok. 69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test -p lakehouse-api --lib upstream_error` | `test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 1677 filtered out` |
| `cargo test -p lakehouse-api --lib connectors` | `test result: ok. 75 passed; 0 failed; 0 ignored; 0 measured; 1623 filtered out` |
| `cargo test -p lakehouse-api --lib pipelines` | `test result: ok. 157 passed; 0 failed; 0 ignored; 0 measured; 1541 filtered out` |
| `cargo test -p lakehouse-api --test sec11_guard` | `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `cargo test -p lakehouse-core` | `test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` (not in the plan's list; run because B1 adds a variant and a test to that crate) |
| `cargo test -p lakehouse-core bad_gateway_maps_to_502` | `test result: ok. 1 passed; 0 failed; 47 filtered out` |
| `bun run typecheck` | exit 0 |
| `bun run lint` | exit 0, `6 problems (0 errors, 6 warnings)`, all pre-existing on this branch and in files this slice does not touch (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx`, `open-format-card.test.tsx`, `use-data-table.ts`, `dashboard-specs.ts`) |

**Deviation: `CARGO_TARGET_DIR`.** The plan's shared
`/home/uaa/rantai/.cargo-target` is not safe while two worktrees build the
same crates, and I hit it. `cargo test -p lakehouse-api --lib
upstream_error` against the shared dir first failed to compile MY
`upstream_error.rs` against the OTHER developer's `lakehouse-dagster` and
`lakehouse-core` (`E0599: no variant ... named Status` / `BadGateway`), and
the next run of the same command executed `lakehouse_api-17d22573297d3f60`,
whose dep-info names `/home/uaa/rantai/wt-pr105-embed/rust/migrations/...`:
the other worktree. That run's "17 passed" was their binary, not mine —
exactly the false green rule 7 exists to catch. The unit's `-C metadata`
and extra-filename are identical across the two worktrees, so the builds
overwrite each other's artifacts and cargo's mtime freshness check can
accept the wrong one. Every cargo command above therefore ran with
`CARGO_TARGET_DIR=/home/uaa/rantai/.cargo-target-pr105-orch`, seeded by
copying the shared dir's `deps/`, `build/` and `.fingerprint/` (not
`incremental/`, ~7.5 GB) so the dependency cache stayed warm, after
touching this worktree's `crates/**/*.rs` and `migrations/*.sql` so every
workspace crate was rebuilt from here. Provenance checked afterwards: the
newest `lakehouse_api-*.d` files in that directory name
`/home/uaa/rantai/wt-pr105-orch`. Nothing in the shared directory was
deleted and `cargo clean` was never run, so the other developer's cache is
untouched. The planner may want a per-branch target dir (or a note that
the shared one is single-developer only) in `AGENTS.md`.

**Not verified, and why.**

- `cargo test --workspace` and `cargo clippy --workspace`: the plan
  forbids them on this 11 GB machine while two developers build; CI runs
  them.
- `cargo test -p lakehouse-api` in full (only the three `--lib` filters
  the plan names) and every integration test (`ingest_run`, `embed_tokens`,
  `route_auth`, …): they need `lakehouse_test_support::database_url()`, i.e.
  a real Postgres, which this run does not have. Read, not run:
  `tests/ingest_run.rs::ingest_run_with_dagster_down_is_a_503_with_an_unreachable_code`
  asserts the 503 + `orchestrator_unreachable` for `spin_up`'s dead
  `127.0.0.1:1` upstream; that is a `Transport`, the arm whose behaviour is
  unchanged, so the narrowed split should not move it — but it is unproven
  here.
- `bun run test`: the plan's B2 acceptance is `typecheck` + `lint` only.
- The 502 is proven by unit tests against constructed `DgError` values, not
  against a live `Dagster` answering 4xx or a GraphQL error; no compose,
  Dagster or ClickHouse was run.
- Slice A (embed events) is not in this branch.

**Plan vs code.** No scope mismatch; the anchors held (`DgError` at 157,
`Server(format!("Dagster HTTP …"))` at 1157, `orchestrator_unreachable` at
181, all at `e17a8281`), and the file's two further non-2xx sites fall
under the plan's "any other non-2xx site". Two things the plan did not
spell out, both recorded above: the new `ApiError::BadGateway` variant
(accepted) and the `routes/connectors.rs` doc comment. Still owned by the
planner: `docs/core/features/no-raw-upstream-errors.md:52` still presents
`orchestrator_unreachable` as the answer for every orchestrator failure,
which is the line SHOULD-FIX 6 rewrites. `ops/g6/g6_ingest_matrix_test.py`
needs no change: its `RETRYABLE_ORCHESTRATOR_CODES` gate is exactly what
makes the 502 fail fast by design.

## 7. Review
