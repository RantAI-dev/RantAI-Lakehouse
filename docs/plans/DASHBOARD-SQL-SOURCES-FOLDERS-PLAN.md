# Dashboard: SQL sources, folders, page-aware Copilot — plan

Status: **in progress** on `feat/uiux` — step 0, SQL sources (backend, console, Copilot) and folders stage 1 done; page-aware Copilot next.
Scope decided 2026-09-26: SQL sources (custom SQL + multi-mart charts),
dashboard folders (stage 1), page-aware Copilot (text context). Out of
scope this round: new chart kinds, vision, folder permissions, revision
history, join builder without SQL.

Research behind the choices (Metabase, Superset, Tableau, Looker, Power BI,
Grafana): `presentasi/dashboard-bi-comparison/` (not committed).

## 0. Facts this plan rests on (verified in code)

| Fact | Where |
|---|---|
| Charts and boards live in ClickHouse `console.bi_chart` / `console.bi_board`, created by idempotent DDL, not Postgres migrations | `lakehouse-bi/src/store.rs:367-417` |
| Every update is an INSERT into a `ReplacingMergeTree(created_at)`; deletes are tombstones; reads use `FINAL WHERE is_deleted = 0` | `store.rs:540-562, 1609-1655` |
| Chart SQL hard-codes `FROM serving.{mart}`; filters key off `def.mart` columns | `lakehouse-bi/src/builder.rs:178, 204, 221-374` |
| Tiles run through `run_spec_sql` → `policy_engine::rewrite_sql_for_roles` (masking + row filters, applied to tables inside subqueries too) | `routes/support.rs:186-221`, `sql_rewrite.rs` |
| **Drill-down `/api/dashboard/records` runs `SELECT * FROM serving.{mart}` with plain `ch.query` — no policy rewrite, and `ApiError::Internal(err.to_string())`** | `routes/dashboard.rs:727-734` |
| Query Studio read-only guard `is_read_only` is private to `query.rs`; row cap 2 000 is applied in Rust after the query ran | `routes/query.rs:73, 427, 519-521` |
| Server-side capped wrapper already exists: `SELECT * FROM (\n<sql>\n) LIMIT n SETTINGS max_result_rows=n, result_overflow_mode='break'` | `routes/query.rs:1150-1156` (`build_capped_statement`) |
| Saved queries are Postgres `saved_query`, shared by the whole team, no update/delete routes | `migrations/0005`, `0046`; `lakehouse-store/src/queries.rs` |
| One ClickHouse user for everything; user SQL can read `console.*` and `system.*` today (table functions refused) | `lakehouse-clickhouse/src/lib.rs:131-173`, `sql_rewrite.rs:1580` |
| One deployment = one tenant; no tenant scoping in repositories | ADR 0003, `tests/security_regressions.rs:433-471` |
| No role except `*:*` holds `dashboard:write` | `migrations/0002_seed_identity.sql:38-46` |
| Page context sent to the model is truncated to **800 chars** and carries tile titles only, no values | `routes/ai/mod.rs:231-241`, `dashboard-page.tsx:259-279` |
| `lakehouse-llm` messages are text-only (`content: String`) — no vision | `lakehouse-llm/src/lib.rs:16-21, 236-252` |
| "dataset" is already taken (AI tools `list_datasets`/`describe_dataset` = catalog tables, `DatasetSla`) | `routes/ai/registry.rs:112-125` |
| Highest migration on this branch: `0048`; the other checkout adds `0049_upload.sql` | `rust/migrations/` |
| Dashboards frontend bypasses `@/services` (direct `apiFetch`, types from `clients/bi-store`) | `src/features/dashboards/*` |

## 1. Naming

"SQL dataset" collides with catalog datasets in the Copilot tool namespace.
Use **SQL source** (UI: "Sumber SQL"), code `sql_source`, id prefix `s_`.
Permission **`dashboard:sql`** — "may author SQL sources". Viewing a chart
built on a source needs only `dashboard:read`.

## 2. Step 0 — fix drill-down policy bypass (security, lands first)

`/api/dashboard/records` must go through the same enforcement as tiles:
build the statement, run it via `run_spec_sql`-equivalent rewrite for the
principal's roles, and classify errors instead of `err.to_string()`.
Test in the `run_spec_sql_enforcement` style: a masked column comes back
masked through `/records`. Cite the finding at the fix site. Separate commit
(and ideally a separate small PR) because it is a fix, not a feature.

Also checked while here, not fixed in this plan: `/values` and `/fields`
read `system.*`/distinct values without rewrite (distinct values of a
row-filtered column can leak) — record as a follow-up finding.

## 3. SQL sources (backend, Rust)

### 3.1 Storage — ClickHouse, next to charts
`console.bi_source` in `ensure_bi_table_uncached`, same pattern as
`bi_board`: `id, title, sql, columns_json, folder_id DEFAULT '', created_by,
created_at, is_deleted`, `ReplacingMergeTree(created_at) ORDER BY id`.
No Postgres migration needed. Why ClickHouse and not `saved_query`: charts
reference sources by id at render time from the same store, and the BI
store's upsert/tombstone semantics already exist.

### 3.2 Validation on create/update (fail closed)
1. `is_read_only` — move it (and `strip_sql_noise`) from `query.rs` into a
   shared `sql_guard` module used by both; do not copy it (AGENTS rule 4).
2. **Only `serving` tables.** Reuse `sql_rewrite::referenced_tables`
   (`sql_rewrite.rs:118`, already `pub`, already what `policy_engine.rs:244`
   uses; `None` = unparseable → refused, the same fail-closed rule). Every
   returned name must be `serving.<ident>`; a bare 1-part name is refused
   too (ClickHouse would resolve it against the connection's default
   database, not `serving`). CTE aliases are already excluded by that
   function.
3. Probe columns: `DESCRIBE (SELECT * FROM (\n<sql>\n))` → store
   `[{name, type}]` in `columns_json`; numeric → measure, else dimension,
   reusing `is_numeric_type`.
4. No trailing `;`, no `SETTINGS`/`FORMAT` clause in user SQL (they would
   break the wrapper).

### 3.3 Execution
`QueryBuilder` gets a source enum instead of a bare mart:
```rust
enum From { Mart(Ident), Sql { id: String, sql: String } }
// Mart → "serving.{mart}"   Sql → "(\n{sql}\n) AS src"
```
Every built statement for a SQL source ends with
`SETTINGS max_result_rows = 2000, result_overflow_mode = 'break',
max_execution_time = 30` (row cap same as Query Studio, decided
2026-09-26). Why 30 s: Superset's SQL Lab default (`SQLLAB_TIMEOUT = 30`,
`superset/config.py`), and it sits below the 60 s route/HTTP timeout
(`routes/mod.rs:112-118`, `lakehouse-clickhouse/src/lib.rs:164`) so
ClickHouse cancels the query itself instead of it running on after the
client gave up. A named constant; revisit with a measured heavy join. Runs through `run_spec_sql`, so
masking/row filters apply to the marts inside the subquery. Test that
explicitly.

`sql_with_filters` / `build_kpi_sql`: look up filter columns from the
source's `columns_json` instead of `mart_columns()[def.mart]`.

### 3.4 Chart input
`ChartInput` gains `source: Option<String>` (serde default). Exactly one of
`mart` / `source` for non-text kinds, else `BadRequest`. Existing stored
charts deserialize unchanged.

Drill-down on a SQL-source tile: **unsupported, honestly** for this round —
the tile menu hides "Drill", and `/records?source=` returns
`supported: false` with a message.

### 3.5 Routes (all in `POLICY_TABLE`, asserted in `route_auth.rs`)
| Method | Path | Policy |
|---|---|---|
| GET | `/api/dashboard/sources` | `dashboard:read` |
| POST | `/api/dashboard/sources` | `dashboard:sql` |
| PUT | `/api/dashboard/sources` | `dashboard:sql` |
| DELETE | `/api/dashboard/sources` | `dashboard:sql` — refuse (409) while charts reference it |
| POST | `/api/dashboard/sources/preview` | `dashboard:sql` |

`/api/dashboard/fields` also returns `sources: [{id, title}]`, and
`?source=` returns its dimensions/measures.

Permission seeding: none this round — only `*:*` gets `dashboard:sql`, same
as `dashboard:write` today. Granting it to Analyst later = migration `0050+`
(not `0049`, taken by the other checkout).

## 4. Folders (stage 1)

- `console.bi_folder`: `id (f_<8hex>), name, parent_id DEFAULT '',
  created_by, created_at, is_deleted`.
- `bi_board` and `bi_source`: `ALTER … ADD COLUMN IF NOT EXISTS folder_id
  String DEFAULT ''` (`''` = root). `Board` gains `folder_id: Option<String>`.
- Rules, enforced server-side: max depth 4 (Grafana's limit), no cycles on
  move, delete of a non-empty folder → 409, names unique per parent.
- Routes: `GET/POST/PUT/DELETE /api/dashboard/folders` (`dashboard:read` /
  `dashboard:write`); moving a board = `PUT /api/dashboard/boards` with
  `folderId`.
- Not in stage 1: per-folder permissions, personal folders (boards have no
  owner column today), revision history.

## 5. Frontend

New code follows the layering rule; existing dashboard code is not
refactored wholesale in this PR.
- `src/services/contracts/dashboards.ts`: `SqlSource`, `SqlSourceColumn`,
  `DashboardFolder`, `DashboardService` interface; client in
  `src/services/clients/dashboards.ts`; bound in `services/index.ts`.
- **Query Studio**: "Simpan sebagai Sumber SQL" next to "Save query"
  (visible with `dashboard:sql`), a `CreateSheet` with title + folder.
- **Chart builder**: the "Mart (Gold)" select becomes "Sumber data" with two
  groups — Marts / Sumber SQL; fields load from `?source=` for sources.
  Tile badge shows the source title.
- **Dashboard switcher → browser**: board list grouped by folder (tree),
  actions: new folder, rename, delete (empty only), move board to folder.
- Pure logic (folder tree build, depth/cycle check mirror, source-vs-mart
  payload) in `src/lib/*` with `*.test.ts`.

## 6. Page-aware Copilot (text, no vision)

- Backend: raise the 800-char context cap to a named constant sized from
  the model's budget (`max_tokens` is 1200 today; measure before choosing),
  with a why-comment.
- Dashboard page: context includes, per tile, kind, source/mart, active
  filters and the first few rows of its already-loaded result (no extra
  queries). Summariser is a pure function in `src/lib` with tests.
- Query Studio page: current SQL + column names + first rows of the last
  result.
- Tools: `create_chart`/`update_chart` accept `source`; new Read tool
  `list_sql_sources`. Creating a source from chat is not added this round.
- Vision: unsupported — `lakehouse-llm` is text-only; revisit only if the
  configured provider accepts image parts.

## 7. Order of work and PRs

1. PR A — Step 0 drill-down fix (+ test).
2. PR B — SQL sources: shared `sql_guard`, `bi_source`, builder `From`
   enum, routes, Query Studio action, chart builder picker, AI `source` arg.
3. PR C — folders stage 1.
4. PR D — Copilot context.

Each PR runs the full AGENTS.md verification block.

Dev setup: `lakehouse-api` from this worktree runs with `cargo run` on the
host (port 18081), no new container. It reuses the running ClickHouse
(8123) — real `serving` data; our `console.bi_*` DDL is additive
(`IF NOT EXISTS`, defaulted columns). It cannot share the running Postgres
database: that one has migration `0049` applied by the other checkout, and
`sqlx::migrate!` refuses to boot a binary missing an applied version. It
gets its own database in the same Postgres container instead. The Next dev
server on 3100 points `RUST_API_URL` at 18081.

## 8. Open questions

1. Measure a heavy join over the largest marts to confirm the 30 s cap.
2. Should saved Query Studio queries and SQL sources merge later (one
   object, Metabase-style), or stay separate? Separate for now.
3. Follow-up findings to file: `/values` + `/fields` without rewrite; the
   report that `enforce` passes role names where permissions are expected
   (`sql_rewrite.rs:1913-1916`, `2444-2450`) — unverified.
4. Follow-up finding: the alerts digest (`lakehouse-alerts` `digest_text`)
   runs each KPI tile's stored `spec.sql` with plain `ch.rows`, without the
   policy rewrite — for every chart, not only SQL-source ones. For a
   SQL-source KPI it also uses the SQL stored with the chart rather than
   the source's current text. Not changed in this plan.
5. Known test failure on `main`, not caused here:
   `state::tests::connector_secret_resolver_admits_...` fails in the full
   `lakehouse-api` run and passes alone (order-dependent environment).
