# SEC-9 Natural-language queries respect masking and permissions — Implementation Plan

**Status:** decisions signed 2026-10-10, not started. Written by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/natural-language-queries-respect-permissions.md`
(read its table first: most of the spec is already met and this plan is
small on purpose). Spec: `docs/core/specs/sec-9.md`.

**Base:** branch `fix/sec-phase-0` at `413334a`.

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Scope | One behaviour change (T1) and tests that pin what is already true (T2). No prompt changes. No new route, permission or migration. |
| Order of checks in `run_sql` | Cheap syntactic filter, then the principal check, then **the policy refusal**, then the engine dry run, then execution. Nothing the policy would refuse is sent to `ClickHouse`, not even as `EXPLAIN`. |
| How | Reuse the existing rewrite/enforcement entry point Query Studio uses to decide a refusal; do not write a second table-function or system-table check (`AGENTS.md` rule 4). The statement is still rewritten again inside `routes::query::run` when it executes. |

## 2. What exists today (anchors, verified at `413334a`)

- `rust/crates/lakehouse-api/src/routes/ai/tools/data.rs:38` `run_sql`: `is_read_only_sql` → principal check → `dry_run_sql` (`EXPLAIN AST` on `ClickHouse`) → `routes::query::run` (which calls `rewrite_sql_for_principal`, where table functions and the sensitive system tables are refused).
- Measured on a running API on 2026-10-10: `run_sql` with `SELECT * FROM url('http://169.254.169.254/latest/meta-data/', 'RawBLOB')` returned the policy refusal, and `system.query_log` showed `EXPLAIN AST SELECT * FROM url(…)` had been received. `EXPLAIN AST` parses and does not fetch, so nothing left the server; the acceptance step still requires that the statement not reach the engine.
- `routes/ai/gate.rs`: the tool gate refuses a tool whose `ToolSpec::permission` the caller lacks; `routes/ai/registry.rs` declares `query:read` for `run_sql` (`:776`) and for `run_saved_query`.
- A test already pins masking: `run_sql_is_masked_the_same_way_query_studio_is` in `data.rs`.
- `tools/queries.rs` `run_saved_query` and any other tool that sends model- or user-supplied SQL to an engine: check whether each has the same ordering problem.

## 3. Tasks

### T1 — Refuse before the dry run

In `run_sql` (and any sibling tool with the same shape), run the policy
decision for the caller before `dry_run_sql`. A refusal returns the same
message the user gets today. Keep `dry_run_sql` where it adds its
engine-verified "is this really a SELECT" check for statements the policy
admits.

*Accept:* a test with a mock `ClickHouse` that records requests: for a
table-function statement and for a refused system table, the tool returns
the refusal and the mock received **zero** requests; for an admitted
statement the dry run and the execution still happen, in that order; the
existing masking test is unchanged and green.

### T2 — Pin what is already true

Tests, where they do not already exist (name the existing ones in the
handoff instead of duplicating them): `run_sql` and `run_saved_query`
through the gate are refused for a principal without `query:read`, with
the permission named and nothing sent to an engine (`SEC-9-AC2`); a failed
statement returns a message with a reference and no engine version or host
(`SEC-9-AC4`); the natural-language entry point in Query Studio has no
route of its own that bypasses the gate (assert on `POLICY_TABLE` /
the router that no `/api/agent*` or text-to-SQL route exists).

### T3 — Docs

`CHANGELOG.md` `[Unreleased]` under Security: one line. Nothing else.

## 4. PR slicing

Commits T1…T3 on the phase 0 branch.

## 5. Out of scope (do not build)

Prompt changes; widening `SENSITIVE_SYSTEM_TABLES`; changing what Query
Studio itself allows; the assistant's other tools.

## 6. Things the developer must verify, not assume

- That calling the policy decision early does not need a database round trip the dry run was protecting against, or if it does (obligations read Postgres), that it is the same one execution already makes.
- That no other tool sends caller- or model-supplied SQL to an engine ahead of the policy decision (`run_saved_query`, dashboard SQL-source tools, alert tools).

## 7. Handoff (developer appends one entry per PR)

### SEC-9 — T1–T3 (developer, 2026-10-10)

Nothing committed (per instruction). Files per task, all under `rust/crates/lakehouse-api/` unless noted:

- **T1:** `src/routes/ai/tools/data.rs` (early refusal in `run_sql`, plus four tests in `run_sql_delegation`), `src/sql_rewrite.rs` (removed a now-false `#[allow(dead_code, reason = "no non-test caller …")]` from `classify_statement`; no logic change).
- **T2:** `src/routes/ai/gate.rs` (one unit test), `tests/ai_tool.rs` (one HTTP test), `tests/route_auth.rs` (one test), plus the AC4 test in `data.rs`.
- **T3:** `CHANGELOG.md` (`[Unreleased]`, Security, one line).

**Deviation (read this first).** The plan says to reuse `rewrite_sql_for_principal`. I did not, because it cannot meet the plan's own acceptance ("zero requests" for a refused system table). `rewrite_sql_for_roles` calls `prefetch_for_tables` BEFORE `sql_rewrite::enforce`, and for `system.query_log` the table list is non-empty, so `engine_and_definitions` sends a `SELECT … FROM system.tables WHERE (database, name) IN (…)` to `ClickHouse` (identifiers only, not the statement text, but a request all the same). It also reads Postgres (`has_any_obligation_async`, `obligations_for_async`). For a table function the list is empty and no request goes out, so only the system-table case differs. Code-read, not measured: I did not write a throwaway test with the full entry point.

What I used instead: `sql_rewrite::classify_statement(sql, &ClickHouseDialect{})`, which is `classify_statement_for_principal(sql, dialect, &[], false)`, the first step `enforce` itself runs (with the same empty permissions, so the same refusals and messages). It is synchronous: **zero round trips**, no Postgres. The refusal is mapped through `policy_engine::refusal_message` and returned as the same `ApiError::Unprocessable` body `routes::query::run` produces (via `api_result_to_value`). No rule is copied. Not refused early, by design: `Unparseable` (left to the dry run, whose classified engine error is what an author sees today), `dictGet`/`joinGet` (needs `has_any_obligation`, a Postgres read), and a view over a governed table (needs `system.tables`); all three still refuse in `routes::query::run`, after the dry run. `EXPLAIN AST` only parses, so none of the three fetches anything. If you want those three early too, that is a round trip and a Postgres read per call; say so and I will switch.

**Other tools checked for the same ordering problem (plan section 6):**

- `run_saved_query` (`tools/queries.rs`): no engine call before `routes::query::run`; reads the stored SQL from Postgres, then calls the handler. No issue.
- Dashboard tools (`create_chart`, `update_chart`, `suggest_dashboard`, `list_sql_sources` in `tools/dashboards.rs`): they send fixed statements built from identifiers, or go through `store::spec_from_input`; none sends model-written SQL text to the engine ahead of a policy decision in this file. I did NOT audit what `spec_from_input` does with a SQL-source chart's `sql` inside the `lakehouse-bi` crate: **not verified** (it is in the plan's out-of-scope "assistant's other tools" area, but section 6 asked).
- Alert tools (`tools/alerts.rs`): `run_alert_rule` goes to `lakehouse_alerts::run_rules`, which runs stored rule SQL against `ClickHouse` outside the policy rewrite. That is existing alert behaviour (not a model-supplied statement at call time) and out of this plan's scope; flagged, not changed. **Not verified** beyond reading the call site.
- `grep EXPLAIN` over `src`: `run_sql`'s dry run is the only production use.

**Existing tests that already cover T2's points (not duplicated):**

- `gate::tests::read_tool_missing_permission_is_refused`, `absent_principal_is_refused_every_permissioned_tool`, `analyst_cannot_create_chart_the_console_would_refuse`, and `tests/ai_tool.rs::missing_permission_refuses_and_audits_with_permission_reason` cover the gate in general, with `create_chart` and `describe_mart`. None named `run_sql` or `run_saved_query`, hence the two new tests.
- `registry.rs` pins `run_saved_query` as `query:read` (`:1470`).
- AC4 on Query Studio itself: `tests/query_author_exception.rs` (`the_author_sees_the_engines_diagnosis_of_their_own_statement`, `an_unreachable_engine_still_answers_the_fixed_message`) and the `upstream_error` unit tests (version dropped, reference kept). The dry run's parse error: `dry_run::run_sql_dry_run_hides_a_clickhouse_parse_error`. The gap was the tool path of an execution failure, hence the new AC4 test.

**New tests:** `data.rs`: `run_sql_refuses_a_table_function_before_anything_reaches_the_engine`, `run_sql_refuses_a_sensitive_system_table_before_anything_reaches_the_engine` (both assert the mock recorded zero requests), `run_sql_dry_runs_then_executes_a_statement_the_policy_admits` (exactly two requests, `EXPLAIN AST` first), `run_sql_failure_has_a_reference_and_no_engine_version_or_host`; `gate.rs`: `the_sql_tools_are_refused_without_query_read_and_name_the_permission`; `ai_tool.rs`: `the_sql_tools_are_refused_without_query_read_and_never_reach_an_engine` (outcome `refused` with the test app's dead `ClickHouse`); `route_auth.rs`: `no_route_runs_model_written_sql_outside_the_assistant_tool_gate`. The existing masking test is unchanged and green. I did not run the new refusal tests against the pre-fix code; they would fail there because the dry run sent `EXPLAIN AST` first (by the code, not by a run).

**Commands run (foreground, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target`, `CARGO_BUILD_JOBS=2`), final code:**

- `cd rust && cargo fmt --check`: clean (after `rustfmt` on `gate.rs`, `data.rs`, `route_auth.rs`, whose diffs were all mine).
- `cargo clippy -p lakehouse-api --all-targets --all-features --locked -- -D warnings`: finished, no warnings (after `touch src/lib.rs`).
- `cargo test -p lakehouse-api --lib`: 1521 passed, 0 failed, 1 ignored.
- `cargo test -p lakehouse-api --test sec11_guard --test route_auth --test security_regressions --test ai_tool`: 4, 31, 10, 9 passed; 0 failed.
- Not run: `bun`, Python, compose (nothing there touched). Test executables over 20M deleted from the target `deps` afterwards; disk 22G free.

## 8. Review (planner appends findings per PR)

### SEC-9 — T1–T3 (reviewer, 2026-10-10)

- **Accepted deviation.** The plan said to reuse the entry point Query
  Studio uses for the early decision. The developer used
  `sql_rewrite::classify_statement` instead, the first step that entry
  point itself runs: it makes no round trip, where the full path asks
  `ClickHouse` for `system.tables` and reads Postgres before refusing,
  which would itself break "zero requests" for a refused system table.
  Same refusals, same messages, no copied rule.
- **Known remainder, accepted.** Three refusals still happen after the
  dry run, because deciding them needs a database read: an unparseable
  statement, `dictGet`/`joinGet`, and a view over a governed table. For
  these `ClickHouse` receives `EXPLAIN AST`, which parses and executes
  nothing. Refusing them earlier costs a round trip and a Postgres read on
  every call.
- **No open BLOCKER.**

Verified by the reviewer: `cargo fmt --check` clean; `cargo clippy -p
lakehouse-api --all-targets --all-features --locked -- -D warnings` clean;
`cargo test -p lakehouse-api --lib` 1521 passed, 1 ignored. The developer
reports `sec11_guard` 4, `route_auth` 31, `security_regressions` 10,
`ai_tool` 9; not re-run by the reviewer.

On a rebuilt API against the dev `ClickHouse`, through `POST /api/ai/tool`
as Platform Admin: `run_sql` with a `url(...)` statement and with a read of
`system.query_log` were each refused with the policy's message, and
`system.query_log` held **zero** rows containing the probe address or the
planted marker afterwards (before this change it held an `EXPLAIN AST` row
for the same kind of statement); a valid statement ran and its
`EXPLAIN AST` and execution were logged; a misspelt column returned the
diagnosis with a reference.

*Not verified:* `SEC-9-AC1` and `SEC-9-AC2` on a running system (they need
a non-admin user with a masking policy and one without `query:read`; both
are covered by tests); `cargo test --workspace`.

Carried forward, outside this item: `system.users` is readable through
Query Studio and so through `run_sql`; `run_alert_rule` runs stored rule
SQL outside the policy rewrite (read at the call site only). Both need a
backlog item.

