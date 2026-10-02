# Gold publish-to-Iceberg as a per-mart option — Implementation Plan

**Status:** approved direction, not started. Written 2026-10-02 by the
planner (Claude Opus) for a developer agent, under the role split in
`AGENTS.md` ("Who plans, who writes, who reviews"). No code has been written
for this plan.

**Feature page (requirements and acceptance checklist):**
`docs/core/features/gold-publish-per-mart.md`, backlog `DATA-1`.

**Base commit:** `6a2b29f` on `main`. Branch off it as
`feat/gold-publish-per-mart`.

**Goal, in the user's words:** exporting Gold to Iceberg must be automatic,
and it should be an option on each mart, not a page in the Build menu that a
user has to remember to click.

**Why this exists.** Gold marts live in `ClickHouse` `MergeTree`
(`serving.*`), which only `ClickHouse` reads. The "open format, no lock-in"
promise (`GTM/ON-PREM-SALES-PLAYBOOK.md`) is only true for Gold when a copy
is written to Iceberg (ADR 0010). Today that copy depends on a manual button,
because the scheduled job cannot authenticate and only covers marts named in
an env var. Nothing inside the console reads the Iceberg copy; it exists for
outside tools.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Scope | Gold (`serving.*`) only. Silver is out of scope. |
| Default | Publishing is **off** per mart until switched on. |
| Where the option lives | On the mart's asset detail page, not in the Build menu. |
| Once on | Published automatically: after authored-pipeline success, with the nightly schedule as fallback. Unchanged marts are skipped. |
| Who may switch it | Principals holding the existing `gold:export` permission (Platform Admin's `*:*` covers it). No new role grant in this plan. Others see the state read-only. |
| Switching off | Stops future publishing only. The existing Iceberg table is **never** dropped by this feature. |
| Direction of copy | Unchanged: `ClickHouse` is the source, Iceberg the copy. Flipping it is a separate, later project. |

The last two rows of "who may switch" and "switching off" are the planner's
recommendations, taken as defaults because the user moved on without
objecting. If the user overrides them, the planner updates this table.

## 2. What exists today (anchors, verified at `6a2b29f`)

Line numbers drift; re-find by symbol if they do not match.

- **Export route:** `rust/crates/lakehouse-api/src/routes/gold.rs`
  - `check_export_token` (~line 121): passes on `gold:export`, else on a
    matching `x-run-token`, else on any `PrincipalId::Service`.
  - `export` (~207), `read_back` (~367), `exports` (~425), `consumers` (~473).
  - Every call to `export` writes one `console.gold_export_run` row
    (`gold_export_history.rs`).
- **Export mechanics:** `rust/crates/lakehouse-api/src/gold_export.rs`.
  Full re-copy per export, appended in batches, refused above
  `GOLD_EXPORT_MAX_ROWS`. Single-flight per mart via `crate::gold_lock`
  (second concurrent export gets `409`).
- **Policy:** `rust/crates/lakehouse-api/src/policy.rs` ~284–293, all four
  gold routes are `Policy::RequiresAuth`.
- **Service identities:** `rust/crates/lakehouse-api/src/main.rs`.
  `bootstrap_service_run_identity` (~517) is the shared helper;
  `bootstrap_alerts_run_service` (~410) is the closest sibling (empty
  scopes, token from config). **There is no gold-export equivalent**, which
  is why the nightly job gets `401` at `auth_gate`.
- **Config:** `Config::gold_export_run_token`
  (`rust/crates/lakehouse-api/src/config.rs` ~488).
- **Dagster job:** `dagster/dispar_orchestrate/gold_export.py`.
  `_marts_from_env` reads `GOLD_EXPORT_MARTS` (default `gold_export_smoke`);
  `_headers` sends only `x-run-token`; schedule is daily 04:00, `RUNNING`.
  The comment above the schedule documents the nightly `401`.
- **Header pattern to copy:** `dagster/dispar_orchestrate/alerts_run.py`
  `_headers` (~78) sends both `Authorization: Bearer <token>` and
  `x-run-token`.
- **Success sensor:** `dagster/dispar_orchestrate/pipeline_events.py`,
  `pipeline_run_finished_sensor` (~196), `monitored_jobs=None`.
- **Per-key settings table to mirror:** `rust/migrations/0050_pipeline_sla.sql`
  plus its store functions in `rust/crates/lakehouse-store/src/pipelines.rs`
  (`get_pipeline_sla` and its upsert).
- **Latest migration:** `0053_pipeline_definition_version.sql`. Next free
  number is `0054` — confirm no open branch has taken it before using it
  (`0049`/`0050` collided once; see the header of `0051`).
- **Gold asset identity:** `rust/crates/lakehouse-api/src/routes/catalog.rs`
  `gold_catalog_row` (~1004) emits asset `id = "serving.<mart>"`,
  `layer = "gold"`. So mart name = asset id without the `serving.` prefix.
- **Console:**
  - `src/services/contracts/gold.ts`, `src/services/clients/gold.ts`,
    exported from `src/services/index.ts` (~74).
  - `src/features/gold/gold-exports-page.tsx` (+ `.test.tsx`), routed at
    `src/app/(data)/gold-exports/page.tsx`.
  - Nav entry: `src/components/app-shell/nav-config.ts` ~133
    (`{ title: "Exports", href: "/gold-exports" }` under Build).
  - Asset page: `src/features/catalog/asset-detail-page.tsx`, tabs in
    `asset-detail-tabs.tsx`.
- **Acceptance test:** `ops/gold_export/gold_export_test.py` (uses
  `GOLD_MART_NAME`, default `gold_export_smoke`).

## 3. Tasks

One task per commit, in order. Per commit, run only the scoped checks for
what it touched (`AGENTS.md`, "Keep build time down"): for Rust that is the
changed crate (`lakehouse-api` or `lakehouse-store`), not the workspace. The
full verification block runs once per PR slice, before the handoff.

Rust is confined to T1–T4 (PRs A and B). T5–T9 touch no Rust and must not
run cargo at all.

### T1 — Let the scheduled export authenticate (bug fix, standalone)

- `main.rs`: add `GOLD_EXPORT_SERVICE_IDENTITY_NAME` (`"gold-export-scheduler"`)
  and `bootstrap_gold_export_service`, calling
  `bootstrap_service_run_identity` with `state.config.gold_export_run_token`,
  **empty scopes**, and `"GOLD_EXPORT_RUN_TOKEN"`. Call it next to the other
  bootstraps (~line 97). Doc comment must say why empty scopes are enough:
  `check_export_token` accepts the matching `x-run-token`; the identity only
  has to clear `RequiresAuth`.
- `gold_export.py`: `_headers` sends `Authorization: Bearer` and
  `x-run-token`, same shape as `alerts_run.py`. Replace the stale "gets
  `401` every night" comment with the current truth.
- Tests: a `main.rs` unit test mirroring the existing `AGENT_RUN_TOKEN`
  seeding tests (seeds one identity; second boot is a no-op); a
  `test_gold_export.py` test that both headers are sent when a token is set
  and none when unset.
- **Accept:** with `GOLD_EXPORT_RUN_TOKEN` set, a request carrying only
  those two headers gets past `auth_gate` and `check_export_token`
  (integration test in `rust/crates/lakehouse-api/tests/`, or extend
  `route_auth.rs` if that is where the sibling identities are covered).

### T2 — Store the per-mart setting

- Migration `0054_gold_publication.sql` with a why-header. Table
  `gold_publication (mart TEXT PRIMARY KEY, enabled BOOLEAN NOT NULL,
  updated_by UUID NOT NULL, updated_at TIMESTAMPTZ NOT NULL DEFAULT now())`.
  A missing row means "off".
- New module `rust/crates/lakehouse-store/src/gold_publication.rs`:
  `get(pool, mart)`, `list_enabled(pool)`, `upsert(pool, mart, enabled,
  updated_by)`. Values bound, `# Errors` on every `Result` fn, `StoreError`
  classification as in `pipelines.rs`. Tests in
  `rust/crates/lakehouse-store/tests/` using `sqlx::test`.
- **Accept:** store tests cover insert, flip, list-only-enabled, and
  unknown mart returning `None`.

### T3 — Publication routes

Add to `routes/gold.rs`, register in `routes/mod.rs`, add to `POLICY_TABLE`,
assert both ways in `tests/route_auth.rs`.

| Route | Policy | Behaviour |
| --- | --- | --- |
| `GET /api/gold/publications` | `RequiresAuth` + `check_export_token` in the handler | `{ "publications": [{ "mart", "enabled", "updatedAt" }] }`, enabled rows only. This is what the scheduler reads. |
| `GET /api/gold/export/{mart}/publication` | `RequiresAuth` | `{ "mart", "enabled", "updatedAt", "lastChangedAt", "lastExportedAt", "canEdit" }`. |
| `PUT /api/gold/export/{mart}/publication` | `RequiresPermission("gold:export")` | Body `{ "enabled": bool }`. Upserts, returns the same shape as the `GET`. |

- `lastChangedAt`: `max(modification_time)` over active parts of
  `serving.<mart>` in `system.parts`. `null` when there are no parts (a
  view, or an engine without parts) — never a guessed time.
- `lastExportedAt`: `started_at` of the newest `status = 'success'` row in
  `console.gold_export_run`; `null` if none.
- `canEdit`: whether the caller holds `gold:export`.
- `PUT` returns `404` when `serving.<mart>` does not exist. Reuse the mart
  validation `export` already performs; do not write a second one (rule 4).
- `PUT` with `enabled: false` only flips the flag. It must not touch
  Iceberg.
- Handlers return `ApiResult<ApiJson<T>>`; no upstream error text in a
  response (principle 4); no new `ApiError::Internal(err.to_string())`.
- If an existing helper records configuration changes to `audit_event`
  (`lakehouse-store/src/audit.rs`), record the toggle through it. If none
  fits, note that in the handoff; do not invent one.
- **Accept:** route tests for: non-permissioned `PUT` → 403; unknown mart →
  404; toggle on then `GET /api/gold/publications` lists it; toggle off
  removes it and the Iceberg table (if any) is untouched.

### T4 — Skip an export when the mart has not changed

- `POST /api/gold/export/{mart}` accepts `?ifChanged=true`. When set, and
  both `lastChangedAt` and `lastExportedAt` are known, and
  `lastChangedAt < lastExportedAt` (strictly; see the slice B review,
  finding B1 — the plan first said `<=`, which was wrong): do not export; return
  `{ "skipped": true, "reason": "unchanged since <rfc3339>" }` with `200`.
- If either timestamp is `null`, export (fail toward publishing, since a
  stale open copy is the worse outcome).
- A skip writes **no** `console.gold_export_run` row and commits no
  snapshot. The console's manual trigger does not pass `ifChanged`.
- **Accept:** tests for skip, for export-when-newer, and for
  export-when-unknown.

### T5 — Scheduler reads the setting

- `gold_export.py`: `list_gold_marts` fetches
  `GET /api/gold/publications` (`timeout=`, `raise_for_status()`), and
  unions the result with `GOLD_EXPORT_MARTS`. Change the env default from
  `gold_export_smoke` to empty, so a fresh deployment publishes only what a
  user switched on. Check `ops/gold_export/gold_export_test.py`,
  `.env.example` and `docker-compose.yml` for anything that relied on the
  old default and keep it working explicitly.
- `export_one_mart` passes `ifChanged=true`. A skipped response is recorded
  through `record_maintenance_run` with `skipped_verbs=["unchanged"]` and
  emits **no** `AssetMaterialization`.
- A failed publications fetch raises (retryable); it must not silently
  degrade to "zero marts".
- Update `source_metadata` `reads`/`writes` for the changed op.
- **Accept:** unit tests (no network) for: DB list plus env list are
  de-duplicated; zero marts yields zero mapped steps and a successful run;
  a skipped mart records the skip and no materialization; a fetch error
  raises.

### T6 — Publish after an authored pipeline succeeds

- New `run_status_sensor` for `SUCCESS` in `gold_export.py` (or beside
  `pipeline_run_finished_sensor`) that returns a `RunRequest` for
  `gold_export_job`. With T4 in place, unchanged marts cost one cheap check.
- It must **not** fire on `gold_export_job` itself or on maintenance,
  backup, alert, capacity, or agent jobs. Find how authored-pipeline jobs
  are identified in `authored_factory.py` and select on that; do not use
  `monitored_jobs=None` without a filter.
- `default_status` `RUNNING`, with the reason in a comment, as every other
  schedule/sensor here.
- A `409` from the single-flight lock (an export of that mart already
  running) is a skip, not a failure.
- Register in `definitions.py`.
- **Accept:** unit tests that an authored job's success yields one
  `RunRequest`, and that `gold_export_job`'s own success yields none.

### T7 — Console: the option on the mart page

- `contracts/gold.ts`: add `type GoldPublication` (camelCase mirroring the
  route; `lastChangedAt`/`lastExportedAt` as `string | null`), and
  `getPublication` / `setPublication` on `GoldService`. Implement in
  `clients/gold.ts` through `apiFetch`.
- New `src/features/catalog/open-format-card.tsx` (`"use client"` first
  line, imports from `@/services` only, `useService`/`useServiceAction`):
  - Rendered on `asset-detail-page.tsx` only when `asset.layer === "gold"`.
    Mart name is the asset id without the `serving.` prefix.
  - A switch, "Publish in open format (Iceberg)", disabled with an
    explanation when `canEdit` is false.
  - When on: last published time, snapshot ID (`getLastExport`), the last
    five runs (`listExportRuns`), and a "Publish now" action
    (`triggerExport`).
  - State line: "Up to date" when `lastChangedAt < lastExportedAt`
    (strictly; slice B review finding B1); "Out
    of date" when newer; "Not measured" when `lastChangedAt` is `null`;
    "Never published" when `lastExportedAt` is `null`.
  - Errors from the toggle are shown in place; no optimistic flip that
    survives a failed `PUT`.
- **Accept:** component tests for each state line, the read-only case, and
  a failed toggle leaving the switch where it was.

### T8 — Console: remove Exports from the Build menu

- Delete the `Exports` item from `nav-config.ts`. Check `activeNavHref`,
  `pageTitleFor`, the command palette and any nav test for references.
- Keep the `/gold-exports` route as an admin overview, linked from the new
  card ("All published marts"). Add an "Enabled" column from
  `getPublication`, and reword the page description to say publishing is
  set per mart. Update `gold-exports-page.test.tsx`.
- **Accept:** `bun run typecheck && bun run lint && bun run test` green; no
  dangling link to a removed nav item.

### T9 — Documentation

- `docs/FEATURE_COVERAGE.md`: a row for the per-mart option.
- `README.md` env table and `.env.example`: `GOLD_EXPORT_MARTS` is now an
  optional operator override with an empty default; `GOLD_EXPORT_RUN_TOKEN`
  also seeds the scheduler's identity.
- `CHANGELOG.md`: the fix (T1) and the feature.
- `GTM/ON-PREM-SALES-PLAYBOOK.md` demo step 7: switch the mart on from its
  page instead of calling `POST` by hand.
- State the known growth plainly where an operator will read it
  (`docs/OPERATIONS.md`): every export of a changed mart appends a full
  copy, and nothing expires old copies yet.

## 4. PR slicing

| PR | Tasks | Why separate |
| --- | --- | --- |
| A | T1 | A bug fix that stands alone and is worth shipping first. |
| B | T2–T5 | Backend and scheduler; behaviour changes without UI. |
| C | T6 | New sensor; easy to revert on its own. |
| D | T7–T9 | Console and docs. |

The planner opens each PR from the developer's branch, reviews it, and merges it before the next starts.

## 5. Out of scope (do not build)

- Expiring or compacting old Iceberg copies. `iceberg-rust` here is
  append-only; this needs its own design (likely the Trino path in ADR
  0009).
- Silver publishing, incremental export, or raising the row cap.
- Measuring consumers (`consumers` stays an honest `supported: false`).
- Writing Gold to Iceberg first and serving it from an accelerated copy.
- New roles or role grants.

## 6. Things the developer must verify, not assume

- That `0054` is still free on every open branch.
- That an empty dynamic fan-out lets `gold_export_job` finish successfully
  on the installed Dagster version.
- That `system.parts.modification_time` moves when an authored pipeline
  rewrites a mart the way this repo's pipelines actually write (insert,
  replace partition, or table swap). If it does not, report it; do not
  paper over it with a different heuristic.
- Whether any compose service or test depends on
  `GOLD_EXPORT_MARTS=gold_export_smoke` being the default.
- If `docker-compose.yml` changes, prove it with `docker compose up` on the
  affected profile from a clean project (rule 8).

## 7. Handoff (developer appends one entry per PR)

### PR slice A — T1 (developer, 2026-10-02)

Commits:

- `3f259ae` — `docs: add planner/developer workflow and gold publish plan`
  (AGENTS.md + this plan file, unchanged, as instructed; the Handoff section
  below is the only later edit to the plan).
- `6e46c9f` — `T1: let the scheduled Gold export authenticate`. Rust
  (`lakehouse-api`): `GOLD_EXPORT_SERVICE_IDENTITY_NAME` +
  `bootstrap_gold_export_service` (empty scopes, from
  `Config::gold_export_run_token`), called in `main` after
  `bootstrap_ingest_run_service`; three unit tests mirroring the
  `AGENT_RUN_TOKEN` seeding tests; `tests/gold_export_auth.rs` acceptance
  test (negative control 401 before seeding; both headers clear `auth_gate`
  and `check_export_token` after). Dagster: `_headers` sends
  `Authorization: Bearer` + `x-run-token` (same shape as `alerts_run.py`);
  the stale "gets `401` every night" schedule comment and the
  `GoldExportConfig.run_token` field comment replaced with current truth;
  `test_gold_export.py` gains both-headers/none-when-unset tests, a
  POST-level header test, and a corrected module docstring.

Verification — every command run in the foreground on `6e46c9f`, fresh
build, counts quoted from the run:

- `cd rust && cargo fmt --check` — pass.
- `cd rust && cargo clippy --workspace --all-targets --all-features -- -D
  warnings` — pass, no warnings.
- `cd rust && cargo test --workspace` — 76 suites, all `ok`: **2812 passed,
  0 failed, 8 ignored**.
- `python3 ops/lint/check_intra_package_imports.py` — pass.
- `python3 ops/lint/check_bare_iceberg_count.py` — pass.
- `(cd dagster && python -m pytest dispar_orchestrate -q)` — **369 passed,
  30 subtests passed, 0 failed**; the 218 warnings are all
  `importlib.resources.read_text` deprecations from `kafka-python` in
  site-packages, none from the touched files.
- Scoped runs made earlier while iterating (same final content):
  `cargo test -p lakehouse-api --bin lakehouse-api
  agent_run_service_bootstrap` — 15 passed (12 pre-existing + 3 new);
  `cargo test -p lakehouse-api --test gold_export_auth` — 1 passed;
  `python -m pytest dispar_orchestrate/test_gold_export.py -q` — 11 passed
  (8 pre-existing + 3 new).

Not run, with reason: `bun run typecheck/lint/test` and
`docker compose --profile '*' config`/`up` — T1 touches no TypeScript and
no compose file (AGENTS.md: run only the lines for the languages the PR
touched). Nothing else was skipped; no claim above is *not verified*.

Environment notes for the reviewer:

- This machine started with no Rust toolchain cache, no
  `~/.cache/rantai-dagster-venv`, and no Docker access for this user. The
  venv was created fresh from the exact pins in `dagster/pyproject.toml`
  plus `pytest`; Docker became reachable after the user added this user to
  the `docker` group (commands run under `sg docker`). Rust builds used
  `CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target` (outside the repo, no
  `cargo clean`).
- While this slice was in progress, the working tree gained planner-owned,
  still-uncommitted edits (`docs/core/**`, AGENTS.md loop steps 0 and 7,
  and a feature-page/backlog reference line near the top of this plan
  file). They are deliberately not part of this branch's commits; the only
  hunk staged from this file for the handoff commit is the one you are
  reading.

### PR slice B — T2–T5 (developer, 2026-10-02)

Branch `feat/gold-publish-per-mart-b` off `main` (slice A's `6e46c9f`
merged as PR #60 before this branch was cut). Four commits, one per task:

- `b74c7e5` — T2. Migration `0054_gold_publication.sql` (why-header; the
  columns exactly as specced, missing row = off);
  `lakehouse-store/src/gold_publication.rs` (`get`/`list_enabled`/`upsert`,
  `StoreError` classification as in `pipelines.rs`, `# Errors` on every
  `Result` fn, values bound); registered in `store`'s `lib.rs`; tests in
  `lakehouse-store/tests/gold_publication.rs` via `sqlx::test` covering
  unknown-mart `None`, insert-then-in-place-flip, and list-only-enabled
  (insert, flip, list: the "unknown returns None" accept is covered).
- `44bdfb3` — T3. The three routes exactly as the table specifies
  (policies, shapes, `404` on unknown mart, `PUT` off only flips the flag
  — no Iceberg call), reusing `mart_exists` for the existing-mart check
  (rule 4: no second validator); `lastChangedAt` from
  `max(modification_time)` over active parts (null when parts = 0),
  `lastExportedAt` from the newest `success` row (new
  `gold_export_history::last_success_started_at`; null if none), `canEdit`
  from the caller's `gold:export`. Toggles recorded through the existing
  `lakehouse-store` audit writer (`store_audit::insert`,
  action `gold.publication_set`, resource kind `gold_mart`) — no new audit
  mechanism. Routes in `POLICY_TABLE` (both-ways assertions come from
  `tests/route_auth.rs`'s existing loops over the table). New dev-deps
  `testcontainers` 0.27 / `testcontainers-modules` 0.15 (clickhouse
  feature) on `lakehouse-api` only, for route tests against a real
  ClickHouse 26.8 (the freshness query cannot be faked on a stub);
  5 tests in `tests/gold_publication.rs`, one shared container per binary
  because `gold_export_history`'s ensure-table `OnceCell` assumes one CH
  per process.
- `2623f07` — T4. `?ifChanged=true` per spec: skip only when both
  timestamps are known and `lastChangedAt <= lastExportedAt`; either null
  exports; a skip is `200 {"skipped": true, "reason": "unchanged since
  <rfc3339>"}` and writes **no** `console.gold_export_run` row, commits no
  snapshot, and never takes the single-flight lock; boolean parsed with
  `== "true"`; the freshness reads run only when the parameter is set, so
  the manual path gains no queries. Skip decision extracted as a pure fn
  (3 unit tests); the export handler's history-write block moved into a
  `record_export_history` helper to satisfy the function-length lint,
  behavior unchanged. Route tests cover skip (history count unchanged),
  export-when-newer, and export-when-unknown (both stop deterministically
  at the unprovisioned Lakekeeper token — 503, past the skip gate,
  nothing recorded).
- `21a787d` — T5. `list_gold_marts` unions
  `GET /api/gold/publications` (same two-header credential as the POST,
  `timeout=`, `raise_for_status()`) with `GOLD_EXPORT_MARTS`, dedup by
  exact mart name (a mart in both sources is one step); two distinct
  marts sanitizing to one step key still `Failure(allow_retries=False)`
  with both names and origins. Env default now empty in code, compose,
  and `.env.example`; compose comment explains the console owns the list
  and the env is an override. `export_one_mart` sends `ifChanged=true`; a
  skipped answer records `skipped_verbs=["unchanged"]` and emits no
  `AssetMaterialization`. A failed fetch raises bare (retryable); a
  malformed body is non-retryable `Failure`. `source_metadata` `reads`
  updated on the fan-out op. Gate runner checked, not edited:
  `ops/gold_export/gold_export_test.py` POSTs its own `GOLD_MART_NAME`
  directly and never reads `GOLD_EXPORT_MARTS` (section-6 item 4).

Verification — every command run in the foreground on `21a787d`, fresh
build, counts quoted from the run:

- `cd rust && cargo fmt --check` — pass.
- `cd rust && cargo clippy --workspace --all-targets --all-features -- -D
  warnings` — pass, no warnings.
- `cd rust && cargo test --workspace` — 78 suites, all `ok`: **2831
  passed, 0 failed, 2 ignored**. Full log kept at
  `/tmp/opencode/t5-workspace-test.log`. Caveat, said plainly: one
  earlier workspace run had 5 `lakehouse-auth` JWKS unit tests fail
  (port/time flake under parallel load, 39s binary); `lakehouse-auth
  --lib` passed 51/51 immediately after, and two consecutive full
  workspace runs on the final commit were fully green — the failure did
  not reproduce and is not in this slice's code.
- `python3 ops/lint/check_intra_package_imports.py` — pass.
- `python3 ops/lint/check_bare_iceberg_count.py` — pass.
- `(cd dagster && python -m pytest dispar_orchestrate -q)` — **375
  passed, 30 subtests passed, 0 failed** (218 warnings, all the known
  `kafka-python` deprecations, none from touched files).
- `docker compose --profile '*' config --quiet` — pass.
- Rule 8, on the compose edit: `GIT_SHA=$(git rev-parse HEAD) docker
  compose -p t5proof --profile dagster up -d --build dagster-code-location
  dagster-webserver dagster-daemon` from a clean project — every init
  container `Exited (0)`, the code server loaded
  `dispar_orchestrate.definitions`, `GOLD_EXPORT_MARTS=""` in the
  code-location container env, and the webserver's GraphQL listed
  `gold_export_job_schedule` with status `RUNNING`; `down -v` removed the
  project afterwards.

Section-6 answers:

- **`0054` free:** `rust/migrations` contained no `0054_*` before
  `b74c7e5`; the only branches on this remote checkout are `main` and
  this one. Other workspaces cutting from `origin/main` are out of this
  machine's sight — the migration lands with its consumer in this same
  PR, so exposure is one merge window.
- **Empty fan-out finishes:** proven by
  `test_nothing_enabled_and_no_override_runs_a_successful_zero_step_job`
  (`execute_in_process`, Dagster **1.13.20** as installed in
  `~/.cache/rantai-dagster-venv`): run succeeds, zero
  `export_gold_mart[*]` steps, `export_one_mart` never called,
  `summarize_gold_export` succeeds with `marts_exported: 0`.
- **`modification_time` moves for this repo's writers:** measured, not
  assumed, against the compose ClickHouse (`clickhouse-server:26.8`)
  image, fresh container. This repo's pipelines append Gold marts with
  plain `INSERT INTO {zone}.\`{table}\` {select_sql}`
  (`authored_factory.py`'s writer), into schema-on-write
  `MergeTree ORDER BY tuple() AS ... LIMIT 0`. Measured sequence:
  create + insert → `max(modification_time)` = write time, 1 active
  part; second insert → moves forward, 2 parts; **zero-row INSERT →
  does not move** (ClickHouse creates no part for an empty insert) and
  row count unchanged — so a rewrite that produces zero rows is
  indistinguishable from no rewrite, which is fine: no data changed, so
  "unchanged" is the true answer. Caveat for the reviewer: background
  MergeTree merges also advance `max(modification_time)` (a merge writes
  a new part stamped with the merge time) — the error direction is a
  spurious "changed" → one extra export, never a missed one.
- **Nothing else relied on the old default:** repo-wide grep for
  `GOLD_EXPORT_MARTS` hits only `gold_export.py` (override), compose
  (now `:-`), `.env.example` (now empty), and the new tests. The gate
  runner uses `GOLD_MART_NAME`. Compose change proven by the `up` above,
  not by `config` alone.

Not run, with reason: `bun run typecheck/lint/test` — slice B touches no
TypeScript (AGENTS.md: only the lines for the languages the PR touched;
T7–T8 are slice D). Nothing else skipped; no claim above is *not
verified*.

Environment notes for the reviewer:

- The ClickHouse testcontainer needs the compose env trio
  (`CLICKHOUSE_USER=default`, `CLICKHOUSE_PASSWORD=` empty,
  `CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1`, tag `26.8`): the 26.8
  entrypoint disables network access for `default` when `CLICKHOUSE_USER`
  is unset even with an empty password, and the container handle must be
  held for the binary's lifetime or it stops mid-test.
- Axum handler ordering: the `Json` body extractor must be the LAST
  argument (PUT route); `ApiResult`'s error type is `ApiRejection`, so a
  direct `Err(ApiError::x())` needs `.into()`.


## 8. Review (planner appends findings per PR)

### PR slice A — T1 (reviewer, 2026-10-02)

Reviewed `6e46c9f` and `83f5c00` against T1.

**Findings: no `BLOCKER`, no `SHOULD-FIX`.**

What was checked against the plan:

- `bootstrap_gold_export_service` goes through the shared
  `bootstrap_service_run_identity`, with empty scopes and the
  `GOLD_EXPORT_RUN_TOKEN` name, and is called beside the other bootstraps.
  The doc comment gives the reason for empty scopes, as T1 asked.
- `gold_export.py::_headers` sends `Authorization: Bearer` and
  `x-run-token`, and returns `{}` when no token is set. The stale "gets
  `401` every night" comment is replaced.
- Tests: three `main.rs` unit tests (seeds one identity and credential;
  second boot is a no-op; unset token seeds nothing), and
  `tests/gold_export_auth.rs` with a negative control (401 before seeding)
  and the positive case over the real router. The Python tests cover both
  headers, none when unset, and the POST itself.
- `docker-compose.yml` already passes `GOLD_EXPORT_RUN_TOKEN` to both
  `lakehouse-api` and the Dagster code location; no compose change was
  needed, so no `up` proof is owed.

Verification re-run by the reviewer on `83f5c00`, foreground, shared
`CARGO_TARGET_DIR`:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  pass.
- `cargo test --workspace` — 76 suites, **2812 passed, 0 failed, 8
  ignored**. Matches the handoff.
- `python3 ops/lint/check_intra_package_imports.py` — pass.
- `python3 ops/lint/check_bare_iceberg_count.py` — pass.
- `(cd dagster && python -m pytest dispar_orchestrate -q)` — **369 passed,
  30 subtests passed**.

Not verified: the schedule succeeding on a running stack. T1's acceptance
is the router-level test; the end-to-end proof is the feature's acceptance
checklist (`docs/core/features/gold-publish-per-mart.md`, step 9).

Observation, not a finding against this change: several handlers accept
"any service identity" as a fallback when their own run token is unset
(`check_export_token`, `routes::alerts::check_run_token`). Each new
scheduler identity therefore also satisfies those fallbacks on the other
routes. This pattern predates T1 and T1 follows it correctly; it is worth
its own look before more identities are added.

Reviewer environment note: the first `cargo test` run aborted because the
session's shell lacked the `docker` group, so the Postgres testcontainer
could not start. Re-run under `sg docker`, which is the result quoted above.

Merged as PR #60 (squash, `2cdbf1b`) on 2026-10-02. CI: 24 checks passed;
`cargo audit`, `cargo deny` and the history `gitleaks` job failed, identically
to `main` and to PRs #58 and #59, and this PR changed no dependencies. The
product owner approved merging on that basis and the rule in `AGENTS.md` was
changed to match (backlog `SEC-8`).

**Slice B starts from `main` at or after `2cdbf1b`**, on a fresh branch
`feat/gold-publish-per-mart-b`. The old branch was deleted on merge.

### PR slice B — T2–T5 (reviewer, 2026-10-02)

Reviewed `b74c7e5`, `44bdfb3`, `2623f07`, `21a787d`, `aa2a60a`. `origin/main`
(`ac8099b`) was merged into the branch first; it merged cleanly.

**One `BLOCKER`, one `SHOULD-FIX`. Not opening the PR until B1 is fixed.**

#### B1 — `BLOCKER` — an equal timestamp must not skip (the plan's error)

`if_changed_skip_reason` skips when `changed_ms <= exported_ms`. Both sides
are whole seconds: `system.parts.modification_time` is a `DateTime`, and
the history row stores `started_at.unix_timestamp() * 1000`
(`routes/gold.rs`, `record_export_history`). So a write that lands in the
same second an export started, after that export read the mart, compares
equal and is skipped on every later run until the mart is written again.
The open copy is then stale while the console says "Up to date" — a silent
wrong answer, the class this repo treats as most serious.

The developer implemented what the plan said: T4 specified `<=`. The plan
was wrong and has been corrected above (T4 and T7 now say strictly `<`).
The handoff's statement that the error direction is "never a missed one" is
true for background merges but not for this case.

Fix:

- Skip only when `changed_ms < exported_ms`. Equal exports. The cost is one
  extra export when a mart was written in the same second its last export
  began, which is the safe direction.
- Update `unchanged_mart_since_the_last_export_is_skipped_with_a_reason`:
  its "equal timestamps" case and comment ("the mart's last write is the
  export itself") are wrong — an export does not write the mart. Add a unit
  test that equal timestamps export.
- Update the doc comments on `if_changed_skip_reason` and `export`'s "Skip"
  section, and the T4 paragraph in `gold_export.py`'s module docstring, to
  say "strictly before".
- Cite `PR slice B review B1` at the fix site and in the commit body.

#### S1 — `SHOULD-FIX` — no Postgres should be `503`, through one helper

`publications`, `publication` and `set_publication` each repeat
`let Some(pool) = state.pg.as_deref() else { return Err(ApiError::Internal(...)) }`.
The repo's convention for an unconfigured Postgres is `503`
(`ApiError::Unavailable`; `routes::pipelines::pool`, and `README.md`:
"dependent routes return `503`"). Use one helper returning
`ApiError::Unavailable` with fixed text, and change the three sites and
their `# Errors` sections. Rule 4: check for an existing shared helper
before adding one to `routes/gold.rs`.

#### Checked and correct

- T2: migration `0054` has a why-header, matches the specified columns, and
  no other branch on `origin` has a `0054`–`0059` migration. Store functions
  bind every value and carry `# Errors`.
- T3: the three routes match the plan's table. The `PUT` is floored at
  `gold:export` in `POLICY_TABLE`; `tests/route_auth.rs` loops the table, so
  both directions are covered without a new test. Unknown mart is `404`,
  measured against `system.tables`. Switching off calls nothing in Iceberg.
  The toggle is audited through the existing `store_audit::insert`.
- T4: the skip returns before the single-flight lock, writes no history row,
  and unknown facts export. The freshness reads happen only when
  `ifChanged=true`.
- T5: the scheduler unions the API list with the env override, de-duplicates
  by name, raises on a failed fetch, and records a skip without a
  materialization. The env default is empty in code, compose and
  `.env.example`.
- The new dev-dependencies add no package to `Cargo.lock`; only feature
  edges on packages already present.

#### Verification re-run by the reviewer on `aa2a60a`

Foreground, shared `CARGO_TARGET_DIR`, Docker via `sg docker`:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  pass.
- `cargo test --workspace` — 78 suites, **2831 passed, 0 failed, 8
  ignored**. The handoff says 2 ignored; the count is 8, as in slice A. The
  JWKS flake the handoff mentions did not occur in this run.
- `python3 ops/lint/check_intra_package_imports.py` — pass.
- `python3 ops/lint/check_bare_iceberg_count.py` — pass.
- `(cd dagster && python -m pytest dispar_orchestrate -q)` — **375 passed,
  30 subtests passed**.
- `docker compose --profile '*' config --quiet` — pass.

Not re-run by the reviewer: the clean-project `docker compose up` proof for
the compose edit. The handoff describes it; the edit is one default value.

#### Observations, not findings against this slice

- **Background merges cause extra copies.** A `MergeTree` merge writes a new
  part with a new `modification_time`, so a mart that is merged after its
  export looks changed and is exported again on the next run, with no data
  change. Each such export appends a full copy. This is the safe direction
  but it weakens "skip when unchanged". A merge-proof signal needs its own
  measurement; tracked as backlog `DATA-10`.
- **This PR touches `Cargo.toml` and `Cargo.lock`.** Under the merge rule, a
  PR that touches dependencies may not merge while a dependency check is
  red, and `cargo audit` / `cargo deny` are red on `main`. Backlog `SEC-8`
  must land first (plan: `2026-10-02-sec-8-dependency-checks.md`), or the
  product owner grants an exception.
