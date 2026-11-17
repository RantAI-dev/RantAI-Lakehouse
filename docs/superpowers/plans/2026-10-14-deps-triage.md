# Dependabot triage, 2026-10-14

Plan by the planner. Base: `main` after #99, #145 and #146. The product owner
asked to tidy dependabot's open pull requests.

## State found

Since #129 dependabot uses the `bun` ecosystem, so its frontend pull requests
now update `bun.lock`. 22 dependabot pull requests were open.

| Group | Pull requests | Decision |
| --- | --- | --- |
| Green on CI | #135 `serde_json` patch; #141 `sqlparser` 0.62 → 0.63; #132 `motion` 13 → 14; #134 `react-day-picker` 9 → 10; #138 `lucide-react` 0.577 → 1.55 | One batch branch from current `main`, so CI runs once instead of once per pull request (the three frontend ones conflict in `bun.lock`). Merge it, then close the five |
| Superseded | #11, #12, #14, #16, #19, #20, #39, #43 (the old `npm` ecosystem, no `bun.lock`) | Closed by the planner |
| Blocked upstream | #139 `arrow-array`, #140 `arrow-schema`, #142 `parquet` 58 → 60 | Fail to compile: `iceberg = "=0.10.1"` still depends on arrow 58, so two arrow versions meet. Move together with `iceberg` when it supports arrow 60: one dependabot group, backlog `REL-7` |
| Needs code | #131, the group of 20 frontend updates (`next build` fails); #137 `croner` 2 → 4 (API change) | Developer, tasks 2 and 3 |
| Large migrations | #136 TypeScript 7, #143 eslint 10, #133 TanStack Table 9 | Not now: backlog `REL-4`, `REL-5`, `REL-6`; dependabot ignores their major versions until those tickets are planned |
| Stale branch | #69 `pymongo` | Carries history no longer on `main`; the product owner comments `@dependabot recreate` |

## Tasks (developer, branch `deps/2026-10-14-batch`)

1. **Batch the five green updates** on a branch from current `main`:
   - `serde_json` per #135;
   - `sqlparser = "0.63"` per #141;
   - `motion`, `react-day-picker` and `lucide-react` at #132's, #134's and #138's versions.

   Use the tools (`cargo update -p`, `bun add`), never a hand-edited lockfile.
   `sqlparser` feeds the masking rewrite (`sql_rewrite.rs`), so read its 0.63
   changelog for parser changes and say what you found.
   Check: `cargo test -p lakehouse-api` (the rewrite tests); `bun run typecheck && bun run lint && bun run test && bun run build`.
2. **#131.** Find which of the group's 20 updates breaks `next build`.
   - Apply the rest.
   - Fix the code if the fix is small and local.
   - Otherwise leave that package out and name it, with the error, for a
     backlog item.

   Same frontend checks.
3. **#137 `croner` 4.** Update and adapt the call sites to the new API with
   the same behaviour; the tests that cover cron parsing must pass unchanged.
   If the change is not local, stop and report.
   Check: `cargo clippy -p <crate> --all-targets -- -D warnings`;
   `cargo test -p <crate>`.

One commit per task. Full verification block before handoff, for the languages
touched. If a task cannot be finished, push the others and report.

## Not included

- arrow/parquet 60;
- TypeScript 7, eslint 10, TanStack Table 9;
- the `pymongo` recreate.

## Handoff (developer, summarised by the planner)

| Commit | Task | Result |
| --- | --- | --- |
| `439b1fb` | 1 | `serde_json` 1.0.154, `sqlparser` 0.63, `motion` 14.1, `react-day-picker` 10.0.2, `lucide-react` 1.55 |
| `ea4723a` | 2 | 18 of #131's 20 updates. `next` and `eslint-config-next` stay at 16.1.6: CI's build fails with a Bun error the developer could not reproduce locally (`REL-8`). The CodeMirror packages were re-added so the lockfile holds one `@codemirror/state`; a nested copy failed "QueryStudioPage Fix with AI" |
| `d7bc7ee` | 3 | `croner` 4: one `parse_cron` helper (5 fields, no seconds or year, `sloppy_ranges` for `5/10`) |
| `28db2b5` | review fix | A bare step (`/10`) is still refused, as under `croner` 2.x |

Checks run by the developer:
- `cargo fmt --check` and `cargo clippy -p lakehouse-api --all-targets -- -D warnings` passed.
- `cargo test -p lakehouse-api`: lib 1595 passed and bin 1641 passed; 36 integration binaries, 323 passed.
- `bun install --frozen-lockfile`, typecheck, lint, test (1165 pass) and build all passed.

Not verified locally: `cargo deny`/`cargo audit` and the workspace-wide block; CI runs them.

## Review (planner)

- **`sqlparser` 0.63**: the changelog has no change to `SELECT`, identifier, `SETTINGS` or table-function parsing.
  - The developer noted that `substitute_in_function` skips `FunctionArg::ExprNamed`.
  - That is not a leak: `substitute_governed_tables`' post-pass backstop refuses a statement whose governed table was not reached (`UnprovableSubstitution`, `sql_rewrite.rs:484-492`). At worst such a query is refused. No ticket.
- **`croner` 4**: same behaviour as 2.x, including refusing `/10` (review fix `28db2b5`, with a test).
- **#131**: everything but `next` applied. `next` 16.4 is `REL-8`.
- No `BLOCKER`.
