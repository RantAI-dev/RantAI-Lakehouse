# Time and row limits an administrator sets, and a Stop that stops

| | |
| --- | --- |
| Module | Dashboards (and Query Studio, which shares the limits and has the only Stop button today) |
| Backlog | `BI-5`, with a new assistant item (next free `AI-` number when it is added to `BACKLOG.md`) |
| Status | Draft (2026-10-24). Decision 1 waits for the owner's signature (questions-for-owner Q2); the rest are planner defaults the owner confirms at the phase 5 QA |
| Plan | `docs/superpowers/plans/2026-10-24-bi-5-limits-and-stop.md` |
| Spec | `docs/core/specs/bi-5.md` |

## Problem

The limits are written into the code: a chart over a SQL source stops after
30 seconds and 2,000 rows, a chart over a Gold table has no engine limit at
all and is cut by the API's 60-second request limit, a point map shows at
most 5,000 points, and Query Studio returns 2,000 rows. When the API gives up
on a chart over a Gold table, the engine keeps running the query. Query
Studio's Stop button only stops the browser waiting; the query runs on.

## What the user can do when this is done

1. As an administrator, in Admin > Settings > Query limits, set how long a chart or a Query Studio query may run (10 seconds to 30 minutes) for the whole install, and a longer or shorter time for chosen roles.
2. As an administrator, set the row limits: rows a chart reads, points a map shows, rows Query Studio returns.
3. See a tile that ran out of time say so: "This chart stopped after 30 seconds, the limit set for your role."
4. Press Stop in Query Studio, or Stop on a dashboard that is still loading, and have the query end on the engine within 5 seconds.
5. Ask the assistant: "What are the query limits?" or "Stop the running query".

## Not included

- Stopping a query on Trino from the console (Query Studio's Trino engine): Stop there still only stops the browser, and the button says so.
- Limits per dashboard or per chart (the install and roles only).
- Tables: a raw table already pages through its whole result (`BI-16` A); its page size is not a limit here.

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | **Time limit.** The administrator chooses 10 seconds to 30 minutes. Default 60 seconds for every chart and Query Studio query (charts over a SQL source go from 30 to 60 seconds). A person with several roles gets the longest limit among their roles; a role without its own uses the install's. | **Owner to sign before building** (Q2) |
| 2 | **Row limits.** Rows a chart reads: default 2,000, range 100 to 100,000 (today 2,000 over SQL sources). Points on a map: default 5,000, range 100 to 50,000. Query Studio rows: default 2,000, range 100 to 100,000. Per install only. | Planner default — owner to confirm at QA |
| 3 | **The engine enforces it.** The limit travels with each query to the engine, which stops the query itself; the API waits 10 seconds longer, so the person gets the engine's clear answer, not a bare "request timeout". A query the API gives up on (the person closed the page) is cancelled on the engine too. | Planner default — owner to confirm at QA |
| 4 | **Stop.** Query Studio's Stop and a new Stop on a loading dashboard cancel the person's own running queries on the engine; within 5 seconds the engine no longer lists them. A person can stop only their own queries; stopping someone else's stays an administrator action in Operations. | Planner default — owner to confirm at QA |
| 5 | **Messages.** A tile that hit the time limit says "This chart stopped after N seconds, the limit for your role" (or "for this install"); a tile cut at the row limit says "Showing the first N rows, the limit set by your administrator". A stopped tile says "Stopped". | Planner default — owner to confirm at QA |
| 6 | **Who changes them.** `settings:write` (the same as the other Admin > Settings sections). Anyone signed in can read the limits that apply to them. | Planner default — owner to confirm at QA |
| 7 | **Exports keep their own limits** (`BI-1`: 1,000,000 rows, 280 seconds) and are not changed by these settings. | Planner default — owner to confirm at QA |

## Limits to tell a customer

- Stop works on the built-in engine (ClickHouse) only; on Trino it stops the page waiting, not the query.
- A long time limit keeps a request open as long: a 30-minute limit means a dashboard may take 30 minutes to answer.
- The limits are per install and per role, not per dashboard.

## Asking the assistant

A new tool `get_query_limits` (read; no approval) answers the limits that apply
to the person. The assistant's `run_sql` and chart previews obey the same
limits, and their "stopped after N seconds" result carries the limit so the
assistant can say it. The assistant cannot change limits (an administrator's
setting; it says where to change them). "Stop the running query" maps to the
same cancel the Stop button uses, for the person's own queries only. Prompt
line; capability named; fixture regenerated; one `AI-15` case ("Why did this
chart time out?"). The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-5-AC1` Lowering the time limit to 10 s stops a slow tile at 10 s with a message naming the limit.
- `BI-5-AC2` Stop in Query Studio on a slow query: within 5 seconds the query is no longer listed in Operations > Workloads.
- `BI-5-AC3` A user without `settings:write` cannot change the limits, and a failure shows a plain sentence.
- `BI-5-AC4` *(proposed)* A role given 5 minutes runs a 2-minute chart that the install's 60 seconds stops for everyone else.
- `AI-x-AC1` "What are my query limits?" answers the person's own limits.
