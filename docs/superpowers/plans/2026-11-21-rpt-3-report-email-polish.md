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

## Handoff

Commits: d4c971ccb (constant, sign-off, test, CHANGELOG), a686d1a57 (print
root white), plus this handoff.

Commands: `rustfmt --edition 2024 --check` on report_runner.rs, settings.rs
and lakehouse-notify/src/lib.rs: clean. No cargo build/check/clippy/test, no
eslint (CSS-only console change).

"Rantai Lake" strings a person can read, found in `rust/crates/*/src`:
1. `report_runner.rs` report email sign-off: fixed (uses `settings::PRODUCT_NAME`).
2. `lakehouse-notify/src/lib.rs` alert email footer "Rantai Lake — Enterprise
   Lakehouse Console": fixed to "RantAI Lakehouse" (literal; the notify crate
   cannot reach the api crate's constant). No test pinned it.
Not changed: `0018_seed_agents.sql` (applied migration), code comments, and
docs. There is no plain-text email part; the emails are HTML only, so the
"text part" has nothing to fix. The test email already said "RantAI Lakehouse";
its body now uses the constant (same text).

Why the root was dark: the theme provider puts the `dark` class on `<html>`
(`color-scheme: dark`, dark tokens). The `@page` margin is painted with the
root/canvas background, while the print rule whitened only `body`. This is
inferred from the code, not observed in a PDF.

Not verified: Rust is not compiled (CI does it): `{PRODUCT_NAME}` is an inline
format capture of a `pub(crate)` const from `settings` (declared there, used
in report_runner via full path); the new test assertions in
`a_body_without_a_link_or_filters_says_so_and_has_no_anchor`. The PDF margin
fix is not checked on a real Gotenberg render. No console test added.

## Handoff 2

PR #641 review SHOULD-FIX 1: a real Gotenberg render showed the margin is
painted with the dark colour scheme's canvas colour (#121212), not the root's
background. In `@media print`, `html` now also has
`color-scheme: light !important` (white backgrounds kept) and the comment says
why. Screen styles are unchanged. Not re-rendered by me; the CSS is not
compiled or tested on this VM.

## Handoff 3

PR #641 review SHOULD-FIX 2: `src/components/app-shell/console-theme-provider.tsx`
wraps the design system's `ThemeProvider` and passes `forcedTheme="light"` when
`usePathname()` is `/render` or under `/render/`; elsewhere nothing is forced.
`src/app/layout.tsx` uses it in place of the bare provider. `design-system/` and
the print CSS are unchanged.

Checks (bun via flock + systemd-run, temporary node_modules symlink removed):
`bun test src/components/app-shell/console-theme-provider.test.tsx` 3 pass;
`bun x tsc --noEmit -p .` no output; `bun x eslint` on the three changed files
no output. Test: light forced on `/render/report/x`, not on `/dashboards`, and
the path matcher.

Race check: the render views (`render-view.tsx`) mark ready (`markReady`) only
after their data has loaded and every tile drew or failed; nothing in them reads
or waits on the theme (they draw light by their own `dashboardLook`). next-themes
gets `forcedTheme` on first render (the pathname is known on the server), so the
class and `color-scheme` are set at mount, before the data fetch completes. Not
confirmed by a real render.

## Review (planner, 2026-11-21)

Checked on the QA stack (integration with this branch, Gotenberg 8.37, a
mail catcher), by reading the PDF's first page fill and rendering the page
with LibreOffice:

- The sign-off reads "RantAI Lakehouse, scheduled report". Done.
- `a686d1a5` alone (`html, body` white) left the full-page `#121212` fill:
  the margin takes the colour scheme's canvas colour.
- `8ba7da03` (`color-scheme: light` on `html` in print) **fixes it**: the
  page's first fill is white and the render is white to the edge.
- `f04d331a` (light theme forced on `/render/*`) was asked for on a misread
  check (the planner's script kept re-reading the first, pre-fix email). It
  is not what fixed the band; it stays because the print views are paper and
  light is right for them whatever the console's default theme, and it is
  covered by its test. No open finding.
