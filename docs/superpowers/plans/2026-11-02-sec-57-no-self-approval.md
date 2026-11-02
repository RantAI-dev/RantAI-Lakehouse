# SEC-57: a person cannot approve their own assistant approval

Backlog `SEC-57` (Task, Administration & Security, P1). Base: `main` at `9c066426`. Planner:
Claude Opus. Developer: another agent (`AGENTS.md`).

## 0. What is wrong, with evidence

When a chat asks for a high-risk (`WriteHigh`) tool call, the API creates an approval
(`kind = 'tool_call'`). Anyone holding `agent:approve` can then decide it on Approvals,
**including the person whose chat asked for it**. A Platform Admin (`*:*`) can therefore ask the
assistant to delete a chart and approve the deletion themselves. The approval step adds nothing.

- **The API does not refuse it.** `routes/agents.rs` `decide_approval` (`:550`) checks the kind
  (`:575-581`) and, on approve, the approver's permission for the tool (`:650-672`). It never
  compares the approver with the requester.
- **The API cannot refuse it today, because the requester is not recorded.**
  `0040_access_requests.sql:45-49` constrains the row: a `tool_call` row must have
  `requested_by_user_id IS NULL`. Its header (`:9-13`) gives the reason: "a digital employee
  acted". `create_pending_approval` (`lakehouse-store/src/agents.rs:1160`) creates the chat's
  approval. It stores only `actor`, the requester's *display name*
  (`routes/ai/mod.rs:1385`), in `agent_run.actor`. A display name is not an identity.
- **Catalog access already has the rule.** `routes/catalog.rs:3858-3865`
  (`decide_access_request`) answers 403
  `ApiError::PermissionDenied("tidak boleh menyetujui permintaan akses milik sendiri")` when
  `existing.requested_by_user_id == Some(principal.id.uuid())`. The check runs before the store
  call, so a refused decision writes nothing. It is proven by
  `tests/agents_approval.rs:441`
  (`access_decide_route_refuses_a_principal_approving_their_own_access_request`).
- **The console has the rule for access requests only.** `src/lib/access-requests.ts:77`
  `isOwnAccessRequest` returns true only for `kind === "access"`. Its test
  (`src/lib/access-requests.test.ts:57`) pins "is false for a tool_call approval even if
  requestedByUserId somehow matched". `approvals-page.tsx:247-249` and
  `approval-columns.tsx:176-182` use it to disable Approve.
- **The existing tests rely on self-approval.** In `tests/agents_approval.rs`,
  `create_pending_delete_chart_approval` (`:58`) asks as `fajar@meridian.example`. The
  approve, reject, double-decide and evidence tests (`:83`, `:137`, `:180`, `:287`) and the
  board tests (`:807`, `:847`) then decide it **as fajar** as well. Under the new rule these
  tests would get a 403.

Source: `docs/core/features/copilot-chat.md:202-206`, open question 2. The product owner decided
on 2026-11-02 that the requester must not approve their own request, with the same rule as
catalog access.

## 1. Decisions

- **D1. The rule (product owner, 2026-11-02).** The person who requested an approval cannot
  decide it. This holds for a tool-call approval exactly as it holds for an access request: the
  same check, the same 403 `permission_denied`, and nothing written on a refusal.
- **D2. One check, not two (rule 4).** The comparison at `catalog.rs:3858-3865` moves into one
  `pub(crate)` helper in `routes/agents.rs`, next to `DecideApprovalBody`, which both routes
  already share (`:443-454`). Proposed name: `refuse_own_request`. It takes
  `(&ApprovalItem, &Principal)` and returns `Result<(), ApiError>`. Both
  `decide_access_request` and `decide_approval` call it right after the kind check and before
  any store write.
- **D3. The refusal text is one sentence for both kinds**, in the language the catalog route
  already answers in: `PermissionDenied("tidak boleh menyetujui permintaan milik sendiri")`.
  This drops the word "akses", so the sentence fits a tool call too. The body stays
  `permission_denied: …` (`lakehouse-core/src/error.rs:56`), so the shape the console reads does
  not change. No test asserts on the old text (`git grep 'milik sendiri'` finds only
  `catalog.rs:3863`).
- **D4. The requester is the signed-in human whose request created the approval.** It is
  recorded in `approval_item.requested_by_user_id` for `tool_call` rows as well:
  - **Chat and `POST /api/ai/tool`** (`request_approval`, `routes/ai/mod.rs:1378`): the
    principal, when it is `PrincipalId::User`.
  - **A headless run started by a person** (`RunAuth::Principal`, `routes/agents.rs:761-783`,
    `create_linked_approval` call at `:1451`): that principal, when it is a user.
  - **A scheduled run** (`RunAuth::Token`) and a **service identity**: `NULL`. No human asked,
    so anyone with `agent:approve` may decide, as today. The column has an FK to `app_user`
    (`0040:30`), so a service id must never be written there.
- **D5. A migration widens the shape constraint. It does not edit `0040` (applied, never
  edited).** The new file is the next free number (`0118` at `9c066426`; re-check `ls
  rust/migrations` when building, because numbers move, see `eb9eaa18`). It drops and re-adds
  `approval_item_kind_shape_check`:
  - `tool_call`: `employee_id IS NOT NULL`, and `requested_by_user_id` may be set or `NULL`;
  - `access`: unchanged.

  Old rows keep `NULL`, so an approval created before this change can still be decided by
  anyone who could decide it before. The migration's header says so.
- **D6. Both decisions are refused to the requester, approve and reject alike**, because the
  catalog check refuses both (it runs before `decision` is looked at). *(planner default)*.
  The alternative is to let the requester reject (cancel) their own tool call. That is a
  different rule from catalog access, so it needs the owner's word. Until then the requester
  waits for someone else, or the approval stays pending.
- **D7. No break-glass on a one-admin install.** On an install with one `agent:approve`
  holder, that person's own high-risk chat calls cannot be approved by anyone. This follows from
  D1. It is written in the feature page's limits, not worked around. *(planner default)*.
- **D8. The console matches the API.**
  - `isOwnAccessRequest` becomes `isOwnRequest`, true for a pending row of **either** kind whose
    `requestedByUserId` is the user.
  - Approve and Reject are both disabled for such a row, in the drawer (`approvals-page.tsx:63-80`)
    and in the row menu (`approval-columns.tsx:176-190`). Today the menu's Reject stays enabled
    for an own access request, and the API refuses it with a 403. That is fixed by the same
    change.
  - The tooltip is per kind:
    - access: "You requested this access; someone else must decide it";
    - tool call: "You asked for this action; someone else must approve it".
  - The chat card for a waiting approval (`src/features/copilot/tool-step.tsx:221-232`) gains one
    line under the summary: "Someone else with approval rights must approve it."
- **D9. Who requested it is shown on Approvals.** The API already serializes
  `requestedByUserId` (`lakehouse-store/src/agents.rs:889-890`). The contract doc
  (`src/services/contracts/agents.ts:119-128`) is corrected, because it says "set only for
  `kind = "access"`". No new field.

## 2. Anchors (verified at `9c066426`)

- `rust/crates/lakehouse-api/src/routes/catalog.rs:3820-3823` (doc), `:3858-3865` (the check
  to move).
- `rust/crates/lakehouse-api/src/routes/agents.rs`:
  - `DecideApprovalBody` `:443-454`;
  - `decide_approval` `:550`, kind check `:575-581`, store call `:583`;
  - `RunAuth` `:761-770`;
  - `principal_for_run_auth` `:778-783`;
  - `create_linked_approval` call `:1451-1465`.
- `rust/crates/lakehouse-api/src/routes/ai/mod.rs:1378-1400` (`request_approval`, `actor` `:1385`).
- `rust/crates/lakehouse-api/src/routes/ai/gate.rs:216-259` (`create_write_high_approval`).
- `rust/crates/lakehouse-store/src/agents.rs`:
  - `ApprovalItem.requested_by_user_id` `:884-890`;
  - `NewApprovalRequest` `:1113`;
  - `create_pending_approval` `:1160`, insert `:1186-1201`;
  - `LinkedApprovalRequest` `:1442`;
  - `create_linked_approval` `:1474`, insert `:1478-1493`.
- `rust/migrations/0040_access_requests.sql:9-13`, `:28-30`, `:45-49`.
- `rust/crates/lakehouse-api/tests/agents_approval.rs` `:58`, `:83`, `:137`, `:180`, `:287`,
  `:441`, `:807`, `:847`, `:878`; `rust/crates/lakehouse-store/tests/agents.rs:533`.
- `src/lib/access-requests.ts:55-83`, `src/lib/access-requests.test.ts:38-75`.
- `src/features/agents/approvals-page.tsx:25`, `:63-80`, `:247-249`, `:339`.
- `src/features/agents/approval-columns.tsx:69`, `:153`, `:176-190`.
- `src/features/copilot/tool-step.tsx:221-232`.
- `src/services/contracts/agents.ts:119-128`.
- `docs/core/features/copilot-chat.md:58-59`, `:116+` (limits), `:202-206` (open question 2).

## 3. Tasks (one commit each, in order)

- **T1. The store and the migration.**
  - The new migration (D5) has a why-header citing `SEC-57`, `0040`, and the old-rows note.
  - `NewApprovalRequest` and `LinkedApprovalRequest` gain `requested_by: Option<Uuid>`. Both
    inserts bind it to `requested_by_user_id`.
  - Correct the `ApprovalItem.requested_by_user_id` doc (`:884-889`): it is set for a tool call
    that a person asked for.
  - Store tests (`sqlx::test`):
    - a `tool_call` row with a requester is stored and read back;
    - one without a requester stays `NULL`;
    - an `access` row without a requester is still refused by the constraint.
  - *Accept:* `cargo test -p lakehouse-store agents` and
    `cargo clippy -p lakehouse-store --all-targets -- -D warnings`.
- **T2. The API: record the requester, refuse self-decisions.** This task sits with T1 in the same
  sitting (`sqlx::test` embeds migrations).
  - `request_approval` (`ai/mod.rs:1378`) passes `principal`'s id when it is
    `PrincipalId::User`. `create_write_high_approval` gains the parameter and forwards it.
  - `run_headless_loop` passes the run's principal (D4) to `LinkedApprovalRequest`.
  - Add `refuse_own_request` in `routes/agents.rs` (D2, D3), with a doc comment citing
    `SEC-57` and the catalog provenance (WS7 E3). `decide_access_request` calls it in place of
    its inline check; that site keeps a one-line comment saying where the check went.
    `decide_approval` calls it after the kind check (`:581`) and before `agents::decide_approval`
    (`:583`).
  - Update both handlers' `# Errors` sections.
  - Tests in `tests/agents_approval.rs`:
    - **new:** `agent_decide_route_refuses_the_requester_approving_their_own_tool_call`.
      fajar asks, fajar approves, the answer is 403 with
      `"permission_denied: tidak boleh menyetujui permintaan milik sendiri"`, the approval is
      still `pending`, the run is still `waiting_approval`, and the chart still exists;
    - **new:** `…refuses_the_requester_rejecting_their_own_tool_call` (D6);
    - **new:** `…allows_a_different_approver` (the full flow with a second principal);
    - **new:** `a_scheduled_runs_approval_has_no_requester_and_any_approver_may_decide_it`,
      in `tests/agents_run.rs`, whose harness runs headless employees; otherwise
      a store-level test, with the reason in the handoff;
    - **changed:** every test that decided its own approval (`:83`, `:137`, `:180`, `:287`,
      `:807`, `:847`) decides it as a **second** principal, made with
      `create_principal_with_permissions(&pool, "dashboard:write, dashboard:read, agent:approve")`.
      Each test still asserts what it asserted before. The commit body names this as the reason
      for the change (rule 2: the tests encoded the old rule; they are not weakened);
    - **unchanged:** the access self-approval test (`:441`), which passes with the shared helper.
  - Check `tests/ai_report_tools.rs`: its fixture already uses a separate approver
    (`:88-94`). Confirm it is green, and do not change it.
  - *Accept:* `cargo test -p lakehouse-api --test agents_approval`,
    `cargo test -p lakehouse-api --test ai_report_tools`,
    `cargo test -p lakehouse-api --test route_auth` and
    `cargo clippy -p lakehouse-api --all-targets -- -D warnings`.
- **T3. The console.**
  - Rename `isOwnAccessRequest` to `isOwnRequest` and drop the `kind === "access"` condition.
    Update its doc.
  - In `access-requests.test.ts`, the `:57` case becomes "is true for a pending tool_call
    approval requested by the given user". The commit says why (the rule changed, `SEC-57`).
    Add a case for a tool_call row with no `requestedByUserId`, which is false.
  - The drawer and the row menu disable both Approve and Reject for an own row, with the tooltip
    per kind (D8).
  - Add the chat card line (D8).
  - Correct the contract doc (D9).
  - *Accept:* `bun run typecheck`, `bun run lint`, and
    `bun test src/lib/access-requests.test.ts`.
- **T4. The feature page.** Done by the planner or the developer; no product code.
  - `copilot-chat.md`: open question 2 becomes a decision row ("The requester cannot decide
    their own approval", `SEC-57`, product owner 2026-11-02).
  - Step 8 says "someone other than the person who asked".
  - The limits gain D7.
  - Add an acceptance line: ask as A, try to approve as A and see it refused, approve as B and
    see it run.
  - *Accept:* the page renders, and `git diff` touches only that page.

Full block before handoff: the Rust line and the TypeScript line of `AGENTS.md` "Verification".

## 4. Out of scope

- Letting the requester cancel their own pending approval (D6 alternative).
- A two-person rule or a quorum, or a break-glass for a single-admin install (D7).
- Backfilling `requested_by_user_id` on old rows from `agent_run.actor`, a display name, which is
  not an identity.
- Translating the other Indonesian gate texts (`gate.rs`), which are an `AI-23` neighbour, not
  this item.

## 5. Handoff (developer appends below)

## 6. Review (planner appends below)

---

### Proposed BACKLOG.md row

| `SEC-57` | Task | A person can approve their own assistant approval: `decide_approval` (`routes/agents.rs`) never compares the approver with the requester, and a `tool_call` approval does not record one (`0040` forces `requested_by_user_id IS NULL`). Catalog access already refuses it (`routes/catalog.rs`, 403). Product owner decided 2026-11-02: the same rule for both. Record the requesting user on tool-call approvals (new migration), move the catalog check into one helper both routes call, and disable Approve and Reject on Approvals for one's own row | Administration & Security | P1 | Todo |  | `features/copilot-chat.md` open question 2; owner decision 2026-11-02 |


**Migration number (planner, 2026-11-02):** this plan uses `0118`. `AI-23` runs after it and uses `0119`.
