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

## 7. Review (planner appends)
