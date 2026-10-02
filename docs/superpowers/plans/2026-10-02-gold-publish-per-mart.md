# Gold publish-to-Iceberg as a per-mart option — Implementation Plan

**Status:** approved direction, not started. Written 2026-10-02 by the
planner (Claude Opus) for a developer agent, under the role split in
`AGENTS.md` ("Who plans, who writes, who reviews"). No code has been written
for this plan.

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
  `lastChangedAt <= lastExportedAt`: do not export; return
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
  - State line: "Up to date" when `lastChangedAt <= lastExportedAt`; "Out
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

_Empty._

## 8. Review (planner appends findings per PR)

_Empty._
