# `WEB-17` Home: one composer over the lake, the day's work below it

| | |
| --- | --- |
| Backlog | `WEB-17` in [BACKLOG.md](../BACKLOG.md) |
| Module | Platform |
| Size | M, one PR (frontend only) |
| Spec checked | Against the code at `8d401544` (`feat/bi-61-dashboard-page`) on 2026-11-14 |

## Why

Home today is a centred greeting and composer with large empty bands above and below. A status pill floats beside the greeting, and each of the three create buttons carries a sparkles button, an icon on the owner's "AI slop" list. The useful content (a dashboard's tiles and Recent) sits below the fold under an uppercase label.

On 2026-11-14 the owner compared three mockups (`design/chart-builder-mockups` at `a7779918`, `/mockups/home/a` to `/c`) and chose **B**: the composer over a large dithered lake scene that is ours (night sky, moon, the whale head from the logo at the waterline), fading into the page where the widgets live. The owner asked for a way to turn the scene off: "tetap perlu ada opsi untuk matiin backgroundnya biar ga terlalu distracting".

## What users get

- A greeting and the AI Copilot composer (Ask / Build) over the lake scene. A one-line context strip under the composer says what Ask can read, using the real count of marts this person can read.
- Suggestion chips under the composer, from the existing `homePrompts()`.
- Directly below, in one laptop screen, a compact bento:
  - **Focal:** the chosen dashboard, drawn live with the real tile renderer.
  - **Recent AI chats:** the latest four, with "N more" to the history page.
  - **Pipelines:** each pipeline's last status and time; a failed one offers "Ask why", which fills the composer.
  - **Needs attention:** open alerts, unhealthy sources and stale assets, the checks behind today's status pill.
- A **Background** choice in Home's Customize menu: Dither (default), ASCII, or Off. It is remembered in this browser. Off hides the art and keeps the layout: nothing moves (owner QA 2026-11-14).
- Light and dark themes; a still scene under reduced motion.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Above the fold at 1366×800 | Greeting and composer only | Composer, chips and the top of the bento; the focal dashboard's first row of tiles is visible without scrolling |
| Scene cost | None | Drawn once to a canvas or static image after first paint; never blocks the composer. Optional motion is cheap (no per-frame full redraw above ~10 fps), paused off-screen and in a hidden tab, and none under `prefers-reduced-motion` |
| Background off | n/a | One click in Customize; the art fades out in place and nothing moves (owner QA 2026-11-14) |
| Data honesty | n/a | Every number comes from an existing API; no widget shows sample data |
| AI icons | Sparkles | `Bot`, as elsewhere since `BI-61` |

## Not in this item

- **Activity heatmap and "this month vs last" totals** (mockup A/B). No API gives per-day counts: query history is clamped to 14 days and pages of 50, pipeline runs need one request per pipeline with no date filter, and chats carry only `updatedAt`. This needs a server aggregate; it is a separate backlog item.
- **"Gold rows landed this month."** No API gives rows landed per period.
- **"Opened N h ago"** for dashboards: the recents API records opens but returns no time.
- The AI chats (history) page: the owner has a new idea; on hold.
