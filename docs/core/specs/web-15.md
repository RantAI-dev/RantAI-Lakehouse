# `WEB-15` The sign-in page shows the product

| | |
| --- | --- |
| Backlog | `WEB-15` in [BACKLOG.md](../BACKLOG.md) |
| Module | Platform |
| Size | S, one PR |
| Spec checked | Against the code at `40c876d9` on 2026-11-14 |

## Why

The sign-in page is a plain form with the logo far from it. It is the first screen anyone sees, and it says nothing about the product. The owner reviewed four mockups on 2026-11-14 (`design/chart-builder-mockups` at `5e7b9b8c`, `/mockups/login/a` to `/d`) and chose **A**: a live sample dashboard on the left, the form on the right.

## What users get

- The left half shows a small dashboard drawn with the console's real tile renderer: a KPI that counts up, a line that draws, bars that grow, and charts that answer the pointer. It sits on a solid navy panel with our own lake-contour lines that ripple slowly. No "Sample data" badge (owner, 2026-11-14); the panel's accessible name still says it is sample data.
- The right half is the form: the logo top-left, the title "Sign in to Lakehouse" (Inter Tight, medium, centred), visible labels with placeholders, a navy Sign in button, a password eye, a Caps Lock warning, a designed error and a loading state.
- Light and dark themes. On a phone, one live tile sits above the form.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| First paint of the form | Form only | The form renders and is usable before the sample dashboard loads; the dashboard and its chart code load after it, never blocking typing |
| Motion | None | A first-load reveal under 900 ms; then every 9 s, only while visible, the data changes in place (bars and line morph, the KPI counts) and the card never disappears (owner, 2026-11-14); none under `prefers-reduced-motion` (a still dashboard) |
| Wrong password | Inline text | The message opens in place, the form shakes once (240 ms), the password is selected for retyping |
| Sign-in behaviour | `login-page.tsx` | Unchanged: the same requests, redirects, `next` handling, SSO flag and notices |
