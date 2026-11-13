# BI follow-ups: export safety, reports and print view, alerts and hygiene

Planner: Claude Opus (hub session). Developer: a non-Opus agent (Sonnet). Base: `origin/main` at
`6247400f` (2026-11-13). The product owner wants the open `BI` and `RPT` follow-ups closed this
week. This plan covers the Todo items that need code: `BI-27`, `BI-28`, `BI-29`, `BI-30`,
`BI-33`, `BI-34`, `BI-35`, `BI-36`, `BI-42` and `BI-43`. The item texts in `docs/core/BACKLOG.md`
are the brief; this plan adds the decisions and the slicing.

Anchors are file names from the backlog rows and a first look at `main`. **The developer
verifies every anchor at `6247400f` before changing code, and stops and reports if one is wrong**
(AGENTS.md, "the plan is wrong against the code").

## Decisions already made

- **D1, `BI-43` (owner, 2026-11-13).** List the person's own alerts for a chart in the chart's
  alert dialog, with remove. The alert action stays visible to anyone who may save an alert. A
  person sees and removes only the alerts they created. Removing someone else's alert still
  needs `alert:write` on the Alerts page.
- **D2, `BI-30`: one CSV owner.** `csv_safe::neutralize_csv` (from #86) is the one rule for
  formula neutralising. Quoting and the BOM move to one shared Rust helper that both
  `csv_export.rs` and `upload_workbook.rs` call. `src/lib/csv.ts` neutralises with the same rule
  as the Rust one, and a shared test vector file proves the two agree.
- **D3, `BI-27` and `BI-34`: bounded exports.**
  - A whole-result export holds one copy of the rows: take them, don't clone.
  - Every export statement carries `max_memory_usage`, a byte cap and a timeout. A cut export
    says so in the file name and the audit row, as `BI-1` already does for its row cap. It is
    never silent.
  - Concurrent downloads are limited per process, the same way as the PDF's two slots. A
    refused download answers 429 with a fixed sentence.
  - The XLSX row writes run in `spawn_blocking`.
  - **Values:** the developer picks them from the existing PDF and `BI-1` settings, states them
    in the handoff, and makes them configurable the same way those are.
- **D4, `BI-28`: honest exports.**
  - Number columns are decided from `result.meta`, so negative `Int64` values are written as
    numbers, not `'-5`.
  - When a tile skipped rows (`RecordsSql.skipped`), the file carries a first-row note in CSV and
    a note sheet in XLSX, saying how many rows were skipped and why.
  - A raw table with a calculated column exports. If one calculated column truly cannot be
    computed in the export path, it is exported empty with a note, never as a 400.
- **D5, `BI-35`: scheduled reports.**
  - A requester cannot approve their own outside send, even with `agent:approve`. The approval
    must come from another person.
  - "Send now" is rate-limited per subscription. One send per subscription per 60 s is the
    default, and it is configurable.
  - A report cut by the 310 s route budget is recorded as failed, with the reason, in the run
    history and the audit.
  - The subscriptions dialog uses `useServiceAction`.
  - Console Delete of a subscription asks for confirmation, with the subscription's name.
- **D6, `BI-36` and `BI-42`: the print view.**
  - The print view (`render-view.tsx`, `/render`) reads uploaded map boundaries through a render
    token route, as the public and embed views do since #246.
  - The render token carries the board's content sites, as the embed frame call does, so image
    and web cards draw in a PDF or a PNG. No wider policy than the board's own sites.
- **D7, `BI-29`: services boundary.** Dashboard feature code reaches clients only through
  `@/services`. `filters/use-filter-values.ts` moves to `useService`. A lint rule
  (`no-restricted-imports` for `@/services/clients/*` under `src/features/`) keeps it that way.
- **D8, `BI-33`: the #199 review leftovers.**
  - The AI-15 case `dash_map_charts_in_separate_tab` gets a second chart in its setup.
  - The XSS test covers the sankey edge, the boxplot outlier and the scatter-label branches.
  - The wrong comment in `chart-option.ts` is corrected.

## Slicing: three PRs, built one after another (the VM is shared and busy)

| PR | Branch | Items | Languages |
|---|---|---|---|
| A | `fix/bi-export-safety` | `BI-27`, `BI-34`, `BI-28`, `BI-30` | Rust, TS |
| B | `fix/bi-reports-render` | `BI-35`, `BI-36`, `BI-42` | Rust, TS |
| C | `fix/bi-alerts-hygiene` | `BI-43`, `BI-29`, `BI-33` | Rust, TS |

- One backlog item per commit, in the order above. The commit messages cite the item IDs.
- PR titles start with the first item ID, for example `BI-27 fix(api): …`.
- PR B branches from `main` after A is opened (not merged), and so does PR C after B.
  Conflicts are resolved hunk by hunk.

## Acceptance checks per item

- `BI-27`, `BI-34`: a test proves an export over the byte cap is cut and labelled. A test proves
  a third concurrent download gets 429. The XLSX write path is in `spawn_blocking` (code
  review). No row clone remains on the whole-result path.
- `BI-28`: tests for a negative `Int64` (written as a number), a skipped-rows note, and a raw
  table with a calculated column exporting with status 200.
- `BI-30`: one Rust helper, and a shared vector test that passes in Rust and in TS.
- `BI-35`: tests for self-approval refused, a second Send now within 60 s refused, and a
  timed-out run recorded as failed. Console: Delete asks first. The dialog uses
  `useServiceAction`.
- `BI-36`, `BI-42`: a render-view test proving the map boundary loads through the token route,
  and a test that the render token's policy contains the board's content sites and nothing else.
- `BI-43`: an API test that a person lists and removes only their own alerts for a chart, and
  is refused for another person's. The console dialog shows "Your alerts on this chart" with
  remove.
- `BI-29`: the lint rule is in, and `bun run lint` is clean.
- `BI-33`: the three tests are extended, and the comment is fixed.

## Verification

Use the AGENTS.md block for the languages each PR touched, run once on the PR's final commit,
in the foreground:
- Rust: `cargo fmt --check`, clippy with `-D warnings` and `cargo test --workspace`, with the
  shared `CARGO_TARGET_DIR`;
- TS: typecheck, lint and tests.

The VM is shared and busy, so build with `nice` and `CARGO_BUILD_JOBS=2`.

## Out of scope

- The rest of `BI-17` (forecasting, clustering) and `BI-18` (reports, templates).
- `BI-3`, `BI-20`.
- `BI-16` close-out (docs only, planner).
- `BI-31` part 3 and `BI-32` (owner checks on a running stack).

## Handoff

(The developer appends one entry per PR: commits, the commands run with counts, the values
chosen for D3 and D5, and anything not verified.)
