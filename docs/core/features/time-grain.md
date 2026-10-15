# Group dates by day, week, month, quarter, year

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-9`, with `AI-3` (assistant parity) |
| Status | Decisions signed 2026-10-10 |
| Plan | `docs/superpowers/plans/2026-10-10-bi-9-time-grain.md` |

## Problem

A date column can only be charted as it is stored. A table with one row
per day cannot be shown per month without writing SQL, and a timestamp
column gives one point per timestamp. A calendar chart on a timestamp
column groups per timestamp and silently keeps one value per day. Nothing
says which time zone "today" or "this month" means: date filters use the
database server's clock. There is no place in the console to set anything
for the whole deployment.

## What the user can do when this is done

1. In the chart builder, after picking a date or timestamp column, choose how to group it: minute, hour, day, week, month, quarter, year.
2. Or choose a part of the date instead: hour of day, day of week, day of month, week of year, month of year, quarter of year. Day of week shows seven bars in week order.
3. On a dashboard, switch the grain of every time-grouped chart at once (for example daily to monthly) from one control in the filter row; an editor can save that choice with "Save as default".
4. Click a bucket (for example March 2026) to list its rows or to filter the dashboard to that range.
5. As an administrator, open Admin > Settings and set the report time zone and the first day of the week. Grouping, "today", "this week" and "this month" all follow them.
6. Ask the assistant: "make this monthly".

## Not included

- A grain on maps, sankey, sunburst, box plot, scatter, bubble, KPI, gauge, table and text tiles.
- Choosing which charts follow the dashboard control: every chart with a grain follows it.
- Fiscal years and custom calendars.
- A grain switch for viewers of an embed or public link: they see the saved grain (`BI-26`).
- A time zone per user or per tenant: one setting for the deployment.
- The built-in dashboard's tiles, which are fixed SQL.

## Asking the assistant

`create_chart` and `update_chart` take `grain`. The tools that describe a
table or SQL source now say which columns are dates, so the assistant can
tell where a grain applies. The standard request set gains "Make this
monthly." (`AI-3-AC1`). The AI team reviews this part.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A new Admin > Settings page holds the report time zone (default `Asia/Jakarta`) and the first day of the week (default Monday) | — | Owner, 2026-10-10 |
| 2 | The dashboard grain control changes every chart that has a grain; there is no per-chart linking | — | Owner, 2026-10-10 |
| 3 | A click on a bucket becomes a date-range filter, and the record list shows that range. A click on a date part (for example "Monday") does nothing, and the tile says so | — | Owner, 2026-10-10 |
| 4 | A chart with a grain may return up to 1000 points (others stay at 100); a cut-off result is marked on the tile | — | Owner, 2026-10-10 |
| 5 | Grains and date parts are Metabase's list | Spec | 2026-10-10 |
| 6 | Changing the settings needs a new permission, `settings:write`; reading them needs only a login (the console needs them to label axes) | Planner default | Owner, 2026-10-11 |
| 7 | First day of the week is Monday or Sunday, no other day | Planner default | Owner, 2026-10-11 |
| 8 | Date filters made in `BI-18` part A ("this month", "last 7 days", a day range on a timestamp column) follow the same time zone and week start, so a filter and a chart never disagree about where a month begins | Planner default | Owner, 2026-10-11 |
| 9 | The dashboard control offers day, week, month, quarter, year, plus hour and minute only when every grained chart is on a timestamp column | Planner default | Owner, 2026-10-11 |

## Limits to tell a customer

- One time zone for everybody. A viewer in another zone sees the report zone's days.
- Minute and hour buckets can be listed (View records) but not used as a dashboard filter; dashboard date filters work in whole days.
- A chart on a plain date column (no time) ignores the time zone and cannot be grouped by minute or hour.
- A chart with a grain shows at most the latest 1000 buckets.
- Decision 8 changes existing dashboards slightly: a relative date filter that used the server's clock now uses the report time zone.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-9-AC1` A line chart of daily rows switches to monthly from the dashboard control, and back.
- `BI-9-AC2` Day of week shows seven bars, Monday to Sunday; with the setting on Sunday, Sunday to Saturday.
- `BI-9-AC3` A user without `dashboard:write` cannot save a grain on a chart or a dashboard, a user without `settings:write` cannot change the settings, and a grain on a text column is refused with a plain message.
- `BI-9-AC4` Changing the report time zone moves a timestamp near midnight to the other day on an existing chart, without re-saving the chart.
- `BI-9-AC5` Clicking "March 2026" on a monthly chart lists exactly March's rows, and "Filter" sets a 1 to 31 March range on the dashboard.
- `BI-9-AC6` A chart grouped by hour over more than 1000 hours shows the latest 1000 and the cut-off mark.
- `AI-3-AC1` "Make this monthly." turns a daily chart into a monthly one.
