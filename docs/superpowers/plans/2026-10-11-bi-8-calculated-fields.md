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

## 8. Review (planner appends)
