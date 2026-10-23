# Dashboards open fast: remembered results and tiles that load together

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-4`, with a new assistant item (next free `AI-` number when it is added to `BACKLOG.md`; `AI-22` is the highest today) |
| Status | Draft (2026-10-24). Decision 1 waits for the owner's signature (questions-for-owner Q1); the rest are planner defaults the owner confirms at the phase 5 QA |
| Plan | `docs/superpowers/plans/2026-10-24-bi-4-caching-parallel-tiles.md` (two parts: A parallel tiles, B remembered results) |
| Spec | `docs/core/specs/bi-4.md` |

## Problem

Opening a dashboard runs every chart's query again, one after another, inside
one request. A 12-tile dashboard waits for the sum of its twelve queries, and
a dashboard of slow tiles can hit the request's 60-second limit even when each
tile alone is quick. Opening the same dashboard a minute later does all the
work again.

## What the user can do when this is done

1. Open a dashboard and see its tiles computed side by side (up to 6 at once), so it takes about as long as its slowest tile, not the sum of all of them.
2. Open it again and get remembered results at once; each tile says when its data was computed ("Data from 10:42") and whether it was remembered.
3. Press Refresh on the dashboard (or on one tile) to compute it fresh, whatever the remembered results.
4. As an administrator, in Admin > Settings, choose how long results may be remembered for the whole install, or switch remembering off.
5. As a dashboard editor, set a different rule for one dashboard or one chart (for example "always fresh" for a live-operations chart).
6. Ask the assistant: "How fresh is this dashboard?" or "Make this chart always fresh".

## Not included

- Remembered results shared between API processes (each process keeps its own; a deployment with two API processes may compute a tile twice).
- Pre-computing dashboards on a schedule before anyone opens them (Metabase "schedule" caching). A later item if wanted.
- Query Studio and the assistant's free SQL (`run_sql`): they stay fresh every time.
- Tiles that paint one by one as they arrive (the page still waits for the last tile of the response; see decision 3).

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | **Freshness rule.** A remembered result is used until the data under the chart changes (checked on every open, from the engine's own record of when each table last changed), and never longer than 24 hours. For data whose changes the engine cannot report (a SQL source over an Iceberg table, a view), results are remembered for 10 minutes. | **Owner to sign before building** (Q1) |
| 2 | **Where the rule is set.** Install (Admin > Settings, `settings:write`), then dashboard, then chart (both `dashboard:write`): the narrowest one set wins. The choices are "Follow the default", "Always fresh", and "Remember for N minutes" (1 minute to 24 hours). | Planner default — owner to confirm at QA |
| 3 | **Parallel tiles.** One dashboard request computes up to 6 tiles at once; each API process computes at most 24 tiles at once across every request, the rest wait their turn. Tiles still arrive in one response. | Planner default — owner to confirm at QA |
| 4 | **Never across permissions.** A remembered result is found by the exact statement that ran after the person's masks and row filters were applied (the statement already names the masked columns and the person's own row-filter values), plus the engine limits it ran with. Two people whose rules differ run different statements, so they can never share a result. Two people with the same rules may share one, which shows them nothing they could not see. | Planner default — owner to confirm at QA |
| 5 | **Shown on every tile.** "Data from HH:MM" (in the report time zone) in the tile's footer and the tile's data dialog; "remembered" when it came from memory. A tile that failed is never remembered. | Planner default — owner to confirm at QA |
| 6 | **Refresh.** The dashboard's Refresh and a tile's "Refresh this tile" compute fresh for that request and replace what is remembered. Anyone who can view the dashboard may refresh it. Auto-refresh (`BI-18` B) follows the rule and does not force fresh results. | Planner default — owner to confirm at QA |
| 7 | **Memory.** At most 256 MB of remembered results per API process (an operator can change it with `RESULT_CACHE_MAX_MB`); the least used go first. A single result over 8 MB is not remembered. | Planner default — owner to confirm at QA |
| 8 | **Embeds, public links, PDFs and scheduled reports** read through the same memory, by the same rule (decision 4 keeps the embed viewer's masks apart). | Planner default — owner to confirm at QA |
| 9 | **Measured.** A RESULT document (`docs/plans/BI-4-RESULT.md`) records a 12-tile dashboard's open time before, with parallel tiles, and on remembered results, with the machine and the data size. The 3-second target is reported as met or not met, never adjusted. | Planner default — owner to confirm at QA |

## Limits to tell a customer

- Each API process remembers its own results, up to 256 MB.
- A table changed outside the engine's own record (for example an Iceberg table read through a SQL source) can show results up to 10 minutes old; the tile says how old.
- Parallel tiles are bounded: a very large dashboard still queues behind 6 at a time.

## Asking the assistant

The assistant's tools that read chart data (the chart previews and
`query_metric`) go through the same memory and return `computedAt` and
`remembered`, so "How fresh is this dashboard?" is answered from the payload.
A new tool `set_cache_rule { board?, chart?, rule: default|fresh|minutes, minutes? }`
changes a dashboard's or chart's rule; it writes, so it asks for approval like
the other dashboard writes. The dashboard page's context gains each tile's
`computedAt`. The standard request set (`AI-15`, `ops/ai_eval/dashboard_cases.json`)
gains "Make this chart always fresh". The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-4-AC1` A second open of a dashboard is served from remembered results and its tiles say so, with the time the data was computed.
- `BI-4-AC2` Two users with different masks on one column open the same dashboard one after the other; each sees only their own masked values.
- `BI-4-AC3` `docs/plans/BI-4-RESULT.md` records the before and after timings of a 12-tile dashboard.
- `BI-4-AC4` A user without `settings:write` cannot change the install's rule, a viewer without `dashboard:write` cannot change a dashboard's, and a failure shows a plain sentence.
- `BI-4-AC5` *(proposed)* After a Gold build changes a table, the next open of a chart on it shows the new data without pressing Refresh.
- `AI-x-AC1` "Make this chart always fresh" asks for approval and then sets the rule.
