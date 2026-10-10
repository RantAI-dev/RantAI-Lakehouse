# BI-8: calculated fields — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/calculated-fields.md`. Specs:
`docs/core/specs/bi-8.md`, `docs/core/specs/ai-4.md`.

**Base:** `feat/uiux`, after `BI-16` part A is committed (the commit is named
in the developer's brief). Read first: `lakehouse-bi` `builder.rs`
(`QueryBuilder::build`, `Relation`, how a grain became a bucket expression
in `grain.rs` — a calculated dimension is the same idea), `store.rs`
(`ChartInput`, `validated_mart_columns`, the `console.*` tables and
`ensure_bi_table`), `sources.rs`, `lakehouse-api` `sql_rewrite.rs` (the role
rewrite: masks, row filters, `ReEncodeLiterals`), `routes/dashboard.rs`
(`fields`), `routes/ai/registry.rs`.

**Verification on this slice (owner's instruction, 2026-10-11):** the owner
tests by hand; the developer runs the reduced checks in section 6; the
reviewer does not re-run suites; CI runs them on the PR. The security items
in section 5 are not reduced.

**Two parts, two handoffs.** Part 1 is T1 to T6. Stop after it and hand off.
Part 2 (T7 to T9) starts when the planner says so.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Language | Own grammar, own parser in `lakehouse-bi` (a new module; no SQL parser involved): `[Column Name]` or a bare identifier for a column or another calculated field of the same source; numbers; single- or double-quoted text; `+ - * /`, comparison, `and` / `or` / `not`, parentheses; function calls, names case-insensitive. Every error carries a message and a character position. Limits: 2,000 characters, nesting depth 32, reference chain depth 8, no cycles. |
| Compilation | The syntax tree is compiled to `ClickHouse` SQL by the server from a fixed function table. A column reference is emitted only if it is a column of the source (validated identifier); text goes through the existing literal helper; numbers are re-printed from their parsed value. No substring of the user's input is copied into SQL. `/` compiles so that division by zero yields null. Date functions use `BI-9`'s `TimeContext`. |
| Types | The checker infers `number`, `text`, `date`, `datetime`, `boolean`, and whether the formula is row-level or aggregate. Mixing an aggregate with a bare row-level column is an error with its position. An aggregate inside an aggregate is an error. |
| Functions, part 1 | Maths: `Abs`, `Round`, `Floor`, `Ceil`, `Power`, `Sqrt`, `Exp`, `Log`, `Mod`, `Greatest`, `Least`. Text: `Concat`, `Upper`, `Lower`, `Trim`, `Length`, `Substring`, `Replace`, `Contains`, `StartsWith`, `EndsWith`, `Left`, `Right`. Dates: `Year`, `Month`, `Day`, `Hour`, `Weekday`, `DateTrunc(unit, d)`, `DateAdd(unit, n, d)`, `DateDiff(unit, a, b)`, `Today`, `Now`. Conditions: `If`, `Case(cond, value, …, else)`, `Coalesce`, `IsNull`, `Between`. Conversion: `ToNumber`, `ToText`, `ToDate`. Aggregations: `Sum`, `Count`, `CountDistinct`, `Avg`, `Median`, `Percentile(x, p)`, `Min`, `Max`, `StdDev`, `SumIf`, `CountIf`, `AvgIf`. One catalog in the server (name, signature, category, one-line help) is the single owner, served to the console and the assistant. |
| Storage | A new `console.bi_field` table created like the other BI tables (no Postgres migration): id, source kind and id (mart name or SQL source id), `name` (an identifier, unique per source, not equal to a column of the source), `formula`, inferred `level` and `type`, created by, updated at. |
| Routes | `GET /api/dashboard/calc-fields?mart=` / `?source=` (`dashboard:read`); `POST`, `PUT`, `DELETE` (`dashboard:write`); `POST /api/dashboard/calc-fields/validate` (`dashboard:read`: checks only, writes nothing) returning `{ ok, level, type }` or `{ ok: false, error: { message, position } }`; the function catalog on a `dashboard:read` route. `GET /api/dashboard/fields` also lists the source's calculated fields, marked as such. All in `POLICY_TABLE` and `tests/route_auth.rs`. Delete is refused (409) while a chart uses the field, naming the charts. |
| Use in a chart | `dimension`, `breakdown`, `measures` and a raw table's `columns` may name a calculated field of the chart's source. A row-level field as a measure takes the chart's aggregate; an aggregate field as a measure ignores it; an aggregate field as a dimension or breakdown is refused. The compiled expression is aliased to the field's name; filters on other columns keep applying to raw columns (the `Relation` approach used for grains). A chart using a field is rebuilt at read time, so a changed formula takes effect. |
| Permissions | The final statement goes through the same guard and role rewrite as every tile. A formula must not let a role read what the rewrite would mask, filter or deny (section 5). |
| Editor | A formula box in the chart builder ("New calculated field" from the dimension and measure pickers, and an edit entry on an existing field): suggestions for `[columns]` and functions as the user types, the function's help line, validation on pause through the validate route, the error shown under the box with the position marked. Save is disabled while the formula is invalid. Little other text. |
| Assistant (`AI-4`) | Tools: `list_calculated_fields`, `validate_formula`, `create_calculated_field` and `update_calculated_field` (low-risk writes), `delete_calculated_field` (approval, like `delete_chart`), all `dashboard:*` as above; the catalog is given in the tool description or a `list_formula_functions` tool. `tool_schemas.json` regenerated and the tool count assertion updated. |
| Part 2 | Table calculations as functions over an aggregate, evaluated over the chart's result in the chart's dimension order: `RunningTotal`, `RunningCount`, `Offset(x, n)`, `PercentOfTotal`, `Rank`, `MovingAverage(x, n)`. Period functions on a grained chart: `PreviousPeriod(x)`, `SamePeriodLastYear(x)`. `Fixed([a], [b], …, aggregate)`: the aggregate at those columns, joined back. Each is refused with a positioned message where it cannot apply (no dimension, no grain, a breakdown it cannot partition by). |

## 2. Tasks, part 1

- **T1 — Language:** tokenizer, parser, type and level checker, compiler, catalog; unit tests for every function, every error class with its position, and the limits.
- **T2 — Store and builder:** `console.bi_field`; resolving a field name in `dimension` / `breakdown` / `measures` / `columns`; always-rebuild; deletion-in-use check.
- **T3 — API:** the routes, policy rows and auth tests; `fields` listing; preview; export (`chart_def_fields` needs nothing if names suffice — say so); assistant tools and fixture.
- **T4 — Console services:** contracts, client, service.
- **T5 — Console editor and pickers:** the formula box, suggestions, help, error marking; fields in the pickers with a mark; `previewKey` covers what changes the preview.
- **T6 — Docs:** `CHANGELOG.md` `[Unreleased]`.

## 3. Tasks, part 2 (do not start until told)

- **T7 — Table calculations** in the checker, compiler and builder (a wrapping statement over the grouped result).
- **T8 — Period comparisons and `Fixed`.**
- **T9 — Console:** catalog entries, help, the builder's refusals shown; docs.

## 4. Out of scope (do not build)

Raw SQL in a formula; cross-source formulas; governed metrics (`BI-2`); formulas in dashboard filters; a separate "fields" management page (the builder is the entry point).

## 5. Things the developer must verify, not assume (not reduced)

Against the dev `ClickHouse` (26.8) and the real role rewrite, with statements and results quoted in the handoff:

- **No injection path.** Formulas whose text, column names or quoted strings contain quotes, backslashes, comment markers, semicolons, backticks and non-ASCII: each is either refused with a position or compiles to a statement in which the text appears only inside a correctly escaped literal. Include a property-style test over generated nasty strings.
- **The role rewrite still holds.** With a role that has a masked column and a row filter on the demo table: a formula over the masked column returns masked values; an aggregate over it aggregates the masked values or is refused, whichever the rewrite does for a plain chart today (say which); a denied column fails closed. If the rewrite cannot see through the compiled statement shape, stop and report — do not ship a bypass.
- Division by zero, null handling in `If` / `Coalesce`, `Percentile` bounds, `DateDiff` sign, `Today` in the report zone.
- That every catalog function compiles and runs on a small table at least once (one generated test statement per function).
- That `console.bi_field` creation is idempotent and counted in the DDL-count test.

## 6. Checks (reduced, by the owner's instruction)

Per task: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, the unit tests of the files touched; `bun run typecheck`, `bun run lint`, the bun test files touched. At the end of each part: one `cargo test -p lakehouse-bi`, one `cargo test -p lakehouse-api --lib`, `--test route_auth --test sec11_guard --test security_regressions`, one full `bun run test`.

## 7. Handoff (developer appends one entry per part)

### Partial notes (developer, uncommitted)

- 2026-10-11 T1 done (not committed): `lakehouse-bi/src/formula/{mod,lex,parse,catalog,compile}.rs` (new), `lib.rs` (`pub mod formula`), `grain.rs` (two small public helpers on `TimeContext`, `in_zone_expr` and `now_expr`, and `Grain::bucket_expr_over`, which `bucket_expr` now calls; behaviour unchanged). 36 formula tests pass; clippy `-p lakehouse-bi --all-targets -D warnings` clean. All 85 generated statements (every catalog example plus edge cases) ran on the real engine, 0 failures; two engine behaviours found and handled (`substringUTF8` refuses a start of 0; `toInt64OrNull` refuses a number).

- 2026-10-11 T2 done (not committed): `lakehouse-bi/src/fields.rs` (new: `FieldDef`, `console.bi_field` CRUD, `FieldCatalog`, `prepare`, `check`, `charts_using`, `fields_using`, name rules), `builder.rs` (`Relation::Calculated`, `AggregateField`, `ReadContext.fields`, `agg_of`/`build_kpi_sql`/`grained_sql` use an aggregate field, `report_over` shared by the mart and source paths), `store.rs` (`bi_field` DDL, 18 to 19 in the DDL-count test; `Resolved.fields`; `spec_from_resolved` applies `prepare`), `tables.rs` (one match arm), `lakehouse-alerts/src/lib.rs` (digest reads the catalog for source tiles). `cargo test -p lakehouse-bi`: 265 lib + 1 pass; clippy `-p lakehouse-bi --all-targets -D warnings` clean.
- 2026-10-11 T3 to T6 done (not committed), see the handoff below.

### Handoff, part 1 (developer, uncommitted, nothing committed or pushed)

**Files by task**

- T1 `lakehouse-bi/src/formula/{mod,lex,parse,catalog,compile}.rs` (new), `lib.rs`, `grain.rs` (`TimeContext::in_zone_expr`, `now_expr`, `Grain::bucket_expr_over`; `bucket_expr` calls it, behaviour unchanged). Checker and generator are one pass in `compile.rs` (deviation from "checker, compiler" as two pieces: one pass keeps "valid" and "this SQL" from disagreeing).
- T2 `lakehouse-bi/src/fields.rs` (new), `builder.rs`, `store.rs`, `tables.rs`, `lakehouse-alerts/src/lib.rs`.
- T3 `lakehouse-api/src/routes/calc_fields.rs` (new), `routes/mod.rs`, `policy.rs`, `routes/dashboard.rs` (`fields` returns `calculated`; board read loads the catalog; records route applies the fields it names), `routes/embed.rs`, `routes/dashboard_sources.rs` (deleting a source deletes its fields), `routes/ai/{registry,gate,audit,prompt}.rs`, `routes/ai/tools/{mod,calc_fields}.rs`, `tests/fixtures/tool_schemas.json`. `chart_def_fields` (YAML export) needs nothing: a chart names a field like a column, so its name is exported; the formulas themselves are not exported.
- T4 `src/services/{contracts,clients}/calc-fields.ts` (+ test), `services/index.ts`.
- T5 `src/lib/formula-assist.ts` (+ test), `src/features/dashboards/formula-editor.tsx`, `chart-builder.tsx`, `src/lib/preview-key.ts`.
- T6 `CHANGELOG.md`.

**Commands and counts (all foreground or awaited; nothing committed)**

- `cargo fmt --check -p lakehouse-bi -p lakehouse-api -p lakehouse-alerts`: clean (`cargo fmt -p` run on them; every file touched was rustfmt-clean at HEAD, so only my hunks moved; `git status` shows no unexpected file).
- `cargo clippy -p lakehouse-bi --all-targets -- -D warnings`, and `-p lakehouse-api -p lakehouse-alerts --all-targets -- -D warnings` (after `touch` of each `lib.rs`): clean.
- `cargo test -p lakehouse-bi`: 265 lib + 1 pass, 1 ignored (the statement writer). Before this work: 210 + 1.
- `cargo test -p lakehouse-api --lib`: 1578 passed, 0 failed, 1 ignored. Before: 1574.
- `cargo test -p lakehouse-api --test route_auth --test sec11_guard --test security_regressions`: 32 + 4 + 10 pass. `route_auth` sweeps `POLICY_TABLE` itself, so the six new rows are asserted both ways (403 for a principal without the permission, never 401/403 with it) without a hand-written case.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, the same 6 warnings. Full `bun run test`: 1213 pass, 1 skip, 0 fail (1214 tests, 136 files). Before: 1201.
- `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored` once: `tool_schemas.json` +135 lines (six tools, nothing removed); tool count assertion 64 to 70.

**Section 5, verified against ClickHouse 26.8.9.10 and the real `sql_rewrite::enforce`**

1. *No injection path*: PASSED. Property test `generated_hostile_text_never_reaches_sql_outside_an_escaped_literal`: 20,000 generated strings (quotes of both kinds, backslash, backtick, semicolon, comment markers, brackets, newline, tab, `%`, non-ASCII, emoji) used as quoted text, as a bracketed column name, as free formula text, as an operator position and as a unit; about 12,000 were refused with a position, the rest compiled. Every compiled statement is audited: outside a literal only ASCII letters, digits, space and `_(),.+-*/<>=!` may appear, no `--` or `/*`, every literal closed, and a text literal decodes back to exactly the typed text. A column that is not a plain identifier (space, quote) is refused at its position, never written. Units are written from a closed list.
2. *Role rewrite*: PASSED. Role with `place` masked and row filter `provinsi = 'Bali'` on `serving.mart_demo_map_points` (the rewrite's own mask is `replaceRegexpOne(toString(col), '(?s)^.*$', '***')`; there is no per-column "deny" in the rewrite, only masks, row filters and refusals). Statements built by the builder, rewritten by `enforce`, run by hand:
   - `Upper(place)` as dimension, sum of visitors: one group `***`, 7997 (Bali only).
   - `Length(place)` as measure by category: Office 6, Retail 3, Service point 3, Warehouse 6 (3 characters of `***` per Bali row).
   - `CountDistinct(place)` by category: 1 each. `Sum(visitors) / CountDistinct(place)` as KPI: 7997. `Count()`: 6 (the filtered rows). `StartsWith(place, 'A')` as dimension: one group `0` (the masked text).
   - Role with `visitors` and `visit_date` masked: `visitors * 2` as measure fails in the engine (`Illegal types String and UInt8 of arguments of function multiply`); `Sum(visitors)` as KPI fails (`Illegal type String of argument for aggregate function`); `Year(visit_date)` fails (`Illegal type String`); a plain chart summing the masked `visitors` fails the same way. So a formula or aggregate over a masked column sees the masked value, and over a masked number is refused by the engine as a plain chart is; nothing clear is reachable. The tile shows the classified error, not the text.
   - Fail closed: obligation with `real_columns: None` gives `UnprovableSubstitution`; an invalid row filter gives `InvalidRowFilter`, with a calculated field in the statement.
   - Structure test: the raw table name appears exactly as often as without the field, only inside the masked, filtered projection, and the calculated expression is outside it. Every catalog example, compiled and rewritten under a mask and a row filter, parses for the rewrite (`quantileExact(p)(x)` included), reads only its own table and is a fixed point of a second rewrite.
3. *Edge behaviour*, run on the engine over a scratch table (`scratch_bi8.t`, dropped afterwards): `amount / 0`, `amount / (qty - qty)`, `Mod(x, 0)`, `Sqrt(negative)`, `Log(0)`, `Log(negative)`, `Sum(x) / 0`: all empty (NULL). `If(IsNull(label), 'none', label)` and `Coalesce` give `none` for the NULL row. `Percentile(x, 0)` 50, `(x, 0.5)` 100, `(x, 1)` 250 over 100/50/250; a value outside 0 to 1 is refused at the argument. `DateDiff('day', 01-01, 01-11)` is 10 and the reverse -10; `DateDiff('month', 31 Jan, 1 Feb)` is 1 (boundaries). `Today()` is 2026-10-11 in `Asia/Jakarta` through the report time zone; `Hour(ts)` of 10:30 UTC is 17. `Round(2.5)` is 2 (the engine rounds halves to even; help text says so). `qty - amount` over unsigned columns gives -250 (no wrap).
4. *Every catalog function runs*: PASSED. 85 generated statements (53 examples plus 32 edge cases) run on the engine, 0 failures after two fixes found by this run: the engine rejects `toInt64OrNull` on a number (now `toInt64`) and rejects `substringUTF8` with a start of 0 (`Substring` clamps to 1, `Right(s, 0)` is written as an empty text). Test `every_catalog_example_compiles_with_the_level_the_catalog_says` pins the compile side.
5. *`console.bi_field` creation*: PASSED. Idempotent `CREATE TABLE IF NOT EXISTS` inside `ensure_bi_table`; the DDL-count test is now 19 (was 18).

**Existing tests changed**

- `routes/dashboard.rs` `records_pages::mount_mart` gained one mock (`console` answers with no rows): a column the relation lacks is now looked up among the source's calculated fields before it is refused, which reads `console.bi_field`; the two tests that failed (`a_column_the_relation_lacks_is_a_400_with_our_message`, `a_raw_table_page_refuses_a_column_the_relation_lacks_and_a_drill_beside_it`) assert the same 400s as before.
- `store.rs` DDL test: 18 to 19 statements. `registry.rs`: tool count 64 to 70 (test renamed). `src/lib/preview-key.test.ts`: `base` gained `calc: ""` (the key has a new input). `builder.rs` tests: two `ReadContext` literals gained `fields`. `tool_schemas.json` regenerated by the existing writer (additions only).

**Deviations and choices where the plan was silent**

1. Field name is immutable on `PUT` (charts refer to it); `PUT` takes `id` and `formula`. A level change (row to aggregate or back) is refused with 409 while a chart or another field uses it. Delete is refused (409) while a chart or another field uses it. Deleting a SQL source deletes its fields.
2. `GET /api/dashboard/fields` returns the fields in a separate `calculated` array, not mixed into `dimensions` / `measures`; the builder merges them (per-row number or aggregate as measure, other per-row as dimension, marked `fx`).
3. Aggregate fields are accepted only as a measure of a chart built by `QueryBuilder` or the KPI/gauge builder; pivot values, raw-table columns, point maps, box plots and a comparing KPI take per-row fields only (refused at save with a message; at read the chart keeps its stored statement). An aggregate field on a grained chart that reads the grouped date column builds no statement (it would read the bucket); reported as above.
4. If a field's formula no longer checks out at read time: a mart tile runs its stored SQL, a SQL-source tile shows the existing "definition is invalid" error. This is the same fallback as a corrupt definition; the field's own message is not shown on the tile.
5. The records route (view records, drill on a field, table page) applies only the fields it names (`column`, `columns`, `sortColumn`), not all.
6. The alert digest of a *mart* KPI uses the statement stored with the chart (as before), so a changed formula reaches it only when the chart is saved again; a SQL-source tile is rebuilt with the field catalog.
7. Assistant: six tools instead of five (`list_formula_functions` carries the catalog, so the descriptions stay short); `delete_calculated_field` is `WriteHigh` (approval) like `delete_chart`; dashboard domain words and the build prompt mention the tools; the confirm and approval summaries are in `gate.rs`. `AI-15` (standard request set) was not touched; the dashboard page sends no new context.
8. A boolean column reads as text in a formula (the column kind has no boolean); `true` / `false` literals exist.

**Not verified**

- Nothing was seen in a browser; the formula box is not rendered by any test (suggestion, help and marking logic is pure and tested in `formula-assist.test.ts`). The editor dialog is nested inside the chart builder's dialog; focus behaviour of a nested dialog is unchecked.
- Role rewrite checked with a fake obligations source and the real `enforce`, not with a policy row in Postgres through the HTTP route (`run_spec_sql` calls the same `enforce`).
- Workspace-wide clippy and test, other `lakehouse-api` integration files, `docker compose`, gates (CI).
- Embeds and public links with a field-using chart end to end (they share `stored_chart_sql` with the dashboard; the catalog load is the only new code).
- The routes `calc_fields::{create,update,delete}` against a mocked or real ClickHouse (only auth, compile and the pure functions are tested).

**To check in a browser**

1. New chart on a mart: under the source picker click **New calculated field**, name `profit`, formula `[a] - [b]` with two real numeric columns; while typing, suggestions appear and a function shows its help line; the status line says "One value per row, number".
2. Type `[a] - ` and pause: the error marks the end; type `Sum([a]) / [b]`: the error marks `[b]`; Save stays off in both. Create works only with a valid formula and shows a 409 sentence for a name already used.
3. The field shows `fx` in the Measure picker; pick it as a bar's measure: the preview shows the sum of the formula. Pick an aggregate field (`Sum([a]) / Count()`) as a measure: its own value per group, aggregation setting ignored.
4. An aggregate field as dimension is not offered; a per-row text field (`Upper([label])`) can be the dimension, and a date field can take Group by.
5. Click an `fx` chip: edit the formula, Save; the preview changes without reselecting. Delete while a chart uses it: the sentence names the chart.
6. As a role with a masked column: a formula over it shows `***` derived values; over a masked number the tile shows the generic failure with a reference id.
7. A user without `dashboard:write` can open the box and see validation, but Create fails with a plain message.
8. Assistant (Build mode): "add profit as revenue minus cost on mart X" runs `create_calculated_field` after Confirm; deleting one waits in Approvals.

### Fix handoff (developer, uncommitted)

- R1 (BLOCKER) cause: the console composer sends an explicit allowlist of tool names with every turn (`toolsFromCaps(enabledCaps, mode)` in `src/features/copilot/use-copilot.ts`, built from `CAPABILITIES` in `src/features/copilot/capabilities.ts`), and the server offers only those. The six calculated-field tools were in the registry and the prompt domains but not in the "Dashboard Builder" capability, so they never reached the model. Fix: the six names added to that capability (`capabilities.ts`, cites R1). Test in `capabilities.test.ts`: reads the server's `tool_schemas.json` and asserts that every chart, board, SQL-source, mart and calculated-field tool the server has is offered by the dashboard capability in Build mode, and that the capability names no tool the server lacks. The other `dashboard:*` tools added on this branch were already listed (BI-9, BI-16 part A and BI-18 changed existing tools only, whose schemas come from the registry; none is sent stale).
- R2 (SHOULD-FIX): the 409 now names charts by title and dashboard ("the chart \"Sales\" on Main", the built-in board is Main), no ids, at most 5 then "and N more". `lakehouse_bi::fields::name_charts` (tested in `fields.rs`), used by `routes/calc_fields.rs`; fields that use the field read "the field x".
- Checks: `cargo fmt --check -p lakehouse-bi -p lakehouse-api` clean; `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -D warnings` clean (after touching lib.rs); `cargo test -p lakehouse-bi --lib fields::` 18 pass; `cargo test -p lakehouse-api --lib` 1578 pass; `bun test src/features/copilot/capabilities.test.ts` 4 pass; `bun run typecheck` clean. Not run: full `bun run test`, lint (the only console change is a list and a test).
- Not verified: the assistant in a browser; the reworded 409 against a live database. The user must reload the console (the allowlist is built in the browser bundle) after the console is rebuilt.

## 8. Review (planner appends)

### BI-8 part 1 — findings from the product owner's QA (reviewer, 2026-10-11)

The reviewer did not re-run the suites (owner's instruction); the developer's counts are in the handoff, CI runs the rest.

QA passed: `BI-8-AC1` (a per-row field charts correctly), an aggregate field as a measure, `BI-8-AC3` (the error at its position, Save disabled), a field as a dimension, reuse in a new chart, deletion refused while a chart uses the field.

- `BLOCKER` R1 — `AI-4-AC1` fails. Asked to add a calculated field, the assistant (Build mode, dev console) answers that the session exposes no calculated-field tools (`create_calculated_field`, `validate_formula`, `list_formula_functions`). The tools are in the registry and the fixture, so something between the registry and the model's tool list leaves them out (a per-mode or per-page tool list, a tool picker default, a gate, a cached session). Find the cause, fix it, and prove with a test at that layer that every `dashboard:*` tool added on this branch (`BI-18` part B, `BI-9`, `BI-16` part A, `BI-8`) reaches the model for a principal who holds the permission.
- `SHOULD-FIX` R2 — The refusal to delete a field in use names the chart by id ("still used by chart u_…"). Name it by title, with the dashboard's name when there is one.
- Accepted as built: "New calculated field" and the field chips sit under Data source, not inside the dimension and measure pickers.

### BI-8 part 1 — closed (reviewer, 2026-10-11)

R1 and R2 are closed, confirmed by the product owner in a browser: the assistant listed the functions, validated the formula and created `double_visitors` after a confirmation (`AI-4-AC1`), the field shows in the builder, and the refusal names the chart and its dashboard. No open `BLOCKER` for part 1.

- `SHOULD-FIX` R3, for part 2 — a field created through the assistant is stored with an empty `createdBy`; record the principal as the HTTP route does.
- Not verified by anyone: `BI-8-AC4` and `AC5` in a browser (a user without `dashboard:write`; a masked role), an embed or public link with a field-using chart, the workspace-wide suites (CI).
