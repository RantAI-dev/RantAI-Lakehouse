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

*(developer)*

## Review

*(planner)*
