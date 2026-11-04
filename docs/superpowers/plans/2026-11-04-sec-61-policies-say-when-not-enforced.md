# SEC-61: the policy list says when a policy enforces nothing

Backlog `SEC-61` (P1, Administration & Security). Planner: Claude Opus, 2026-11-04.
Anchors verified at `main` `d19bdf73`. Base branch: this one.

## 0. What is wrong, with evidence

- The Create Policy wizard (`src/features/governance/policy-create-page.tsx`, removed by
  `WEB-3` slice 1) saved free-text `subjects`, `resources` and `conditions`.
- The policy engine enforces only a `conditions` value that
  `policy_engine::PolicyCondition::parse` accepts (`rust/crates/lakehouse-api/src/policy_engine.rs:82-97`):
  structured JSON with roles, a table, and a mask or a row filter. Everything else is skipped
  (`:2-21`, `load_enforceable_conditions` `:279`).
- The kinds Agent autonomy, Retention and Residency are enforced nowhere.
- Yet the Policies page shows such a policy as **Enforced** (`policy-columns.tsx:71-75`,
  `StatusBadge` of `status = ready`), and its drawer offers Enforce / Stop enforcing
  (`policy-actions.tsx:21-47`) as if that changed anything. An admin who made a masking policy
  in the wizard believes a column is masked when it is not. Fail-closed in the engine,
  fail-open in what the person is told.

## 1. Decisions

- **D1. One test of "enforceable", the engine's own (rule 4).** `GET /api/governance/policies`
  (`routes/governance.rs:970`) answers each policy with a new boolean `enforceable`:
  `PolicyCondition::parse_opt(policy.conditions.as_deref()).is_some()`. No second parser,
  in Rust or TypeScript. `parse_opt` loses its `dead_code` allow (it has a caller now).
  Status is not part of it: `enforceable` says whether the rule CAN be enforced; `status`
  says whether it is switched on.
- **D2. The wire shape.** A response type in `routes/governance.rs` (e.g. `PolicyView` with
  `#[serde(flatten)] policy: Policy` and `enforceable: bool`), so `lakehouse-store`'s
  `Policy` stays the table's mirror. `src/services/contracts/governance.ts` `Policy` gains
  `enforceable: boolean` (required: every response carries it). Any other route that returns
  a `Policy` to the console (create returns one; check `create_policy`, the status PUT)
  carries it too, from the same function.
- **D3. The console says it.**
  - Status column (`policy-columns.tsx`): a `ready` policy with `enforceable: false` shows a
    "Not enforced" pill (warning tone) instead of "Enforced"; a draft one shows its draft
    status as today. The column's filter still filters on `status`.
  - Drawer: one line above the actions when `enforceable` is false: "This policy is written
    as text, so it masks nothing and filters no rows. Add the rule on the table's Access tab,
    where it is enforced, then delete this one." with a link to `/catalog`.
  - Enforce / Stop enforcing are disabled for a non-enforceable policy, with that sentence as
    the reason (Delete stays).
  - The Effect column (`policy-columns.tsx`) renders every value red today because its filter
    expects allow/deny and the stored value is "Permit with obligation". Show the stored
    effect as neutral text; drop the allow/deny filter. (Found in the same audit; one line.)
- **D4. The assistant's `draft_policy` tool** is not changed here. If its drafts are not
  structured `conditions`, say so in the handoff; the planner files it.

## 2. Anchors (at `d19bdf73`)

| What | Where |
| --- | --- |
| The test | `rust/crates/lakehouse-api/src/policy_engine.rs:82-111` (`parse`, `parse_opt`) |
| The routes | `routes/governance.rs:970` (`list_policies`), `create_policy`, the status PUT; `policy.rs:334-342` (unchanged) |
| The store type | `rust/crates/lakehouse-store/src/governance.rs:73-104` (unchanged) |
| The contract | `src/services/contracts/governance.ts:11-27` |
| The console | `src/features/governance/policy-columns.tsx:71-90`, `policy-actions.tsx:21-70`, the drawer in `policies-page.tsx` |
| Route tests | `rust/crates/lakehouse-api/tests/` (grep `governance/policies`) |

`WEB-3` slice 1 (branch `feat/web-3-slice-1-remove-thin-pages`) edits `policies-page.tsx`'s
header; this task does not touch the header, so a merge conflict, if any, is small.

## 3. Tasks (one commit each, in order; push after each)

**T1. The API (D1, D2).** With tests: a policy with structured conditions → `enforceable:
true`; free-text conditions → false; no conditions → false; an Agent autonomy policy → false.
- *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api <the new tests' filter>`.

**T2. The console (D3).** Tests for the pill, the drawer line and the disabled buttons.
- *Accept:* `bun run typecheck && bun run lint && bun run test src/features/governance`.

**T3. Changelog and docs.** `## [Unreleased]` → `### Security`: the Policies page no longer
calls a policy enforced when the engine cannot enforce it (`SEC-61`). The governance feature
page, if it describes the Status column.

## 4. Mutation check (in the handoff)

Make `enforceable` always `true` and run T1's free-text test: it must fail. Restore, rerun.

## 5. Verification before handoff

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
git status
```

## 6. Out of scope

- Migrating old free-text policies to structured ones (they cannot be, without a person).
- The assistant's tool (D4). Removing the wizard (`WEB-3`).

## 7. Handoff (developer appends below)

## 8. Review (planner appends below)
