# `RPT-3`: the report email's name and a PDF with no dark band

Planner and reviewer: Claude Opus. Developer: Claude Sonnet. Base: `main`.
Branch: `fix/rpt-3-report-email-polish`.

## Why

The planner checked `RPT-1` on the QA stack on 2026-11-21 with a local mail
catcher (owner: "aku percayakan ke kamu"). Every acceptance check passed:
test email, Send now with the PDF, a filtered subscription that shows only
its filter, skip-when-empty (skipped and recorded), an outside domain that
is refused, and a scheduled send. A recipient still sees two faults:

1. The email ends with **"Rantai Lake, scheduled report"**
   (`rust/crates/lakehouse-api/src/routes/report_runner.rs:1304`, HTML; check
   the plain-text part too). The product is "RantAI Lakehouse" (owner,
   2026-11-18, #590).
2. Every PDF has a **dark band** around the white page: the page number
   "1/1" is printed inside it, so it is the page margin
   (`@page { margin: 10mm; }`, `src/app/globals.css:172`), painted with the
   root background. `@media print` sets only `body` to white; Gotenberg
   prints with `printBackground=true` (`rust/crates/lakehouse-api/src/pdf.rs:274`).

## Decisions (planner, 2026-11-21)

1. One product-name constant in `lakehouse-api` (grep for an existing one
   first, rule 4), used by the report email's sign-off and by the test email
   (`routes/settings.rs:825`, `:1439`), so the name lives in one place. The
   sign-off reads "RantAI Lakehouse, scheduled report" in both the HTML and
   the text part. Sweep `rust/crates/*/src` for any other "Rantai Lake" a
   person can read (emails, PDFs, notifications) and fix those too; list
   them in the handoff. Applied migrations are not edited
   (`0018_seed_agents.sql` keeps its seed rows; out of scope).
2. Printed pages are white to the edge: in `@media print`, the root
   (`html`) is white as well as `body`, so the margin area is white for
   every PDF (dashboard, chart, report). Find why the root is dark in the
   print view (theme class or variable) and say it in the commit; do not
   change the on-screen theme.
3. Tests: the report email test(s) assert the new sign-off (update any that
   pinned the old one, saying why); a console test that the print rule
   whitens `html` is optional if the CSS is not testable here.

## Checks (this VM)

No cargo build, check, clippy or test on the VM (they run out of memory):
`cargo fmt --check` on the changed crates only, and CI compiles and tests.
Console: `bun x tsc --noEmit -p .` is not needed for a CSS-only change; run
`bun x eslint` on any changed TS file. Wrap bun commands in
`flock /tmp/claude-1000/lh-bun.lock systemd-run --user --scope -q -p
MemoryMax=3G -p MemorySwapMax=0 nice -n 15 …`. CHANGELOG entry.
