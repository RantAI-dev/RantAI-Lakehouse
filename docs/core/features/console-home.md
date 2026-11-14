# Console Home

| | |
| --- | --- |
| Backlog | `WEB-17` |
| Spec | [`specs/web-17.md`](../specs/web-17.md) |
| Plan | `docs/superpowers/plans/2026-11-14-web-17-home-revamp.md` |
| Route | `/` |

## What the user can do

1. Ask or build with the AI Copilot from the composer at the top; sending opens the conversation.
2. Start from a suggestion chip.
3. See one chosen dashboard live, and open it.
4. Reopen a recent AI chat, or go to all of them.
5. See each pipeline's last status; ask the AI Copilot why a failed one failed.
6. See what needs attention: open alerts, unhealthy sources, stale assets.
7. Choose the background: Dither, ASCII or Off, from Customize.

## Decisions

| ID | Decision | Value | Signed |
| --- | --- | --- | --- |
| D1 | Layout | Mockup B: composer over the lake scene, bento below | Owner, 2026-11-14 |
| D2 | Background choice | Customize menu: Dither (default), ASCII, Off; remembered per browser. Off keeps the layout; only the art is hidden, the hero does not move (changed 2026-11-14, owner QA: "kalo pas off jangan naik ke atas gini composernya, tetap aja kek gitu tapi kosong aja backgroundnya") | Owner, 2026-11-14 |
| D3 | Widgets without an API today | Left out (activity heatmap, month totals, Gold rows this month, "opened" times), not faked | Planner |
| D4 | History page | On hold; the owner has a new idea | Owner, 2026-11-14 |

## Limits

- The background choice lives in this browser (`localStorage`), like the Dashboards table/gallery choice; it does not follow the account to another device.
- Chat times are "last updated", the only time the sessions API returns.

## Acceptance checklist

| ID | Where | Check | Result |
| --- | --- | --- | --- |
| `WEB-17-AC1` | `/`, 1366×800, light and dark | Composer, chips and the focal dashboard's first tile row are visible without scrolling | Not run |
| `WEB-17-AC2` | Composer | Ask and Build both start a conversation as before | Not run |
| `WEB-17-AC3` | Customize → Background → Off | The art fades out; the greeting, composer and chips stay exactly where they were; switching back fades it in without a jump; the choice survives a reload | Not run |
| `WEB-17-AC4` | A failed pipeline | "Ask why" fills the composer with a question about that pipeline | Not run |
| `WEB-17-AC5` | Reduced motion on | The scene is still; nothing animates | Not run |
| `WEB-17-AC6` | iPhone 13, Safari | No horizontal scroll; composer usable; scene does not stutter | Not run |
| `WEB-17-AC7` | Any widget | No sample or invented numbers | Not run |
