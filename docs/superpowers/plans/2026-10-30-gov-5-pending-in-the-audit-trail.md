# GOV-5: a decision still waiting reads Pending in the audit trail, not Denied

Backlog: `GOV-5` (Task, Governance, P3). Decided by the product owner on 2026-10-30: a chat
draft waiting for Confirm, and a request waiting for approval, read **Pending**. Base: `main`
at `4ce58470`; every `file:line` below was read there.

Planner: Claude Opus. Developer: a different agent (`AGENTS.md`). The developer does not change
a decision below; if the code disagrees with the plan, stop and report it.

## 0. What is wrong

`outcome_of` (`rust/crates/lakehouse-api/src/routes/governance.rs:322`) narrows
`audit_event.outcome` (eight values) onto the console's three: `success`, `denied`, `error`. It
maps `needs_confirmation` and `needs_approval` to `denied` on purpose (doc at `:317-321`), so an
open decision never reads as a success. The audit page then shows a draft that nobody has
confirmed yet as "Denied", and `:396` reports its policy decision as `deny`. Both are wrong: the
policy did not refuse anything, and the decision is still open.

## 1. Decisions (made; do not change)

- **D1.** A fourth outcome, `pending`, labelled "Pending", for `needs_confirmation` and
  `needs_approval` only. `refused` and `rejected` stay `denied`. `failed` stays `error`. The
  three success values stay `success`.
- **D2.** The policy decision of a pending row is `pending`, not `deny` (`:396`). A success row
  stays `allow` and everything else stays `deny`.
- **D3.** The Pending badge uses the `info` tone. It is neither a warning nor a success.
- **D4.** No other value moves, no migration, and no change to what is written into
  `audit_event`. This is read-side only.
- **D5.** This is not a BI feature, so the assistant-parity rule does not apply. No assistant
  tool reads this outcome: `routes/ai/` has no audit tool (checked with grep at `4ce58470`).

## 2. Anchors (at `4ce58470`)

| What | Where |
| --- | --- |
| The mapping and its doc | `routes/governance.rs:317-329` (`outcome_of`) |
| Where it is used | `routes/governance.rs:395` (`outcome`), `:396` (`policyDecision`) |
| Its unit test | `routes/governance.rs:2222` (`outcome_of_treats_pending_gates_as_denied_not_success`) |
| Console type and labels | `src/lib/status.ts:286` (`AuditOutcome`), `:288` (`AUDIT_OUTCOME_LABEL`) |
| Badge tone | `src/components/patterns/status-badge.tsx:397` (`OUTCOME_TONE`), test `status-badge.test.tsx` |
| Filter options | `src/features/governance/audit-columns.tsx:41` (`OUTCOME_OPTIONS`, built from the labels, so it picks up the new one) |
| Contract | `src/services/contracts/governance.ts:254` (`outcome: AuditOutcome`), `:255` (`policyDecision: string`) |

## 3. Tasks (one commit each, in order)

**T1. API (`routes/governance.rs`).**
- `outcome_of` maps `needs_confirmation` and `needs_approval` to `"pending"` (D1), and its doc
  says why. Keep the old reason too: an open decision must not read as a success.
- `:396`: `pending` gives `"pending"` (D2).
- Rename the test to say what it checks now, for example
  `outcome_of_reports_a_gate_still_waiting_as_pending`, and assert all eight values. Add a test
  that a pending row's `policyDecision` is `pending` (in `audit`'s mapping, or extract the
  two-line mapping into a pure function and test that).
- Cite `GOV-5` at the site.
- *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api governance`.

**T2. Console (`src/lib/status.ts`, `status-badge.tsx`).**
- `AuditOutcome` gains `"pending"` with the label `Pending`. `OUTCOME_TONE.pending = "info"`
  (D3).
- Add a case to `status-badge.test.tsx` that `OutcomeBadge` renders "Pending".
- Check that nothing else narrows on the union. `grep -rn "AuditOutcome" src` should show only
  the places above. Report any other place.
- *Accept:* `bun run typecheck`, `bun run lint`, and the touched test files:
  `bun test src/components/patterns/status-badge.test.tsx src/features/governance`.

**T3. Docs.** If a feature page or `docs/core/PRODUCT.md` describes the audit outcomes, add
Pending. Search for "Denied" near "audit" under `docs/core/`. The developer does not touch
`BACKLOG.md`.

## 4. Verification before handoff

On the final commit, in the foreground:

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
git status
```

`bun run test` cannot run whole on a 13 GB machine (one process reached 7.2 GB before). Run it
one folder at a time and total the counts. Say so in the handoff.

## 5. Out of scope

- Any change to how gates write `audit_event`.
- A filter that hides pending rows.
- `home-ui.tsx`'s own `"denied"` (a card state, unrelated).

## 6. Handoff (developer appends below)

## 7. Review (planner appends below)
