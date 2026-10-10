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

## 8. Review (planner appends findings per PR)
