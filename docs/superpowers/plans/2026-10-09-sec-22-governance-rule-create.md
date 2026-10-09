# `SEC-22` Adding a governance rule needs `governance:write` — Implementation Plan

**Status:** ready to build. Decision 1 on the feature page was signed by the
product owner on 2026-10-09; decisions 2 and 3 are the planner's and are
flagged to the owner.
Written 2026-10-09 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/sec-22.md`. Backlog `SEC-22`.
**Feature page:** `docs/core/features/governance-rule-create.md`.

**Why.** Adding a governance rule asks only for a login, while changing or
removing one asks for `governance:write`.

---

## 1. Slice and base

One pull request. Base `origin/main` at `c338862`, branch
`fix/sec-22-governance-rule-create`, worktree `/home/hv/lakehouse-sec22`.
Commits stay local; the planner pushes and opens the pull request.

## 2. What exists today

Anchors verified at `origin/main` `c338862`. `A` is
`rust/crates/lakehouse-api/src`. Re-find by symbol if a line has moved.

| # | Code | Verdict |
| --- | --- | --- |
| F1 | `A/policy.rs:278-279`: `GET` and `POST /api/governance/{kind}` are `Policy::RequiresAuth`. `PUT` and `DELETE /api/governance/quality/{id}` and `DELETE /api/governance/classification/{id}` are `governance:write` (`:270-277`). | Adding is open to any login. |
| F2 | `A/routes/governance.rs:831` `create_rule` takes `Option<Extension<Principal>>` and uses it only for the audit row; it dispatches to `create_quality_rule`, `create_classification_rule` and the residency writer, none of which checks anything. | No check in the handler either. |
| F3 | `A/routes/ai/tools/governance.rs:140` and `:159`: the assistant's tools call `create_classification_rule` and `create_quality_rule` directly. A route policy does not cover them. | The assistant is a second way in. |
| F4 | Console: "Add rule" (`src/features/catalog/asset-quality.tsx`), "Add Quality Rule" (`src/features/governance/data-quality-page.tsx:173`), "Classify" (`src/features/catalog/asset-access.tsx:237`), "Add Rule" (`src/features/governance/classification-page.tsx:133`) are shown to everyone. Edit and Delete beside them check `hasPermission("governance:write")`. | Buttons that will now be refused. |

## 3. Decisions already made by the planner

- **The route policy changes, not the handlers.** `POST
  /api/governance/{kind}` becomes
  `Policy::RequiresPermission("governance:write")`. `GET` stays
  `RequiresAuth`. The comment above the two entries is rewritten to say why
  they now differ.
- **The assistant's tools check the caller.** Each tool that adds a
  governance rule refuses a principal without `governance:write` before it
  calls the handler, with the fixed text "You do not have permission to add
  governance rules." Look first at how `routes/ai/registry.rs` and the tool
  dispatcher already know a tool's required permission or the caller's
  principal; use that mechanism if one exists, and do not write a second
  one (rule 4). If the tools have no access to the principal at all, stop
  and report.
- **No migration, no new permission, no new grant.**
- **The tool schema snapshot (`tests/fixtures/tool_schemas.json`) must not
  change.**

## 4. Tasks

One task per commit. Cite `SEC-22` and the finding at each fix site and in
the commit body (rule 13).

**T1. Route policy (F1, F2).** `A/policy.rs`. *Check:* the unit tests in
`policy.rs` gain one asserting the `POST` needs `governance:write` and the
`GET` only a login; `tests/route_auth.rs` already loops `POLICY_TABLE` both
ways, so add a named test beside the existing annotation ones: a principal
with every permission except `governance:write` gets 403 from `POST
/api/governance/quality`, `/classification` and `/residency`, and nothing is
stored.

**T2. Assistant tools (F3).** As decided above. *Check:* unit tests beside
the existing ones in `tools/governance.rs`: without the permission each tool
returns the fixed text and stores nothing; with it, the existing behaviour.
Regenerate nothing; assert the schema snapshot test still passes by type-check.

**T3. Console (F4).** The four buttons render only with
`hasPermission("governance:write")`, the way Edit and Delete beside them do.
A residency page with an add button, if one exists, gets the same. *Check:*
tests beside the existing ones for each of the four files: hidden without
the permission, shown with it.

**T4. Docs (planner, same branch).** `CHANGELOG.md` (Security),
`docs/core/BACKLOG.md`, `docs/OPERATIONS.md` if it lists permissions.

## 5. Build limits on this machine

Same as `docs/superpowers/plans/2026-10-08-src-6-broken-connectors.md`
section 5: shared target dir and two jobs
(`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
CARGO_BUILD_JOBS=2`), `df -h /` before each cargo command, `touch
rust/crates/*/src/lib.rs rust/crates/lakehouse-store/src/*.rs` before a
quoted clippy run, no `cargo test` here (Rust tests are *not verified*; CI
runs them first), `cargo fmt --check` and `cargo clippy -p lakehouse-api
--all-targets -- -D warnings`. TypeScript: `bun install --frozen-lockfile`,
then `bun run typecheck && bun run lint && bun run test` in full.

## 6. Out of scope

- Reading rules, running rules, the other auth-only `{kind}` fan-outs.
- New roles or grants.

## Handoff

*(developer, Sonnet 5.5, 2026-10-09)*

**Commits** (branch `fix/sec-22-governance-rule-create`, local only):

- `0ba42e5` fix(api): adding a governance rule needs governance:write (T1)
- `2ca4fc2` fix(api): the assistant needs governance:write to add a governance rule (T2)
- `09dd137` fix(console): show the add-rule buttons only with governance:write (T3)

**Commands run**

- Rust, from `rust/` with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target CARGO_BUILD_JOBS=2`, `df -h /` first (49-53G free):
  `cargo check -p lakehouse-api --tests` (clean, after T1);
  after `touch` of the crate sources, `cargo fmt --check` (exit 0) and
  `cargo clippy -p lakehouse-api --all-targets -- -D warnings` (clean, no warnings), run again on the final Rust state.
- TypeScript: `bun install --frozen-lockfile` (lockfile unchanged), then
  `bun run typecheck && bun run lint && bun run test`: typecheck and lint clean, `920 pass, 1 skip, 0 fail`, 2319 expect() calls, 921 tests in 102 files.
  The new page tests alone, with `asset-detail-tabs.test.tsx`: 108 pass, 0 fail.

**Not verified.** No `cargo test` was run (machine limit): the new unit tests in `policy.rs` and `routes/ai/gate.rs`, the changed `registry.rs` expectations, the schema snapshot test, and the new `tests/route_auth.rs` test (needs Postgres via `sqlx::test`) are *not verified (first run is CI's)*. Nothing was run in a browser.

**What changed against the plan**

- T1: the route test is `adding_a_governance_rule_without_governance_write_is_refused_and_stores_nothing` in `tests/route_auth.rs`; it uses a principal with `catalog:read,catalog:write,policy:read,policy:write,query:read,pipeline:write` (not literally every permission) and counts rows in `quality_rule`, `classification_rule`, `residency_rule`.
- T2: the mechanism that exists is `ToolSpec.permission` in `registry.rs`, enforced by `gate::decide` at every chat and `POST /api/ai/tool` dispatch (and used to filter the tools offered). Both tools now declare `governance:write` there. The refusal text is therefore the gate's existing `refused: this user lacks the 'governance:write' permission this tool needs`, not the plan's fixed text "You do not have permission to add governance rules.": adding that would have been a second check (rule 4). Tests sit in `gate.rs` and `registry.rs`, not `tools/governance.rs`, because the refusal happens in the gate, before the tool runs. The tool schema JSON is untouched.
- T3: no residency page with an add button exists. The button tests are in `asset-detail-tabs.test.tsx` (Quality and Access tabs) and two new files, `data-quality-page.test.tsx` and `classification-page.test.tsx`. Two existing tests clicked the buttons synchronously; they now `await findBy…` because `hasPermission` is false until the session loads.

**Open for the reviewer**

- The headless agent-run path (`routes/agents.rs`, `run_tool` at the scheduled and approved-run call sites) does not call `gate::decide` for the permission check, so an agent employee that has these tools can still add a rule. This is outside the plan's scope and untouched; it may need its own backlog item.
- The "hidden without the permission" page tests wait 50 ms after the session answered, so that "no button" is not just "not loaded yet".

## Review


*(planner)*
