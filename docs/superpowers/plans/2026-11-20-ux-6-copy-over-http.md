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

## Review
