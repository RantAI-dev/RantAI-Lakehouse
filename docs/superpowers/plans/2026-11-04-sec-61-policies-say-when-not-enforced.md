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

**Developer handoff, 2026-11-04.** Base `2553a19c`; branch
`fix/sec-61-policies-say-when-not-enforced`, pushed after every commit.

### Commits

| Task | Commit | Contents |
| --- | --- | --- |
| T1 | `c19f8f92` | `PolicyView` + `enforceable` on the four policy routes; `parse_opt` allow removed; TS contract field; unit tests |
| T2 | `6c7a3da9` | Status pill, drawer notice, disabled actions, Effect column; 7 render tests |
| T3 | `a2ec0a4e` | `CHANGELOG.md` `[Unreleased]` → `### Security` |

### T1 — the API (D1, D2)

- `routes/governance.rs`: `PolicyView { #[serde(flatten)] policy: Policy,
  enforceable: bool }`. `PolicyView::new` is the only computation:
  `PolicyCondition::parse_opt(policy.conditions.as_deref()).is_some()`.
  `list_policies`, `create_policy`, `create_policy_route` and
  `set_policy_status` return it; `delete_policy` still answers `{ok, id}`.
  `lakehouse-store`'s `Policy` is untouched (still the table's mirror).
- `policy_engine.rs`: `parse_opt` loses the `dead_code` allow. Its stated
  reason was already stale — `catalog_governance.rs:927` and two call sites
  in this route call it.
- `src/services/contracts/governance.ts`: `Policy.enforceable: boolean`,
  required. The two fixtures in `governance.test.ts` carry it (legacy row →
  `false`, structured row → `true`) and assert it.
- Tests (unit, in `routes::governance::tests`): structured conditions →
  true; free text → false; none → false; an Agent autonomy policy with its
  usual prose → false; and one test that the serialized body keeps every row
  field (`name`, `status`, `updatedAt`) and adds a top-level `enforceable`
  boolean.
- *Not verified:* no new DB-backed HTTP test for `GET`/`POST`/`PUT
  /api/governance/policies`. The tests assert the mapping and the exact
  serialized body the routes return; the routes are one-line wrappers over
  it. Existing route tests passed in the workspace run.

### T2 — the console (D3)

- Status cell: `PolicyStatusPill` in `policy-columns.tsx` — `ready` +
  `enforceable: false` → warning-tone "Not enforced" pill whose `title` is
  the reason; everything else keeps `StatusBadge`. Filtering and sorting
  stay on `status` (the accessor is unchanged).
- Drawer (`policies-page.tsx`): `PolicyNotEnforceableNotice` sits above the
  actions when `enforceable` is false; it is the plan's sentence verbatim,
  plus a link "Open the catalog" → `/catalog`.
- `PolicyActions` takes `enforceable`; Enforce / Stop enforcing are
  `disabled` with the sentence as `title`; Delete stays enabled.
- Effect column: neutral text instead of the allow/deny pill, filter
  dropped (`enableColumnFilter: false`); sorting kept.
- `asset-access.tsx` (the table's Access tab) also renders `PolicyActions`;
  it passes `enforceable: true`, with a comment: every `policySummary` entry
  comes from `catalog_governance::summarize_policies`, which drops rows
  `PolicyCondition::parse_opt` rejects, so those rows are enforceable by
  construction.
- Tests `src/features/governance/policy-enforcement.test.tsx`: 7 cases —
  ready+false → "Not enforced" (and not "Ready"), draft+false → "Draft",
  ready+true → "Ready", the notice's sentence and `/catalog` link, Enforce
  disabled with the reason + Delete enabled, Stop enforcing disabled,
  Enforce enabled for an enforceable draft.
- **Plan/code mismatch, reported rather than silently resolved:** the plan
  says the page shows such a policy as **"Enforced"**; the code's
  `StatusBadge` renders `ENTITY_STATUS_LABEL.ready`, which is **"Ready"**
  (`src/lib/status.ts:27`). The decision was implemented as written in
  intent — ready + `enforceable: false` is "Not enforced", ready +
  `enforceable: true` still reads "Ready". No decision changed.
- **Small extension within D3's intent, flagged for review:** the drawer's
  own status badge also uses `PolicyStatusPill`, so the drawer cannot say
  "Ready" directly above a line saying the policy enforces nothing. The
  plan only named the notice line.
- Limit: a `title` on a disabled button is not shown by every browser, so
  the drawer notice is the reliable place for the reason.

### T3 — changelog and docs

- `CHANGELOG.md`, `[Unreleased]` → `### Security`, first bullet: the
  Policies page no longer calls a policy enforced when the engine cannot
  enforce it, including the limit that no saved policy is migrated.
- No page under `docs/core/features/` describes the Policies page or its
  Status column (checked with `grep -rn "Policies" docs/core/features/`,
  which matches nothing), so none was edited. The planner may want one.

### Mutation check

`PolicyView::new` temporarily `let enforceable = true;` →
`cargo test -p lakehouse-api --lib free_text_conditions_are_not_enforceable`
**failed** (0 passed, 1 failed, 3007 filtered; assertion "the engine skips
prose, so the list must not call it enforced"). Restored, reran
`cargo test -p lakehouse-api --lib enforceable` → 5 passed, 0 failed.
(The mutation also breaks the other three `false` cases, as expected; the
free-text test is the one the plan names.)

### Commands actually run, with counts

Machine note: 13 GiB RAM. Default parallelism OOM-killed `rustc` (signal
9) twice, so every cargo command below ran with `CARGO_BUILD_JOBS=1` and
the shared `CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target`; no
`cargo clean`. Stale `clickhouse-server` testcontainers from earlier runs
were removed before the workspace run (they held about 2.5 GiB).

Per commit:

- T1: `cargo fmt --check` → ok. `cargo clippy -p lakehouse-api
  --all-targets -- -D warnings` → finished, 0 warnings.
  `cargo test -p lakehouse-api --lib enforceable` → 5 passed, 0 failed,
  3003 filtered. `cargo test -p lakehouse-api --lib routes::governance::`
  → 44 passed, 0 failed.
- T1 (contract): `bun run typecheck` → pass. `bun run test
  src/services/contracts/governance.test.ts` → 3205 passed, 1 skipped,
  0 failed (327 files; the script is `bun test src`, so the whole console
  suite ran). `bun --bun eslint` on the two files → clean.
- T2: `bun run typecheck` → pass. `bun run test src/features/governance`
  → 3212 passed, 1 skipped, 0 failed (328 files). The new file alone:
  `bun test src/features/governance/policy-enforcement.test.tsx` → 7
  passed, 0 failed, 14 expects. `bun --bun eslint` on the five touched
  files → clean.
- T3: markdown only, no check.

Section 5 block (final commit `a2ec0a4e`):

- `cargo fmt --check` → ok.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  → finished in 6m27s, 0 warnings.
- `cargo test --workspace` → 105 targets `test result: ok`; **one flake**:
  `lakehouse-api --test upload_routes`,
  `a_fifth_simultaneous_piece_of_one_user_is_429_with_retry_after`
  (5 accepted where it expects 4), which stopped the run before
  `upstream_errors` and the doc tests. Rerun of that binary:
  `cargo test -p lakehouse-api --test upload_routes` → **175 passed, 0
  failed**; that test alone → 1 passed; `cargo test -p lakehouse-api
  --test upstream_errors` → 7 passed; `cargo test --workspace --doc` →
  all ok. Judgement: a timing flake under `RUST_TEST_THREADS=4`, unrelated
  to SEC-61 (nothing here touches uploads or limits); green on rerun, but
  the planner should know it flaked once.
- `bun run typecheck` → pass.
- `bun run test` → 3212 passed, 1 skipped, 0 failed, 3213 tests / 328
  files, 8752 expects.
- `bun run lint` → **exit 1**, and every error is in `dagster/.venv/**`, a
  local untracked Python virtualenv on this machine (eslint does not read
  `.gitignore`). `bun --bun eslint --ignore-pattern 'dagster/.venv/**'` →
  exit 0, **0 errors**, 10 pre-existing warnings. *Not verified:*
  `bun run lint` exactly as written, because of that directory; none of
  the errors names a repository file.
- `git status` → clean, nothing generated staged.

*Not verified, with the reason:* the two `ops/lint` scripts,
`python -m pytest dispar_orchestrate`, and `docker compose --profile '*'
config --quiet` (plus a real `up`) were not run: this change touches no
Python or compose file, and yields the per-commit scoping rule in
`AGENTS.md`. The browser look of the pill and the drawer is for the
owner's acceptance checklist; the tests assert the DOM, not the rendering.

### D4 — the assistant's `draft_policy`

`draft_policy` (`routes/ai/registry.rs:1552-1565`) declares `conditions`
as a bare `"type": "string"`, with no description and no shape, and the
tool passes the model's arguments straight to `create_policy`
(`routes/ai/tools/governance.rs:110-126`). `create_policy_body` only
refuses JSON-shaped conditions that describe no obligation, so a draft in
practice lands with prose or no `conditions` at all — and is therefore
`enforceable: false` under this change. The draft is honest (it stays a
draft, and the page now says "Not enforced"), but **the assistant never
produces an enforceable policy**. Not changed here (D4). Suggestion for
the planner to file: either give the schema the structured shape (`roles`,
`table`, `mask` / `rowFilter`) or state in the description that the draft
enforces nothing and must be re-authored from the table's Access tab.

## 8. Review (planner appends below)
