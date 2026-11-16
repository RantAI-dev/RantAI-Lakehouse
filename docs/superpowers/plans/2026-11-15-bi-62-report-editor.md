# BI-62: the report editor as a document (mockup D), plan

| | |
| --- | --- |
| Backlog | `BI-62` |
| Spec | `docs/core/specs/bi-62.md` |
| Feature page | `docs/core/features/report-editor.md` |
| Base | `feat/bi-61-dashboard-page` at `96491b8d5` (PR #541: the shared side panel with tabs, which contains #528 reports and #530/#531). This PR follows #541 |
| Branch | `feat/bi-62-report-editor`, worktree `/home/hv/mtharreq/lakehouse-bi62` |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made

- **Owner QA 2026-11-16: no Preview button** — the editor already shows the reader's view.

- **D1, layout = mockup D (owner, 2026-11-15, "aku suka yang D ini, gaskan kita implementasikan").**
  The reference is the local branch `design/chart-builder-mockups` (worktree
  `/home/hv/mtharreq/lakehouse-mockups`), route `/mockups/report-editor/d`, code in
  `src/features/mockups/report-editor/` (`editor.tsx`, `extensions.ts`, `menus.tsx`, `chart.tsx`,
  `draft.tsx`, `frame.tsx`, `thread.ts`, `ui.tsx`, `report-editor.css`, `mockups.tsx`). Read it
  with `git -C /home/hv/mtharreq/lakehouse-mockups show <commit>:<path>` at the commit the planner
  names in the "Reference commit" line below (the D mockup is being finished; until that line is
  filled, read the worktree as it is and say so in the handoff). The mockup code is ours (same
  repository, AGPL): port it, do not copy third-party component code. Copy the look and the
  behaviour, not the fixtures.
  - Reference commit: *(filled by the planner when the D mockup is committed)*.
- **D2, editor library: Tiptap 3, MIT packages only, exact versions.** The mockup uses
  `@tiptap/react`, `@tiptap/pm`, `@tiptap/core`, `@tiptap/starter-kit`, `@tiptap/extensions`,
  `@tiptap/extension-typography`, `@tiptap/suggestion`, `@tiptap/extension-bubble-menu`, all
  `3.34.0`, MIT. Add exactly those the editor needs, pinned (no `^`), and check each new transitive
  dependency's licence (the repo is AGPL-3.0: MIT, Apache-2.0, BSD and ISC are fine; ask before anything else). No Tiptap Pro, no
  `@tiptap/extension-drag-handle` (it pulls collaboration packages): the drag handle is ours, as
  in the mockup. The owner has used Tiptap before.
- **D3, storage stays a list of blocks; the editor is a view of it.** No new storage format and
  no migration: `lakehouse_bi::reports::Block` (`rust/crates/lakehouse-bi/src/reports.rs:66`)
  grows, everything old still parses.
  - New block kinds: `quote { text }` and `divider {}`.
  - `chart` gains `width: "text" | "wide"` (absent = `"text"`).
  - Heading levels written by the editor are 1 and 2; 3 still reads.
  - **Inline marks are a Markdown subset inside the text fields** of heading, paragraph, list
    items and quote: `**bold**`, `*italic*`, `[words](address)`, with `\` escaping `\ * [ ] ( )`.
    Why: one string per field keeps the schema and every reader (console, PDF render view,
    assistant tools, exports) simple, plain text reads the same, and the assistant already writes
    Markdown. The editor escapes what a person types, so a literal `*` stays a `*`.
  - The server checks every inline link address with the same rule as the `link` block (today's
    `safe_url` in `reports.rs`; `https://`, `http://`, `mailto:`, a console path), and refuses a
    report holding another one with a fixed sentence (a new `ReportRule` variant). Limits are
    unchanged (30 charts, 300 blocks, 10,000 characters per text, 100 list items).
  - **Old `link` blocks** still parse, read and print as today. The editor shows one as a
    paragraph holding an inline link and saves it that way (the only lossless rewrite).
- **D4, autosave replaces Save.** Save about 1 s after typing stops, and immediately on blur,
  on leaving the page (route change) and on `pagehide`/`beforeunload` (best effort). The header
  shows Saving… · Saved · Couldn't save · Retry (DESIGN.md: one short status line). A new report
  is created on its first non-empty change (title or any block), then the URL becomes its edit
  URL without a navigation flash. Last write wins when two people edit (as today, a limit on the
  feature page). A server refusal (e.g. the 31st chart, an unsafe link) shows its sentence next to
  the status and keeps the local text.
- **D5, charts.** A chart block is a node drawing the live chart through the existing
  `ReportingProvider` / `TileBody` path, in a soft card without border (DESIGN.md), with the
  chart title and "from <dashboard>" small above it, and a hover toolbar: Swap · Text width/Wide ·
  Remove. Pickers show **live previews grouped by dashboard** with search (DESIGN.md: choices are
  previews). Thumbnails render lazily when visible.
- **D6, the Charts shelf is a tab of the one side panel** (`useSidePanelTabs`,
  `src/components/app-shell/side-panel.tsx`, BI-61 T6): tabs **AI Copilot · Charts**. Drag a
  thumbnail into the document or click to insert at the cursor. On a wide screen the panel opens
  on Charts for an empty report; otherwise it keeps its last state. On a phone it is a sheet.
  DESIGN.md: one side panel at a time, one header, close by divider click / dock / Esc.
- **D7, the reader page, the editor and the PDF use one renderer** (`report-blocks.tsx`): marks,
  quote, divider and chart width render the same everywhere; marks are built as React nodes
  (never `dangerouslySetInnerHTML`), links only through `safeReportUrl`.
- **D8, the assistant in the page (PR B).** "Ask AI to write…" (first item of `/`) and
  "Describe" (a chart's hover toolbar) send a request to the existing assistant in Ask mode with
  the report as page context (title, blocks as Markdown, the chosen chart's id, title, dashboard
  and the rows the reader can see, capped), stream the answer into an inline **draft** node, and
  Accept (Enter) turns it into blocks with the D3 parser; Discard (Esc) removes it. The exchange
  is one assistant conversation, so it also appears in the AI Copilot tab. No sparkles; the
  `Bot` mark. Drafts never save until accepted.
- **D9, assistant parity (AGENTS.md).** The assistant's report tools `create_report` /
  `update_report` (`rust/crates/lakehouse-api/src/routes/ai/registry.rs:2015-2035, 2893-2905`,
  prompt hints `prompt.rs:877, 1205`) accept the new block kinds, the chart width and the inline
  Markdown subset; regenerate (hand-edit, see Verification) `tests/fixtures/tool_schemas.json`;
  the standard request set (`AI-15`, `ops/ai_eval/dashboard_cases.json`) gains "write a report
  with a bold sentence, a quote and a wide chart". This is in PR A so the assistant never writes
  a report the editor cannot show.
- **Phone, motion, focus.** No drag handles on touch (the mockup does the same); `/`, Alt+↑/↓ and
  tapping a chart (its toolbar) work. Reduced motion: no animated widths or slides. Focus rings on
  everything. Mono font only for code.

## 2. Tasks

PR A (editor, storage, reader, PDF, Charts tab, assistant tools):

1. **T1 (Rust) Blocks grow.** `reports.rs`: `Quote`, `Divider`, `Chart.width`, the inline-link
   check with its `ReportRule` and sentence, unit tests (parse old and new JSON; a `javascript:`
   inline link refused; escaped brackets are not a link; limits unchanged). Assistant tool
   schema and prompt hints (D9) and the fixture. *Check:* `cargo fmt --check`,
   `scripts/ci/check_test_modules.sh`; compile, clippy and tests in CI (no Rust compile on the
   VM, owner rule 2026-11-14).
2. **T2 (TS) Contract and the inline Markdown subset.** `src/services/contracts/dashboard-reports.ts`
   types; `src/lib/report-inline.ts`: parse to tokens, serialise from Tiptap marks with escaping,
   `safeReportUrl` on links; round-trip tests incl. hostile input (`[x](javascript:…)`,
   unbalanced `**`, nested marks, backslashes). *Check:* scoped `bun test`, typecheck.
3. **T3 One renderer.** `report-blocks.tsx` renders marks, quote, divider, chart width; reader
   page, the editor and `report-render-view.tsx` (PDF) use it. *Check:* render tests for each kind.
4. **T4 The editor.** Replace `report-editor-page.tsx`'s form with the D editor: title line,
   `/` menu (grouped, height-capped with edge fade, keyboard), Markdown shortcuts, bubble menu
   (bold, italic, link with `safeReportUrl`), ⋮⋮ and + handles, Alt+↑/↓, chart node (D5), picker,
   Download PDF, N-of-30 near the limit, old link blocks mapped (D3). The editor converts
   blocks → document on load and document → blocks on save (pure functions, unit-tested
   round trip on every block kind). *Check:* tests for slash filtering, shortcut mapping,
   conversion round trip.
5. **T5 Autosave (D4).** A small state machine (idle → dirty → saving → saved | error) with
   debounce, flush on blur/leave, create-on-first-change, refusal sentences kept beside the
   status. *Check:* unit tests with fake timers and a fake service (debounce, flush, retry,
   create then update, refusal).
6. **T6 Charts tab (D6).** Register the tab on the report editor page; thumbnails grouped by
   dashboard, search, drag and click insert, lazy render, sheet on phone. *Check:* tests for tab
   registration, click insert, search.
7. **T7 Finish.** Light/dark, 1190×800, 1366×900, 390×844 screenshots (headless Chrome against a
   production build is not possible on the VM without the QA stack; take them from the QA stack
   after the planner deploys, or from a component harness), reduced motion, focus. Handoff.

PR B (the assistant in the page, D8), on top of PR A:

8. **T8** inline drafts: `/` "Ask AI to write…" (one-line prompt with three suggestions as in
   the mockup), chart "Describe", streaming into a draft node, Accept/Discard, the same
   conversation in the AI Copilot tab, page context as in D8. *Check:* tests with a fake stream:
   draft appears, Accept inserts parsed blocks, Discard leaves the document unchanged, nothing is
   saved before Accept.
9. **T9** AI-15 cases for the editor ("describe this chart", "write a short summary of this
   report"); handoff.

## 3. Verification (VM rules, 2026-11-14 and 2026-11-15)

- **No Rust compilation on the VM** (`cargo build/check/clippy/test` OOM the shared VM). Rust is
  written carefully and verified by GitHub CI on the draft PR; locally only `cargo fmt --check`
  and `scripts/ci/check_test_modules.sh`. The fixture `tests/fixtures/tool_schemas.json` is
  edited by hand to match the new schema; CI's comparison test confirms it.
- TS: `bun install` once (after adding the pinned Tiptap packages), then only scoped tests and
  typecheck, each under `flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -p
  MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`; `npx eslint` on changed files. Never the full
  `bun run test` or `bun run lint` (CI runs them). No dev server, no prettier.
- Watch for test pollution (CI runs all files in one process): restore every global and
  `mock.module` you change, and set up every global your test relies on.
- QA: the planner deploys the console to the QA stack from the preview branch; the API image with
  T1 comes from the PR's CI image artifact (as for #528), never from a VM build.

## 4. Out of scope

Co-editing, comments and version history on reports, images/tables in reports, report delivery
by schedule, templates for reports.

## Handoff

(The developer appends one entry per PR: commits, commands run with counts, packages added with
licences, and everything *not verified*.)

### Handoff, PR A (T1 to T7), developer (Opus, console design), 2026-11-15

**Commits** (branch `feat/bi-62-report-editor`, on `db69c34c4`), one task each:

| Task | Commit | What |
| --- | --- | --- |
| T1 | `fa351711e` | `reports.rs`: `Quote`, `Divider {}`, `Chart.width` (`ChartWidth`, default `text`, not serialised when `text` so old rows write back unchanged), `ReportRule::BadInlineLink` + sentence, inline-link check (linear; checks the console's link rule at *every* unescaped `[`, so it is a superset of what the console can draw); the route keeps `width` when it fills `board`; integration test for the new kinds over HTTP and the updated unknown-kind sentence; tool schema (`report_blocks_schema`), prompt hint, fixture hand-edited; AI-15 case `report_with_marks_a_quote_and_a_wide_chart`; CHANGELOG |
| T2 | `0d231af05` | contract (`ReportChartWidth`, quote, divider, width); `src/lib/report-inline.ts` (parse, serialise, escape, Tiptap nodes both ways, links only through `safeReportUrl`); `blocksForSave` keeps a wide width |
| T3 | `6c665dc57` | `report-blocks.tsx` one renderer (marks as React nodes, quote, divider, chart width box, D look of the card); `report-document.css` shared by editor and reader; reader page as the document; print view test reads charts through the width box |
| T4 | `9bfc5fdb2` | the editor (Tiptap 3): `report-editor-page.tsx`, `-extensions.ts`, `-chart.tsx`, `-menus.tsx`, `-ui.tsx`, `report-charts.tsx` (catalogue + lazy board payloads), `report-chart-picker.tsx` (D picker), `src/lib/report-doc.ts` (blocks ↔ document); `listCharts` service call; old block-edit helpers in `report-blocks.ts` removed with their tests; packages added |
| T5 | `b0216e963` | `src/lib/report-autosave.ts` state machine and its wiring (debounce 1 s, flush on blur, unmount, pagehide, hidden tab; beforeunload confirm when unsaved; create on first non-empty change, `history.replaceState` to the edit URL; refusal sentence beside the status) |
| T6 | `0febc8c15` | Charts tab: `report-chart-shelf.tsx` (`useSidePanelTabs`, portal into the panel, opens on Charts for an empty report on ≥1024 px, else remembered per browser), header toggle |
| T7 | this commit | CHANGELOG extended; this handoff |

**Mockup state ported.** The "Reference commit" line was still empty when T4 started, and the
mockups worktree had D uncommitted on top of `5b1cca202`. Ported from the worktree as read at
2026-11-15T06:29Z; sha256 prefixes: `editor.tsx c4fdef1c1aba`, `extensions.ts 048fe6fd5b69`,
`menus.tsx 87f0b0bd0ef8`, `chart.tsx 899751cd16f1`, `frame.tsx 6c0764b3d25d`,
`mockups.tsx e982c1fd3cea`, `report-editor.css cafc4e1d768f`, `context.ts be9c51423b01`. If the
final D commit differs, the diff against these is what is left to port.

**Packages** (`bun add --exact`, under the 3 GB scope): `@tiptap/core`, `@tiptap/pm`,
`@tiptap/react`, `@tiptap/starter-kit`, `@tiptap/extensions`, `@tiptap/extension-typography`,
`@tiptap/suggestion`, `@tiptap/extension-bubble-menu`, all `3.34.0`, MIT. The lockfile gained 48
packages (the eight above, the StarterKit's `@tiptap/extension-*`, `prosemirror-*` 1.x,
`orderedmap`, `rope-sequence`, `linkifyjs`, `fast-equals`, `@types/use-sync-external-store`);
each `package.json` was read: **48 of 48 MIT**. No Tiptap Pro, no drag-handle extension. The
lockfile was written by bun 1.4.0 (VM); `packageManager` says 1.3.12; only additions, no
existing line changed.

**Commands run, on the final commit, with counts:**

- `cd rust && cargo fmt --check` — clean. `bash scripts/ci/check_test_modules.sh` — "every test
  file under rust/crates/*/tests/ is compiled into a test binary".
- `flock … systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0 nice -n 15 bun test
  src/lib/report-blocks.test.ts src/lib/report-inline.test.ts src/lib/report-doc.test.ts
  src/lib/report-autosave.test.ts src/features/dashboards/report-editor-extensions.test.ts
  src/features/dashboards/report-blocks.test.tsx src/features/dashboards/report-pages.test.tsx
  src/features/dashboards/report-chart-shelf.test.tsx
  src/features/dashboards/report-render-view.test.tsx src/components/app-shell/side-panel.test.tsx
  src/services/clients/dashboards.test.ts src/features/copilot/capabilities.test.ts` — **142 pass,
  0 fail, 12 files** (one process, so the files' `mock.module`/global restores were exercised
  together).
- `… bun run typecheck` — clean.
- `npx eslint` on the 27 TS/TSX files changed since `db69c34c4` — clean.
- `(cd ops/ai_eval && python3 -m unittest -q test_ai_eval)` — 206 tests OK (pytest is not
  installed on the VM).
- The tool schema fixture: checked that Python's `json.dumps(indent=2, ensure_ascii=False)`
  reproduces the committed file byte for byte before editing it the same way, and that every
  description string of `report_blocks_schema` appears in the fixture.

**Not verified** (rule 7):

- **No Rust was compiled** (owner rule 2026-11-14): `cargo build/check/clippy/test` were not run,
  so the new Rust unit tests in `reports.rs` (7), the prompt test
  (`the_report_hint_names_the_editors_blocks_and_marks`), the API integration test
  (`the_editors_blocks_save_as_written_and_an_unsafe_inline_link_is_refused`), the updated
  unknown-kind sentence assertion, clippy pedantic on `ChartWidth::is_text` (an `allow` with a
  reason for `trivially_copy_pass_by_ref`) and the byte-identical fixture comparison
  (`tool_schemas_snapshot_is_byte_identical`) run first in CI on the draft PR.
- The full `bun run test`, `bun run lint`, `next build` and a dev server were not run (VM rules).
- **T7 screenshots** (light/dark at 1190×800, 1366×900, 390×844), the real look of reduced
  motion and focus rings, drag and drop with a real mouse (block handles, a chart from the
  Charts tab), the bubble menu's placement, the `/` menu's placement near the screen edges, the
  phone sheet and IME/Android typing: none seen in a browser; they need the QA stack. Reduced
  motion and focus were checked in code only (CSS animations sit under
  `prefers-reduced-motion: no-preference`; spinners and hover lifts carry `motion-reduce`).
- The AI-15 case was not run against a model.
- Saving on a closing tab is best effort (the request may be cut); unsaved text then makes the
  browser ask to confirm. Not tried in a browser.

**Known limits and choices to review:**

- A chart inserted in this sitting is drawn from its dashboard's payload (`/api/dashboard?board=`,
  with the board's saved filters), the reader then reads it through the report route; the two
  can differ if the dashboard has saved filters. The editor does not re-read the report after a
  save.
- A new report with no title is saved as "Untitled report" (the placeholder the page shows),
  because the server requires a title and D4 creates on the first non-empty change.
- The editor drops empty paragraphs (blank lines) on save, and flattens an indented list item into
  the same list (a report's list has one level), as `report-doc.ts` says.
- Download PDF in the editor shows only for a report that existed when the page opened (the
  flags come from its read); a report made in this sitting gets it on the next visit.
- The bold/italic rule is a whitespace-flanking star rule of our own (documented in
  `report-inline.ts`); CommonMark edge cases such as `**a**b**` read differently from a
  Markdown renderer. The server never needs it (it checks links only).
