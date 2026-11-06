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

### Handoff (developer, 2026-10-30)

Branch `fix/gov-5-pending-audit` (base `main` at `4ce58470`, the plan commit
`81ea5ca0` on top). Read-side only, as decided.

**Commits**

- `e271f8bf` — fix(governance): a gate still waiting reads Pending in the audit trail (GOV-5 T1).
  `outcome_of` maps `needs_confirmation`/`needs_approval` to `"pending"`; the two-line
  policy-decision mapping became the pure function `policy_decision_of`, which returns
  `"pending"` for a pending row, `"allow"` for a success row and `"deny"` for everything
  else. The old test is renamed to `outcome_of_reports_a_gate_still_waiting_as_pending`
  and asserts all eight `audit_event.outcome` values; `policy_decision_of_reports_pending_for_a_gate_still_waiting`
  is new. Nothing was written into `audit_event`; `refused`/`rejected` stay `denied`,
  `failed` stays `error`.
- `91ed2ebd` — feat(console): the audit trail shows a Pending badge for a decision still waiting (GOV-5 T2).
  `AuditOutcome` in `src/lib/status.ts` gains `"pending"` with the label `Pending`;
  `OUTCOME_TONE.pending = "info"`; `status-badge.test.tsx` gains a case asserting the
  label and the `info` tone (and that `Denied` keeps the warning tone).
- A last commit adds this handoff to the plan file and nothing else; its hash is the
  branch head.

**T3 (docs): nothing to change, so no commit.** No feature page and no `docs/core/PRODUCT.md`
describes the audit outcome vocabulary. Checked with `grep -rn "Denied" docs/ --include='*.md'`
(the only hits are the `GOV-5` row in `BACKLOG.md`, which the developer does not touch, and
an unrelated line in `docs/archive/`), `grep -rni "denied\|audit outcome" docs/core/`
(`calculated-fields.md` talks about a denied *column*, unrelated),
`grep -rn "Outcome" docs/core/` (only a table header in `BACKLOG.md`) and
`docs/core/reference/user-guide.md` (describes the Audit Log page, not its outcomes).
`docs/core/features/` has no page for the Audit page.

**Commands run, with counts**

Per task, scoped to what the commit touched:

- T1: `cargo fmt --check` — clean. `cargo clippy -j 2 -p lakehouse-api --all-targets -- -D warnings`
  — clean (Finished in 6m 17s). `cargo test -j 2 -p lakehouse-api governance` — the lib
  target 80 passed / 0 failed and the bin target 83 passed / 0 failed; both new tests ran
  and passed (`outcome_of_reports_a_gate_still_waiting_as_pending`,
  `policy_decision_of_reports_pending_for_a_gate_still_waiting`).
- T2: `bun test src/components/patterns/status-badge.test.tsx src/features/governance` —
  39 pass, 0 fail, 83 `expect()` calls, 9 files. `bun run typecheck` — clean.
  `bun run lint` — 0 errors, 11 warnings, all pre-existing and none in a touched file.

Final block on the last code commit (`91ed2ebd`), in the foreground. The
branch head adds only this plan file, so the Rust and TypeScript results
stand for the head as well:

- `cargo fmt --check` — clean.
- `cargo clippy -j 2 --workspace --all-targets --all-features -- -D warnings` — clean
  (Finished in 5m 48s).
- `cargo test -j 2 --workspace` — **159 binaries, 8551 passed, 0 failed, 51 ignored**
  (log: `/tmp/gov5-test.log`).
- `bun run typecheck` — clean. `bun run lint` — 0 errors, 11 warnings (pre-existing, none
  in a touched file).
- The bun suite one folder at a time, under `flock /home/shiro/rantai/.heavy.lock`:
  **25 folders, every run exited 0**, log `/tmp/gov5-bun.log`. The `src` root run is the
  whole tree: **3016 tests across 304 files — 3015 pass, 1 skip, 0 fail** (94 s; it did not
  OOM here). The per-folder numbers overlap, because a folder run recurses into its
  sub-folders (`src/components` alone reports 7 files, `src/lib` 116); the honest union is
  the 3015 / 1 / 0 of the `src` root run, and the per-folder runs only prove that every
  folder also passes on its own.
- `git status` — clean, nothing generated staged.

**Not verified / skipped**

- `python3 ops/lint/check_intra_package_imports.py`, `python3 ops/lint/check_bare_iceberg_count.py`,
  `(cd dagster && python -m pytest dispar_orchestrate -q)` and `docker compose --profile '*' config --quiet`
  were **not run**: this branch touches no Python, no Dagster and no compose file
  (`AGENTS.md`, "Only build what changed").
- The `bun test src` whole-tree run was not expected to fit in memory here; it did
  (3016 tests in 94 s). The folder-by-folder loop was run anyway, as instructed.

**For the reviewer**

- `lakehouse-core`'s `status::AuditOutcome` (`rust/crates/lakehouse-core/src/status.rs:215`)
  still has three variants. It is dead code — the audit route narrows with its own
  `outcome_of`, and `grep -rn "AuditOutcome" rust/` finds only that definition and a doc
  comment. The plan does not list it, so it was left alone; it now differs from the console
  union, which has four. Say whether it should gain a `Pending` variant.
- `grep -rn "AuditOutcome" src` finds only the four places the plan lists
  (`src/lib/status.ts`, `src/components/patterns/status-badge.tsx`,
  `src/services/contracts/governance.ts`, `src/features/governance/audit-columns.tsx`),
  so nothing else narrows on the union. The outcome filter on the audit page is built from
  `AUDIT_OUTCOME_LABEL`, so it offers "Pending" without another change.

## 7. Review (planner appends below)

### Review round 1 (planner, 2026-10-30, at `675159dc`)

- T1 matches D1 and D2:
  - `outcome_of` maps exactly `needs_confirmation` and `needs_approval` to `pending`;
  - the new pure `policy_decision_of` gives `allow` for success, `pending` for pending and
    `deny` for everything else;
  - both are tested over all eight values.
- T2 matches D3: `pending` / "Pending" and the `info` tone, with a badge test.
  `OUTCOME_OPTIONS` picks the new label up by itself.
- T3 is a no-op, and the handoff shows the search. Agreed: no page lists the outcomes.
- **The `lakehouse-core` `status::AuditOutcome` question:** that enum is used nowhere in Rust
  (`git grep` at `675159dc` finds only a doc mention in `governance.rs`). It is out of scope under
  D4, so it is left as is. Not a finding.

**Open: no BLOCKER, no SHOULD-FIX.** The reviewer's own full run is on `main` merged with this
branch and `BLD-6` (#280). Its counts go in the merge note.

