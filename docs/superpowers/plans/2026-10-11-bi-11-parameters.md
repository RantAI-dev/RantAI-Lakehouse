# BI-11: parameters — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/parameters.md`. Specs:
`docs/core/specs/bi-11.md`, `docs/core/specs/ai-6.md` (the `BI-11` row).
`QS-5` (named parameters in SQL) does not exist; the "In SQL" row of the
spec is out of this plan and reported as not met.

**Base:** `feat/bi-phase-2`, after `BI-10` is committed (named in the
developer's brief). Read first: the phase 2 plans before this one and
their handoffs; `lakehouse-bi` `formula/` (lexer, parser, checker,
compiler, catalog), `fields.rs`, `shape.rs`, `filters.rs`; how the
dashboard's `grain` travels (`DashboardQuery`, the page URL, "Save as
default", embeds): a parameter value travels the same way;
`src/lib/dashboard-filter-state.ts`, `filters/filter-bar.tsx`,
`grain-switch.tsx`, `chart-option.ts`.

**Verification:** as for `BI-6`. Section 4 here is not reduced: a parameter
is viewer-supplied input that ends up in a statement.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Definition | A deployment-wide parameter in a new `console.bi_param` table (created like the other BI tables): `name` (an identifier, unique), `label`, `type` (`number`, `text`, `date`, `list`), `default`, and by type: `min` / `max` (number, date), `maxLength` (text, at most 200), `values` (list: 1 to 200 entries, each `{ "value": …, "label": … }`, all text or all numbers). Routes under `/api/dashboard/params` (`GET`: `dashboard:read`; `POST`, `PUT`, `DELETE`: `dashboard:write`), in `POLICY_TABLE` and `route_auth`. `name` and `type` are immutable after creation. Delete is refused (409) while a formula, a chart or a dashboard uses it, naming them by title. |
| Value | Per request: `params=<JSON object name → value>` on `GET /api/dashboard`, the preview, records and export routes. Each value is validated against the definition (type, range, length, membership in the list) and refused (400, fixed sentence naming the parameter) when it does not fit; an unknown name is refused. A parameter with no value sent takes the board's saved value, else its default. |
| On a dashboard | A board lists the parameters whose controls it shows and their saved values, in a new `params_json` column on `console.bi_board` (the `grain` pattern), written through `PUT /api/dashboard/boards` (`dashboard:write`) and returned in the payload with each parameter's definition. The console draws a control per listed parameter in the filter row (number input, text input, date picker, select), keeps the value in the page URL beside `f` and `grain`, and "Save as default" and Reset cover it. Embeds and public links use saved values and offer no control. |
| In a formula | `{name}` in the formula language is a parameter reference: a literal of the parameter's type, compiled from the validated value through the existing literal helper, never from the request text. The checker knows the parameter's type; an unknown name is a positioned error. A field that references a parameter records it (for the delete-in-use rule), and any chart using such a field is rebuilt at read time (already the case for fields). |
| As top-N | A chart's `limit` may be `{ "param": <a number parameter> }`. At read time the value must be a whole number within the chart kind's limit range, else the tile shows a plain sentence. |
| As a reference line | Optional `referenceLines` on a chart (bar, hbar, line, area, stacked, combo): up to 5, each `{ "value": <number> | { "param": <a number parameter> }, "label": <text, optional> }`. Returned on the tile with resolved values; the console draws them on the value axis and labels them. A plain number needs no parameter (the feature is useful on its own). |
| As a column switch | A chart's `dimension`, `breakdown` or a measure's `column` may be `{ "param": <a list parameter> }`. At save, every value of that parameter must be a column (or field) of the chart's source that is valid in that slot; at read, the value is looked up in the parameter's list and then validated again as a column of the source. The value is never used as an identifier unless it passed both. A parameter used this way has text values. |
| Not in SQL | A SQL source's or saved query's text cannot reference a parameter. The builder says "Parameters in SQL arrive with Query Studio parameters" where a user would look for it. |
| Assistant | Tools: `list_parameters`, `create_parameter` (low-risk write), `update_parameter` (low-risk write), `delete_parameter` (approval); `limit`, `referenceLines` and the `{ "param": … }` forms in the chart schemas; all tools named in `src/features/copilot/capabilities.ts`; `tool_schemas.json` regenerated, tool count assertion updated. |
| Export | `chart_def_fields` lists the new fields; a board's YAML export lists its parameters. |

## 2. Tasks

- **T1 — `lakehouse-bi`:** the parameter type, validation of a definition and of a value; the store; `{name}` in the lexer, parser, checker and compiler with a value context; `limit`, `referenceLines` and column-switch resolution in the shape and builder. Unit tests per type, per refusal, per slot.
- **T2 — API:** the routes, policy and auth tests; `params` on the dashboard, preview, records and export routes; the board's `params_json`; delete-in-use; embed and public paths; assistant tools, schemas, fixture.
- **T3 — Console services and controls:** contracts, client, service; controls in the filter row with URL state, Save as default and Reset.
- **T4 — Console builder:** a parameter manager (list, create, edit, delete) reachable from the builder and from the filter row's "Add" menu; `{` suggestions in the formula editor; "use a parameter" in the limit field and the column pickers; reference-line rows; `previewKey` and `previewSpec` carry the parameter values.
- **T5 — Console tiles:** reference lines drawn and labelled; the tile's plain sentence when a parameter value does not fit.
- **T6 — Docs:** `CHANGELOG.md` `[Unreleased]`, including that parameters do not reach SQL.

## 3. Out of scope (do not build)

Parameters in SQL text; dependent parameters; lists filled from a query; controls on embeds and public links; per-user saved values.

## 4. Things the developer must verify, not assume (not reduced)

Against the dev `ClickHouse` (26.8) and the real role rewrite, statements and results quoted:

- **No injection path.** Text and list values containing quotes, backslashes, comment markers, semicolons, backticks and non-ASCII reach the statement only inside an escaped literal; a number or date value that is not one is refused before any statement is built. Extend the property test of the formula compiler to parameter values. A column-switch value that is not in the parameter's list, or is in the list but not a column of the source, never reaches a statement.
- `BI-11-AC1`: a formula `[visitors] / {target}` with target 1000 and then 500 gives results that differ by exactly a factor of two on `serving.mart_demo_map_points`.
- `BI-11-AC2`: a list parameter switching a measure between `visitors` and a second numeric column gives each column's own totals.
- The role rewrite is unaffected: a column-switch to a masked column shows the mask (or the engine's refusal for a numeric aggregate, as a plain chart does), and cannot name a table.
- A value outside `min` / `max`, too long, or not in the list gets the fixed 400, on every route that takes `params`.
- Charts and formulas saved before this change produce the same SQL.

## 5. Handoff (developer appends)

## 6. Review (planner appends)
