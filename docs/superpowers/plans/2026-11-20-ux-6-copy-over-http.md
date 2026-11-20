# `UX-6`: every copy button works over plain http

Planner and reviewer: Claude Opus. Developer: DeepSeek V4.1 Flash (owner
2026-11-20). Base: `main` at `d85b0eeac`. Branch: `ux-6-copy-over-http`.

## Why

QA found that `navigator.clipboard` does not exist outside a secure context
(https or localhost). Staging and an on-prem console opened by IP over http
have none (`LH-QS-2-04` F4; `LH-DATA-11-08` F1 asked for "audit +
fallback"). `src/lib/copy-text.ts` (`copyText`) already falls back to a
textarea and `execCommand("copy")`, but only six files use it. **32 calls in
22 files** still call `navigator.clipboard` directly: over http they throw,
or fail silently behind `?.` and `() => {}`. The worst are the one-time
password and secret reveal screens, where an admin cannot copy the value
that is shown once.

## Decisions (planner, 2026-11-20)

1. Every copy in `src/` goes through `copyText` from `@/lib/copy-text`. No
   file outside `src/lib/copy-text.ts` touches `navigator.clipboard`. Test
   files may mock it.
2. A copy shows success only when `copyText` resolves `true`. When it
   resolves `false`, the user sees `notifyFailure("Could not copy", "The
   browser refused the copy. Select the text and copy it by hand.")` from
   `@/lib/notify`. The check icons and "Copied" states (`copy-button.tsx`,
   `chat-messages.tsx`) flip only on `true`. Sites that showed no toast
   before keep their success behaviour (none, or the icon) and gain only
   the failure toast.
3. A lint rule keeps it that way: in `eslint.config.mjs`, a **new** block
   for `src/**/*.{ts,tsx}` with `ignores: ["src/lib/copy-text.ts",
   "src/**/*.test.{ts,tsx}"]` and `no-restricted-properties` on `object:
   "navigator", property: "clipboard"`, message naming `copyText` and
   `UX-6`. Do **not** add it to the existing `no-restricted-syntax` array:
   flat config would then replace the `WEB-9` select rule in that block.
   The block carries a why-comment like its neighbours.
4. `copyText` itself does not change. Out of scope: `crypto.randomUUID`
   (already guarded; `use-copilot.ts:68` checks first), any new copy
   button, any wording change beyond decision 2.

## Anchors (at `d85b0eeac`)

`src/lib/copy-text.ts` (the helper); a site done right:
`src/features/queries/query-cell-inspector.tsx:44`. The 22 files:

```
src/components/copyable.tsx (1)
src/features/admin/roles-columns.tsx (1)
src/features/admin/service-identities-columns.tsx (1)
src/features/admin/service-identities-page.tsx (1)   secret reveal
src/features/admin/tenants-columns.tsx (1)
src/features/admin/user-columns.tsx (3)
src/features/admin/users-page.tsx (1)                password reveal
src/features/agents/approval-columns.tsx (1)
src/features/catalog/asset-overview.tsx (1)
src/features/catalog/data-explorer-actions.tsx (1)
src/features/catalog/sample-inspector.tsx (1)
src/features/connectors/connector-setup-checks.tsx (1)
src/features/copilot/chat-messages.tsx (1)
src/features/copilot/copy-button.tsx (1)
src/features/governance/audit-columns.tsx (4)
src/features/governance/data-quality-columns.tsx (2)
src/features/governance/lineage-columns.tsx (3)
src/features/governance/policy-columns.tsx (2)
src/features/pipelines/pipeline-columns.tsx (1)
src/features/pipelines/run-inspector.tsx (1)
src/features/pipelines/run-log-console.tsx (1)
src/features/queries/saved-query-columns.tsx (2)
```

Re-find them with `grep -rn 'navigator.clipboard' src | grep -v '\.test\.'`.

## Tasks

- **T1 — the lint rule first.** Add the block from decision 3. Check:
  `bun run lint` fails and names exactly the 32 sites above (quote the
  count). Commit the rule together with T2 only if you cannot commit a red
  lint; otherwise commit it alone and say so in the message.
- **T2 — shared components.** `copyable.tsx`, `copilot/copy-button.tsx`,
  `copilot/chat-messages.tsx`. Each uses `copyText` and decision 2. Add a
  test for `CopyButton` and `Copyable`: with `navigator.clipboard` absent
  and `document.execCommand` stubbed to return `true`, the click shows the
  success state; stubbed to `false`, it shows the failure toast and no
  success state. The test must fail on the old code (say how you checked).
- **T3 — the reveal screens.** `users-page.tsx`, `service-identities-page.tsx`.
  Note: the textarea fallback runs inside an open dialog; `copyText` already
  appends the textarea to the dialog. Check it on the page, not only in a
  unit test, if you can run the console; otherwise write *not verified*.
- **T4 — the remaining 17 files.** Row menus and pages. Mechanical: replace
  the call, add the failure toast.
- **T5 — verify and hand off.** `grep -rn 'navigator.clipboard' src | grep
  -v '\.test\.'` prints only `src/lib/copy-text.ts`. Then the frontend block
  below, and a Handoff entry at the end of this file.

One commit per task, pushed after each commit.

## Verification (frontend only; no Rust, Python or compose is touched)

```bash
bun run typecheck && bun run lint && bun run test
git status
```

## Out of scope

The download block (`LH-QS-2-06` F1) is already fixed. `LH-DATA-11-08` F3
(the row menu has no Edit) is a separate item.

## Handoff

Developer: DeepSeek V4.1 Flash, 2026-11-20. Base `main` at `d85b0eeac`,
branch `ux-6-copy-over-http`. Five commits, each pushed after it was made.

| Task | Commit | What |
| --- | --- | --- |
| T1 | `5dab2956b` | lint block in `eslint.config.mjs` (red on purpose) |
| T2 | `7ca0a2b4c` | `copyable.tsx`, `copy-button.tsx`, `chat-messages.tsx` + 2 test files |
| T3 | `2a9320635` | `users-page.tsx`, `service-identities-page.tsx` + 2 page tests |
| T4 | `f916644d8` | the remaining 17 files (21 calls) |
| T5 | `0ccd011c5` | pin the detail a racy `schema-browser` test waits on (see below) |

T1 was committed alone with lint red (32 errors, exactly the 22 files this
plan lists), because nothing in the repo blocks a red pre-commit hook. T4
removes all 32; the 12 warnings on `main` are untouched.

`grep -rn 'navigator.clipboard' src | grep -v '\.test\.'` prints only
`src/lib/copy-text.ts` (4 lines: the helper's own doc comment and body).
The comments added in T2–T4 say "clipboard API" so they do not match.

### Commands run, on the final commit (`0ccd011c5`)

```bash
bun run typecheck                          # exit 0, no output
bun run lint                               # exit 0, 12 warnings, 0 errors
bun run test                               # exit 0: 4627 pass, 1 skip, 0 fail,
                                           #   14658 expect() calls,
                                           #   4628 tests across 429 files
git status                                 # clean
```

Frontend only: no cargo, Python or Docker command was run, and nothing
outside `src/` and `eslint.config.mjs` changed.

### Tests added, and how they were checked against the old code

- `src/components/copyable.test.tsx`, `src/features/copilot/copy-button.test.tsx`:
  `navigator.clipboard` removed for the whole file (the staging situation),
  `document.execCommand` stubbed `true`/`false`. Checked by stashing the two
  components and re-running: **4 fail, 0 pass** there
  (`TypeError: undefined is not an object (evaluating 'navigator.clipboard.writeText')`
  for `CopyButton`; no "Could not copy" toast for `Copyable`), 4 pass on the
  commit.
- One page test each in `users-page.test.tsx` and
  `service-identities-page.test.tsx`: click Copy with no clipboard API and
  assert the textarea `execCommand` sees holds the revealed value **and sits
  inside the dialog**. Checked the same way (both pages stashed): **2 fail**
  on the old code, 2 pass on the commit.

### Not verified

- **The reveal screens on a running console over plain http** (`UX-6` T3
  asked for this). This session had no stack to open the console against, so
  the dialog-level check is the two page tests above, which run in happy-dom
  with `document.execCommand` stubbed — not a real browser over http. The
  textarea is appended to the active element's `[role=dialog]` ancestor;
  the tests focus the Copy button first, because `fireEvent.click` does not
  move focus in happy-dom while a real click does (Chrome, Firefox).
- A click in a browser that does not focus a button on click (Safari on
  macOS) leaves `document.activeElement` on the dialog itself or on body;
  then the fallback is not inside the dialog and the copy can be refused —
  the user gets the failure toast and is told to copy by hand. That is the
  accepted behaviour in decision 2, and it is not measured.

### The one test this branch had to touch outside `UX-6`

`schema-browser.test.tsx` (`QS-4`) has one test that asserts a *loading*
state ("keeps the table's name unknown … until its detail loads") while
`stubServices` resolves `getAsset` at once. Adding a second `UsersPage`
render in `users-page.test.tsx` was enough to lose that race: the row was
already insertable and the click inserted `SELECT * FROM undefined.undefined`
(the name built from the empty detail). It reproduces on the base commit
with a test that only renders `UsersPage` once more, so it is the test's race,
not this change: `git worktree` at `0061cd790` + one extra page-rendering
test in `users-page.test.tsx` → same failure; the same worktree without it →
0 fail, twice. `0ccd011c5` holds the detail pending in that test, which is
the state its name describes; no assertion changed and it still fails if the
click ever inserts something. The reviewer should look at that commit on its
own, as it is outside the plan's task list.

## Review

Planner review, 2026-11-20, at `c01af29eb`. No BLOCKER.

- Checked: no `navigator.clipboard` left in `src/` outside
  `src/lib/copy-text.ts`; the lint block is its own block and leaves the
  `WEB-9` select rule alone; success shows only on `true` at the sites read
  (`copyable.tsx`, `copy-button.tsx`, `chat-messages.tsx`, `users-page.tsx`,
  `audit-columns.tsx`); the new tests were shown to fail on the old code.
- `0ccd011c5` (the `schema-browser` race) is outside the plan but justified
  in its commit: no assertion changed, the test now holds the state its
  name describes. Accepted.

**R1 SHOULD-FIX: one failure toast, written 34 times.** The sentence
"Could not copy" / "The browser refused the copy. Select the text and copy
it by hand." is repeated in 24 files, most inside the same
`copyText(x).then((copied) => { if (copied) notifySuccess(...) else
notifyFailure(...) })` block. Rule 4 (`AGENTS.md`): one helper. Add to
`src/lib/copy-text.ts`:

```ts
/** Copies `text` and tells the user: `success` (if given) on success, the
 *  fixed failure toast otherwise. Resolves to whether it worked. */
export async function copyTextWithToast(text: string, success?: string): Promise<boolean>
```

and use it at every site that only toasts. Sites that flip an icon
(`copy-button.tsx`, `chat-messages.tsx`) call it without `success` and set
the icon on `true`. Add one unit test for the helper (success toast on
`true`, failure toast on `false`, no success toast on `false`). The
failure text then exists once in `src/` (check: `grep -rc 'refused the
copy' src | grep -v ':0'` lists only `copy-text.ts` and tests).
