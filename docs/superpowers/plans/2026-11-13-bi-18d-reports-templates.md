# BI-18 part D: reports with live charts, and dashboard templates

Planner: Claude Opus (hub session). Developer: a **separate Opus developer agent**, because this
is new console UI (the AGENTS.md exception for console design work). Base: `origin/main`
`6247400f` (2026-11-13). Feature page: `docs/core/features/reports-and-templates.md`; its
decisions R1–R5 and T1–T5 are signed by the owner, 2026-11-13. Do not re-ask or change them.

## 1. Anchors (find by symbol; verify at `6247400f`, and report a mismatch instead of improvising)

- **Dashboards API:**
  - `rust/crates/lakehouse-api/src/routes/dashboard.rs` (boards, tiles, the stored chart spec);
  - `dashboard_folders.rs` and `dashboard_access.rs` (folders and per-folder access, which
    reports reuse);
  - `dashboard_file.rs` (`BI-19` D's export and import of a board's definition, which is the
    natural base for a template's saved copy);
  - `dashboard_organise.rs` (trash, favourites, personal space).
- **The print path:** `pdf_export.rs` and `render.rs` (Gotenberg, the render token), and the
  console `render-view.tsx`.
- **Policy:** `POLICY_TABLE` and `tests/route_auth.rs`; every new route is asserted both ways.
- **Console:**
  - `src/features/dashboards/` (`dashboard-list-page.tsx` for the new tabs, `dashboard-grid.tsx`
    and `content-cards.tsx` for how a chart tile renders live, `folder-dialogs.tsx`,
    `dashboard-file.ts`);
  - the services under `src/services/` (contracts, then clients, then `services/index.ts`, then
    features).
- **Assistant:** the dashboard tools under `rust/crates/lakehouse-api/src/routes/ai/tools/`, the
  capability list `src/features/copilot/capabilities.ts`, `tests/fixtures/tool_schemas.json`,
  and the AI-15 standard request set.
- **Migrations:** the next free `NNNN`, checked against `origin/main` **and the open PRs** at the
  time you write it.

## 2. Tasks (one commit each, in order; Rust first)

1. **T1, store and API for reports (R1, R2, R4).**
   - A `report` table (folder, title, ordered blocks as JSON: `heading`, `paragraph`, `list`,
     `link`, `chart` with a chart reference), plus a migration with a why-header.
   - Routes for list, get, create, update and delete (into the trash, like boards), with
     `dashboard:read` and `dashboard:write`, and folder access applied the same way as boards.
   - Server-side checks:
     - at most 30 chart blocks, with a fixed sentence naming the limit;
     - a chart reference the caller cannot read renders as the board's not-available sentence,
       never data;
     - text is stored as plain structured text, not HTML, and rendered escaped.
2. **T2, the print path for reports (R3).** A report renders in the print view and downloads as
   a PDF through `pdf_export.rs`, the same two slots, timeout and render token as dashboards.
3. **T3, templates (T1, T2, T4).**
   - Save a board as a template: a copy of its definition, reusing `dashboard_file.rs`'s
     serialisation, with no data or snapshots.
   - List templates, start a board from one (an independent copy in a chosen folder), and delete
     a template.
   - `dashboard:write` on all writes.
   - A started board whose chart table is missing gets the existing "source not available" tile
     state.
4. **T4, assistant tools (R5, T5).**
   - `create_report`, `update_report` (add or replace blocks, add a chart by reference),
     `list_templates` and `create_dashboard_from_template` are low-risk writes behind
     `dashboard:write`. `delete_report` and `delete_template` need approval.
   - Gate and audit text, the prompt's dashboard section, `capabilities.ts`, and a regenerated
     `tool_schemas.json` with the tool count assertion updated.
   - Two AI-15 cases: write a report with one chart; start from a template.
5. **T5, console: Reports (R1–R4).**
   - A "Reports" tab on the Dashboards page.
   - The report editor and reader: a text editor of blocks (heading, paragraph, list, link) and
     a chart picker from existing dashboards; the chart blocks render live, the same way a
     dashboard tile does.
   - Folder placement, Download PDF, and Delete (to the trash) with a confirmation.
   - Empty, loading, error and refusal states are designed, not left as defaults.
   - The site's existing console design language: DESIGN.md does not apply to the console;
     follow the console's own components and theme, including dark mode.
6. **T6, console: Templates (T1, T2, T4).**
   - "Save as template" in the board actions menu.
   - A "Templates" tab listing them, with name, source board, created by and date; "Use
     template" asks for a name and a folder, then opens the new board.
   - Delete with a confirmation.
   - Hidden for people without `dashboard:write`, and the server refuses anyway.
7. **T7:** CHANGELOG, plus the feature page's status set to "Built" by the developer only in the
   handoff text (the planner updates the page and the backlog after merge).

## 3. Checks per task

- T1: API tests for create, read and order of blocks; the 30-chart refusal; folder access
  refusal; a masked and row-filtered role sees policy-applied data through a chart block; HTML in
  a paragraph is escaped.
- T2: a render test that the report prints with its blocks in order.
- T3: save, list, start and delete tests; the independent copy (editing the copy does not change
  the template); a missing table gives the honest tile state; 403 without `dashboard:write`.
- T4: tool tests plus the regenerated schema fixture; the two AI-15 cases are added and pass in
  the harness used today (or report how they were run).
- T5 and T6: component tests for the editor, reader, picker, tabs and dialogs; refusals rendered
  as the fixed sentences; the actions hidden for a read-only role.

## 4. Verification (once, at the end, in the foreground)

- **Full block from AGENTS.md:** Rust fmt, clippy `-D warnings` and `cargo test --workspace`,
  then TS typecheck, lint and test, then `git status`.
- **The shared machine is busy:** use `nice -n 15`, `CARGO_BUILD_JOBS=2`, and a shared
  `CARGO_TARGET_DIR` (`/home/hv/.cache/lakehouse-bi-target` or the one the other BI developer
  uses; don't create a per-worktree `target/`).
- **Browser check:** a QA stack is being brought up separately in `/home/hv/mtharreq/qa-stack/`.
  Don't start your own Docker stack. If you need a browser check, run `next dev` against that
  stack's API port, as `qa-stack/README.md` says, or report *not verified*.

## 5. Out of scope

- Scheduled report sends, report version history, report comments.
- Built-in templates and cross-deployment templates.
- Anything in `BI-17` or the other `BI-18` parts.

## 6. Handoff (developer appends)

### Handoff, PR slice 1 (developer, Opus, 2026-11-13)

Branch `feat/bi-18d-reports-templates`, base `origin/main` `6247400f`, plan commit `f4c3dfdf`.
Not pushed; no PR opened.

**Commits, in plan order** (all as `mtharreqq <72479829+mtharreqq@users.noreply.github.com>`):

| Commit | Task |
| --- | --- |
| `44ef28f8` | T1: reports: block rules, store, routes, trash, folder guard, sweep |
| `44962499` | T2: a report as a server-made PDF, and the console print view |
| `859cb0f3` | T3: dashboard templates |
| `340afe13` | T4: the six assistant tools, gate texts, prompt, fixture, AI-15 cases |
| `6f28075f` | T5: Reports tab, reader, editor and chart picker |
| `153f002c` | T6: Templates tab and Save as template |
| `a101bd07` | T7: CHANGELOG |
| `1d5f1d57` | T5 fix: `ROUTE_ACCESS` entries for the three report pages (found by the full `bun run test`) |

Several files carry hunks of more than one task (`routes/mod.rs`, `policy.rs`,
`lakehouse-store/src/lib.rs`, `tests/route_auth.rs`, `tests/dashboard_reports.rs`,
`dashboard-list-page.tsx`, `services/index.ts`). Each commit holds that task's version of
them. **The intermediate commits were not compiled one by one** (this machine took 25 to 90
minutes per Rust build today); every check below ran on the whole tree, and the full block
ran on the last commit.

**Migrations: `0142_dashboard_reports.sql` (T1) and `0143_dashboard_templates.sql` (T3).**
They are free because `origin/main` ends at `0141` (checked with `git fetch` and `git ls-tree
origin/main rust/migrations/` on 2026-11-13). No open PR adds a migration: the open PRs were
listed with
`gh pr list -R RantAI-dev/RantAI-Lakehouse --state open` (#516, #515, #491, #438, #431, #430,
#429, #427, #426, #425, #414, #379), and `git diff --name-only origin/main...<head> --
rust/migrations/` is empty for #516, #515 and #491. The rest are dependabot and site PRs. The
PR A worktree (`lakehouse-bi`, `fix/bi-export-safety`) also ends at `0141`. Both are Postgres
tables, as `dashboard_access_rule` (0103) and `dashboard_comment` (0114) are. Boards and
folders stay in ClickHouse; `folder_id` names a `bi_folder` row and has no foreign key, which
the 0142 header explains.

**New routes** (all in `POLICY_TABLE`, both ways in `tests/route_auth.rs`):

| Route | Policy | In the handler |
| --- | --- | --- |
| `GET /api/dashboard/reports` | `RequiresAuth` | `viewer_for`, folder visibility |
| `POST /api/dashboard/reports` | `dashboard:write` | `edit` on the folder |
| `GET /api/dashboard/reports/{id}` | `RequiresAuth` | folder visibility; each chart read through `payload_for` |
| `PUT /api/dashboard/reports/{id}` | `dashboard:write` | `edit`; `manage` where it leaves when moved |
| `DELETE /api/dashboard/reports/{id}` | `dashboard:write` | `manage` on the folder; to the trash |
| `GET /api/dashboard/reports/{id}/pdf` | `RequiresAuth` | as the dashboard PDF; audited `dashboard.report_pdf` |
| `GET`/`POST /api/dashboard/templates` | `dashboard:write` | save needs the board visible |
| `POST /api/dashboard/templates/{id}/use` | `dashboard:write` | `edit` on the target folder |
| `DELETE /api/dashboard/templates/{id}` | `dashboard:write` | |

Changed routes:
- `GET /api/dashboard/trash` lists reports (`kind: "report"`), and `POST
  /api/dashboard/trash/restore` takes `kind: "report"` (`manage` on its folder, or `edit` at the
  root when the folder is gone).
- `DELETE /api/dashboard/folders` refuses a folder that holds a report, live or trashed (409).
- `POST /api/render/data` answers a report token (new `RenderClaims.report`).
- Deadlines: a report read is a board path, the PDF gets the PDF timeout, and starting from a
  template gets the export timeout.
- The hourly trash sweep purges reports after 30 days.

**New assistant tools** (registry 115 → 121, `tests/fixtures/tool_schemas.json` regenerated with
the ignored `write_tool_schema_fixture` test):
- `create_report`, `update_report` (`blocks` replaces them, `append` adds blocks at the end,
  `chart` adds one chart at the end) and `create_dashboard_from_template`: `WriteLow`;
- `list_templates`: `Read`;
- `delete_report` and `delete_template`: `WriteHigh`.

All six declare `dashboard:write` and call the routes' own functions. Also changed: gate
summaries, approval reasons, the prompt bullet and its approvals list, a word domain, the
dashboard capability in `capabilities.ts`, and the step labels. AI-15: `report_with_one_chart`
and `dashboard_from_template` were added to `dashboard_cases.json`, and the harness learned the
`report` and `template` kinds.

**Console:**
- `/dashboards/browse?tab=dashboards|reports|templates`; the Templates tab only shows with
  `dashboard:write`.
- `/dashboards/reports/<id>`, `/dashboards/reports/<id>/edit` and `/dashboards/reports/new`.
- `/render/report/<token>`.
- "Save as template…" in a dashboard's ⋯ menu.
- The trash dialog shows reports.

**Commands run, with their counts:**

Per-task scoped checks, on the whole tree before committing. Every cargo command ran through
`flock /home/hv/.cache/lakehouse-cargo.lock nice -n 15 env CARGO_BUILD_JOBS=<1|2> CARGO_TARGET_DIR=<dir> cargo …`.
- `cargo test -p lakehouse-bi --lib reports::`: 9 passed.
- `cargo clippy -p lakehouse-bi -p lakehouse-store -p lakehouse-embed -p lakehouse-api --all-targets -- -D warnings`: clean, after fixing 3 findings plus 2 `assert_is_empty` findings in the new tests.
- `cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`: 1 passed (the fixture now has 121 tools).
- `cargo test -p lakehouse-api --lib`: 3648 passed, 18 ignored.
- `cargo test -p lakehouse-api --test ai_written_tools`: 6. `--test dashboard_import`: 9. `--test dashboard_reports`: 13. `--test dashboard_templates`: 6, after fixing three test-side mistakes, none in product code. `--test pdf_export`: 9. `--test render_data`: 13. `--test route_auth`: 98.
- `cargo test -p lakehouse-bi -p lakehouse-embed -p lakehouse-store --lib`: 656, 23 and 322 passed.
- `bun test` on the new files: `report-blocks.test.ts` 6, `report-render-view.test.tsx` 2, `report-pages.test.tsx` 11, `template-pages.test.tsx` 10, and `capabilities` with the two lib tests 36. All pass.
- `python3 -m unittest ops/ai_eval/test_ai_eval.py`: 206 OK.

Full block on `1d5f1d57` (the Rust code is identical to `a101bd07`, where the Rust block began):
- `cargo fmt --check`: OK. `cargo clippy --workspace --all-targets --all-features -- -D warnings`: OK (on `a101bd07`; the Rust code is identical on `1d5f1d57`).
- `cargo test --workspace`: **not verified at handoff time.** The single run died when the disk filled (see Machine). The batched run (`scratchpad/workspace_tests.sh`, same targets, PID 1136608) was still running when this handoff was written; only `lakehouse-alerts` had finished (0 failures). The planner's own run before merge is the one to trust.
- `bun run typecheck`: OK. `bun run lint`: 0 errors, 11 warnings, all in files this slice does not touch. `bun run test`: 4395 pass, 0 fail, across 401 files (one failure in the first run, ADM-13's page-access drift test, fixed in `1d5f1d57`).
- `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: OK.
- `git status`: clean.

**Not verified:**
- **A browser check.** The QA stack (`/home/hv/mtharreq/qa-stack`) runs `origin/main`'s API,
  which has none of the new routes. Rebuilding its API takes about 1.5 h on someone else's
  stack, and the brief said not to start my own. No screenshots.
- **The two AI-15 cases.** They need a model key and a throwaway stack. Only their shape was
  checked: the harness's own tests pass, and the cases parse.
- **A dropped table read end to end.** `BI-18D-AC7` (a chart whose table was dropped shows
  "source not available" on a dashboard started from a template) is not checked as a rendered
  tile. The test proves the chart is copied as it was and never run before it is saved. The
  sentence then comes from the existing tile path (`run_spec_sql`, the tenant gate's
  "Table not found or not available…" or the engine error under a reference). That path is not
  exercised against a real missing table here.
- **The real Gotenberg.** The report PDF was tested only against a `wiremock` Gotenberg (the
  address it opens, the token and the audit). The real print of `/render/report/<token>` was not
  run: the `pdf_real_gotenberg` test was not run for reports.
- **The intermediate commits, compiled one by one** (see above).
- **Dagster pytest and compose.** Not run, because this slice touches neither.
- **gitleaks.** Not run: it is not installed on this machine.

**For the planner:**
- **Status.** The feature page's status may be set to "Built"; I left the page unchanged, as
  the plan says.
- **Plan reading 1.** The plan lists `list_templates` among "low-risk writes"; I made it a
  `Read` with the `dashboard:write` permission, because a listing that needs a Confirm click
  would be odd. Please confirm.
- **Plan reading 2.** The plan has no `list_reports` tool, so for "add this chart to the
  report" the assistant needs the report's id from `create_report` or the page context. The
  report page does not send a context today.
- **Plan reading 3.** "The board's not-available sentence" became two sentences: "This chart
  is not available." for a chart the reader cannot see, and "report not found." for a report
  they cannot read (the same shape as "dashboard not found.").
- **Limits I added.** Beyond the signed limit of 30 charts: 300 blocks, a title of 200
  characters, 10,000 characters of text per block, 100 list items and a 2,000-character URL.
  The list endpoints read at most 1,000 reports and 500 templates and say `truncated`.
- **Template visibility.** Templates are deployment-wide (T4). Saving one needs only that the
  dashboard is visible to the person, so a template made from a restricted dashboard shares its
  definition with every `dashboard:write` holder. It shares no data, which each reader reads
  with their own policy. Saving a template does not follow the download switch, which the
  dashboard file does. Decide whether that is wanted.
- **Folder delete.** A report blocks the delete of its folder (409), as a trashed dashboard
  does.
- **Machine.**
  - The shared `CARGO_TARGET_DIR` mixes artifacts across worktrees. Both worktrees build the
    same workspace crates into it, and cargo judged the other worktree's `lakehouse-bi` rlib
    fresh from its dep-info paths. The result was "unresolved import `lakehouse_bi::reports`"
    in a build of this branch.
  - I built in `/home/hv/.cache/lakehouse-bi18d-target`, seeded from a copy of the shared dir
    with the workspace crates' fingerprints removed, still under the lock.
  - A single `cargo test --workspace` links about 170 debug test binaries and filled the disk
    to 100%, and the linker died with a bus error. I deleted my own test executables and the
    incremental cache at once; disk went back to 85 GiB free. The full test then ran in
    batches (lakehouse-api and lakehouse-store ten test targets at a time, each batch under
    the lock), deleting the executables after each batch.
  - One `lakehouse-api` lib-test compile was OOM-killed at `CARGO_BUILD_JOBS=2`, so the later
    runs use 1.
  - Four `bun` processes owned by `hv` have used a full core each for 5 days. They are not
    mine and I left them alone.

## 7. Review (planner appends)

### Review, round 1 (planner, Opus, 2026-11-14)

Read: the route policy rows, both migrations, `dashboard_reports.rs` (list, read, chart reads),
`render.rs` (the report token), `pdf_export::export_report_pdf`, `dashboard_templates.rs` (save)
and `dashboard_file.rs` for comparison. The report read is right: chart blocks are read as the
reader through `payload_for`, a chart the reader cannot see is "not available" without a query, and
the report token names one report and reads it as the person now. The console was not reviewed in
this round; it goes to the owner's QA on a preview stack.

- **BLOCKER 1, saving a template skips the download switch (answers open question 5).** A template
  holds what the dashboard file of `BI-19` part D holds, and that export follows the per-role
  download switch (owner decision Q-D1, `dashboard_file.rs`, after the visibility check). Saving a
  template asks only for `View` on the board, so a role with downloads off can copy the same content
  out through a template. Fix: saving a template runs the same checks as the dashboard file export,
  in the same order (the board visible, else the same 404; then `require_download`), and also needs
  `Edit` on the source dashboard, because the template is then listed to every `dashboard:write`
  holder, including people who cannot see that dashboard. Tests: a role with downloads off gets the
  download refusal; a person with `View` but not `Edit` on the board gets 403; a hidden board is the
  same 404 for both.
- **SHOULD-FIX 2, the assistant cannot find a report to edit (answers open question 2).** R5 says
  the assistant edits reports, and `update_report` needs an id the chat has no way to learn. Add a
  read tool `list_reports` (the reports the person can read: id, title, folder, updated), and have
  the report page send its report id and title as page context, the way the dashboard page sends
  its board. Regenerate `tool_schemas.json`.
- **Accepted as built:**
  - question 1, `list_templates` as a read that still needs `dashboard:write`;
  - question 3, "report not found." matches the board's not-found sentence (`BI-18D-AC3`);
  - question 4, the extra input limits and list caps. The planner adds them to the feature page's
    limits.
- **Verification:** the reviewer re-runs the full block on the final commit before the PR.
