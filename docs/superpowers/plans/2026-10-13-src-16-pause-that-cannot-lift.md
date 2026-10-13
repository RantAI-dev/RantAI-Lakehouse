# `SRC-16` A schema-change pause that nothing can lift — Implementation Plan

**Status:** ready to build. `SRC-16` is a Task (a defect in `SRC-8`, which is
In Acceptance), filed by the delivery review of 2026-10-12
(`docs/superpowers/plans/2026-10-12-delivery-review.md`). The product owner
asked for it to be done now (2026-10-13). It changes no decision of the
`SRC-8` feature page (`docs/core/features/source-schema-changes.md`); it
makes decisions 1 and 4 hold where they did not.
Written 2026-10-13 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Base and branch:** `main` at `ce9d8ed`; branch `fix/src-16-pause-stuck`.

**Why.** Decision 1 of `SRC-8` says of the "pause" policy: "the whole
connector stops loading until someone approves." In two cases there is
nothing to approve, so the connector stays stopped for good.

---

## 1. What exists today (anchors verified at `ce9d8ed`)

| # | Case | Code | Verdict |
| --- | --- | --- | --- |
| F1 | A connector whose changes are seen only after the load (files, REST, MongoDB, Kafka, SFTP), on policy `pause`, sees its first change. | `lakehouse-store/src/schema_diff.rs` `decide`: with `!can_hold_back` it returns `pause_connector = (policy == Pause)` and every change as `ChangeStatus::Applied`. The pause is written (`schema_change.rs:501`). Nothing is `pending`, so `POST …/schema-changes/approve` answers 404, and `lift_pause_if_nothing_waits` (`schema_change.rs:1110`) is never reached. | Confirmed by reading. The connector cannot be resumed from the product. |
| F2 | A connector is paused by an approvable change on one table while another table holds a change Approve refuses (a type the column cannot hold; a removed key column of a deduplicating table). | `lift_pause_if_nothing_waits` counts every `pending` row except `table_added`. The refused change stays `pending`, so the pause stays after everything approvable is approved. | Confirmed by reading. `SRC-8` review BLOCKER 3a stopped such a change from *setting* the pause; it did not stop it from *holding* one. |
| F3 | Changing the policy away from `pause`. | `PATCH /api/connectors/{id}` writes the policy and leaves the pause (a comment says only approval lifts it). | By design then; with F1 it removes the last way out. |
| F4 | The console. | The connector page shows "Paused … until a change is approved" beside "Nothing is waiting for a decision". | Follows from F1. |

## 2. Decisions (planner; none changes a signed decision)

- **D-a. Under `pause`, a change seen after the load waits to be
  acknowledged.** For a source that cannot hold a change back, `decide`
  keeps `action = Load` (the rows are already in the table) and, when the
  policy is `pause`, records the changes as `pending` and pauses the
  connector. Approving that table marks them approved, accepts the loaded
  shape as the baseline, and lifts the pause when nothing approvable is left.
  Under the other three policies nothing changes: such changes stay
  `applied`.
- **D-b. Only a change that can be approved holds a pause.** The lift check
  ignores a pending change that Approve would refuse (the same rule that
  computes `canApprove`), as it already ignores `table_added` notices. Those
  tables keep waiting on their own.
- **D-c. Leaving the `pause` policy lifts a schema-change pause.** When the
  policy is changed from `pause` to another, a pause whose reason is the
  schema-change one is lifted in the same transaction. Tables with pending
  changes keep waiting individually, as they do under the other policies. A
  pause set for another reason is not touched.
- **D-d. Connectors stuck today are released.** A migration lifts every
  schema-change pause on a connector that has no pending change at all: the
  state F1 leaves, which no user action can clear.
- **D-e. The console never shows a pause with nothing to do.** When a
  connector is paused for a schema change, the Schema changes tab lists what
  to approve; if the API ever reports such a pause with nothing approvable
  pending, the tab says so and points to the policy choice as the way out
  (D-c), instead of "Nothing is waiting for a decision".

## 3. Tasks

One task per commit; Rust tasks 1 to 4 sit together and the migration lands
with task 4. Cite `SRC-16` and the finding at each site.

1. **The decision.** `schema_diff.rs` `decide`: D-a. Pure tests: for each of
   the four policies, a change seen after the load; under `pause` it is
   `pending`, the action is `Load`, the connector pauses; under the others
   it is `applied` and nothing pauses. The existing tests that pin
   "`applied` and paused" for this case are corrected to the rule, and the
   commit says which and why.
2. **Approve and the baseline for an after-load table.** `schema_change.rs`
   and `routes/schema_changes.rs`: a pending change recorded in the
   after-load phase can be approved; the baseline becomes the shape that was
   loaded (the observation's carry-over rule for columns missing from a
   batch stays as it is); the audit event is written as for any approval.
   Route tests: files connector on `pause` → change observed after the load →
   connector paused, change pending, `canApprove` true → approve → 200,
   `pauseLifted` true, connector runs again (`ingest/run` no longer 409, the
   due list includes it).
3. **What holds a pause (D-b).** `lift_pause_if_nothing_waits` and the code
   that computes `canApprove` share one rule. Tests: an approvable change on
   table A and an unapprovable one on table B, policy `pause` → approving A
   lifts the pause; B still waits; removing B's table still clears it
   (`SRC-8` review BLOCKER 3b).
4. **Leaving `pause`, and the release (D-c, D-d).** The policy update lifts
   the schema-change pause in the transaction that writes the policy; a
   pause with another reason stays. Migration, next free number, why-header:
   `UPDATE connector SET paused_reason = NULL, paused_at = NULL WHERE
   paused_reason = '<the schema-change text>' AND NOT EXISTS (a pending row
   for it)`. Tests for both, and a store test of the migration's effect on a
   seeded stuck row.
5. **Console (D-e).** `connector-schema-changes-panel.tsx`: the paused
   state's text follows what is pending; the note of D-e for the
   should-not-happen case. Component tests for each state.
6. **Gate.** `ops/g6/g6_ingest_matrix_test.py`, group `other`: a files
   connector on `pause` whose CSV gains a column: after the run the
   connector is paused and one change waits; approve; the connector is not
   paused; a second run loads. Prints `[g6] SRC-16 …` lines.
7. **Docs (planner).** `CHANGELOG.md`, the `SRC-8` feature page's limits,
   `BACKLOG.md`.

## 4. Build limits on this machine

Shared cargo target `/home/hv/.cache/lakehouse-src6-target` (another agent
uses it: cargo waits on the lock), `CARGO_BUILD_JOBS=2`, `df -h /` before
each cargo command, no `cargo test`, no containers. Rust tests and the gate
step are *not verified* until CI on the pull request.

## 5. Out of scope

- `SRC-17` (alert delivery) and `SRC-18` (a queued or Dagster-launched run
  of a paused connector still loads; the observation routes' permission).
- Any change to what a breaking change does on a SQL source.

## Handoff

*(developer: Sonnet 5.5, 2026-10-13)*

**Commits** (branch `fix/src-16-pause-stuck`, on `main` at `ce9d8ed`, plan commit `4b135b6`):

1. `8a05a92` task 1, `decide` (D-a, F1).
2. `ed48be4` task 2, approve of an after-load change (route test and docs; see below).
3. `8ce0925` task 3, the lift rule shared with `canApprove` (D-b, F2).
4. `e67631f` task 4, policy change lifts the pause, migration `0067_release_schema_change_pause_with_nothing_to_approve.sql` (D-c, D-d, F3).
5. `d0397f5` task 5, the Schema changes tab (D-e, F4).
6. `13cb258` task 6, gate step `step_files_pause_after_the_load` (`[g6] SRC-16 step 1..4`).

**Interpretations (nothing in the plan said otherwise).**

- After the load, a `TypeChange::Other` change (which Approve refuses) is recorded `applied` and pauses nothing, under every policy. The plan's D-a says "pending and pause"; for this one change that would recreate the stuck state (approve answers 409) and the old rule (`SRC-8 review BLOCKER 3a`) already exempts it before the load. A mixed batch (Other plus an added column) is entirely `applied`, because `accept_observed` needs all rows applied.
- Task 2 needed no store change: the observation already stores the waiting shape (batch plus carried-over columns) and `approve_object` makes it the baseline. Only doc comments changed there; the commit says so.
- D-b is implemented by reading the connector's pending rows and applying `mark_approvable` (the one `canApprove` rule), instead of a SQL-only rule, because "cannot be loaded" is decided in Rust (`classify_type`). The lift now reads all pending rows of the connector (no 500-row cap).
- D-c: `update_connector` runs in a transaction and lifts before writing the policy, only when the stored policy is `pause` and the new one is another.
- D-d: the migration lifts when the connector has no pending row of any kind. A connector stuck by F2 (only unapprovable rows pending) is not released by it; leaving `pause` (D-c) or removing the table releases it.
- D-e: the panel gets an optional `pausedReason` prop; it recognises the schema-change pause by the API's exact text (`SCHEMA_CHANGE_PAUSE_REASON`, mirrored in the panel with a comment). A pause with another reason leaves the tab as it was.

**Existing tests changed** (all pinned the old rule):

- `schema_diff.rs`: `a_source_that_cannot_hold_back_always_loads_and_records_applied` (now `..._and_only_pause_holds_the_change`).
- `routes/schema_changes.rs`: `after_the_load_changes_are_applied_and_a_missing_column_is_not_removed` (now `after_the_load_under_pause_a_change_waits_...`); `changing_the_policy_does_not_lift_a_pause` (now `moving_the_policy_away_from_pause_lifts_the_schema_change_pause`).
- `lakehouse-store/tests/schema_change.rs`: helper `connector` now calls new `connector_named` (names are unique); no assertion changed.

**Tests added**: `schema_diff`: `after_the_load_a_type_change_that_cannot_be_approved_is_applied_and_pauses_nothing`. `routes/schema_changes`: `after_the_load_under_the_other_policies_a_change_is_applied`, `after_the_load_a_pause_is_lifted_by_approving_the_change`. Store: `an_unapprovable_change_on_another_table_does_not_keep_the_pause`, `an_approvable_change_on_another_table_still_keeps_the_pause`, `leaving_the_pause_policy_lifts_the_schema_change_pause_only`, `the_release_migration_lifts_only_a_schema_change_pause_with_nothing_pending`. Console: four cases in `connector-schema-changes-panel.test.tsx`.

**Related but unaffected** (read, assertions not touched by the change): `an_added_column_is_handled_by_each_policy`, `a_removed_column_waits_under_every_policy_and_the_baseline_stays` (both `before_load`), `after_the_load_an_unloadable_type_change_is_applied_and_never_waits` (`apply_non_breaking`, so still applied), `the_pause_policy_pauses_the_connector_until_approval` (SQL, `before_load`), the BLOCKER 2/3b store tests (`a_type_change_the_column_cannot_hold_...`, `removing_*`), `routes/connectors.rs` PATCH policy tests (no pause involved).

**Commands run and outcomes**

- `cd rust && cargo fmt --check`: clean.
- `df -h /` before clippy: 15 GB free. Touched every changed Rust file, then `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: finished in 2m23s, 0 warnings, 0 errors (it compiles the tests too). Run once, after tasks 1 to 4; no Rust changed afterwards.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (all in files this work did not touch). `bun run test`: first full run 1103 pass, 1 skip, 1 fail (`ConnectorDetailPage > marks a table that waits for a schema decision in the Ingest tab...`, a 30 ms failure); that file alone passed 28/28, and a second full run gave 1104 pass, 0 fail, 1105 tests across 113 files. The failure looks like a timing flake; not reproduced, not investigated further.
- `~/.cache/rantai-dagster-venv/bin/python -m py_compile ops/g6/g6_ingest_matrix_test.py ops/g6/seed_files_fixture.py`: ok. `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: OK. `python3 ops/lint/check_connector_gate_coverage.py`: OK, 11 types covered, 0 waived.

**Not verified**

- Every Rust test above (no `cargo test` allowed here). Each was traced by hand against the code (SQL, helpers, JSON names, status codes) but a hand trace is not a run; CI is the proof. Points most worth a reviewer's eye: the route test `after_the_load_a_pause_is_lifted_by_approving_the_change` sets `adapter = 'files'` directly and asserts `ingest/run` is "not 409" after the approval (the Dagster mock is bare, so the launch fails with another status); `leaving_the_pause_policy_...` and the migration test use the store helpers with `pause_connector: true`.
- The migration was not applied to a database; the store test runs its statement on seeded rows.
- The gate step (`ops/g6`) was compiled only; it needs the compose stack. No `docker compose` was run. Expected to be proven by the `other` job on the PR.
- No browser check of the console.
- Task 7 (docs) is the planner's.

## Review

### Review 1, 2026-10-13 (planner), commits `8a05a92`..`616191b`

Read: the whole diff against `ce9d8ed`. Re-run by the reviewer on `616191b`:
`cargo fmt --check` clean; the workspace clippy command of section 4 clean
(2m23s, all targets, so every test compiles); `bun run typecheck` clean;
`bun run lint` 0 errors, 6 warnings in files this branch does not touch;
`bun run test` 1104 pass, 1 skip, 0 fail (1105 tests, 113 files); the three
`ops/lint` checks pass; `py_compile` of the two gate files. *Not verified*:
every Rust test and the gate step, which only the pull request's CI can run.

No `BLOCKER`. No `SHOULD-FIX`.

Accepted, and recorded because they go beyond the plan's words:

- **D-a, narrowed by the developer.** A type change Approve refuses, seen
  after the load, is recorded `applied` and pauses nothing under every
  policy, and so is a batch that mixes it with an approvable change. As
  written, D-a would have made it `pending` under `pause` and recreated the
  stuck state. This is the rule `SRC-8 review BLOCKER 3a` already applies
  before the load. Accepted.
- **D-b.** The lift check reads the connector's pending rows and applies
  `mark_approvable` in Rust, because "cannot be loaded" is decided there.
  One rule, as the plan asked.
- **D-d, a limit.** Migration `0067` releases a paused connector with no
  pending row. A connector paused with only unapprovable rows pending (F2
  reached before this fix) is not released by it; choosing another policy
  than "pause", or removing the table, releases it. The console now says so.
- **After D-c on an after-load source.** A change left pending when the
  policy leaves `pause` is recorded `applied` by the next run and the stale
  pending row is withdrawn by the observation's own delete. Read, not run.

Open for the product owner: a browser check of the Schema changes tab's two
new notes.
