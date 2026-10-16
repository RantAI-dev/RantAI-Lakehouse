# Reusable metrics

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-2`, with `AI-7` (assistant parity) |
| Status | Planner defaults; the owner asked for best practice and confirms at the end-of-phase QA |
| Plan | `docs/superpowers/plans/2026-10-11-bi-2-reusable-metrics.md` |

## Problem

"Revenue" or "active customers" is rebuilt in every chart, and two charts
can disagree. The built-in metric cards are fixed in a deployment file. The
assistant writes its own SQL for a number the company already has a
definition for.

## What the user can do when this is done

1. Define a metric once: a name, a description, a formula, the table it is computed from, its filters, and its default date column and grain.
2. Save a named filter (a segment), such as "domestic visitors", and reuse it in metrics and charts.
3. Pick a metric by name as a chart's measure or as a KPI; every chart then shows the same number.
4. See, for any metric, the comparison with the previous period and with the same period last year.
5. See which charts use a metric before changing it.
6. Set an alert on a metric.
7. Ask the assistant a question and have it answer from a named metric when one matches, saying which one.

## Not included

- Certified metrics: that mark comes from `DATA-12`, which does not exist yet. This slice waits for it and says so.
- Metrics across joined tables or SQL sources that need more than one table: a metric reads one table or one SQL source.
- Metric goals, owners' approval flows, and a metric catalogue page beyond a list.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A metric's formula is written in the product's formula language (`BI-8`) and must be an aggregate | Planner default | Owner to confirm at QA |
| 2 | A metric's own filters apply only to that metric's number, so a chart can show a filtered metric beside an unfiltered measure | Planner default | Owner to confirm at QA |
| 3 | Defining and changing metrics and segments needs a new permission, `metric:write`; using them needs only to see the dashboard | Planner default | Owner to confirm at QA |
| 4 | A metric or segment in use cannot be deleted; changing a metric shows the charts that use it first and then applies to all of them at once | Planner default | Owner to confirm at QA |
| 5 | The built-in metric cards from the deployment file stay as they are | Planner default | Owner to confirm at QA |
| 6 | The assistant prefers a named metric when one matches the question and names it in its answer; when none matches it says so before writing a query | Spec (`AI-7`) | 2026-10-11 |

## Limits to tell a customer

- A metric belongs to one table; a chart on another table cannot pick it.
- Comparisons need the metric's date column.
- Certification is not available until `DATA-12`.
- An alert rule on a metric is evaluated with the fixed viewer role that every alert rule runs with, not with the access of whoever wrote the rule (a rule has no owner); a metric that role cannot read makes the rule fail with a stated reason instead of firing.

## Acceptance checklist

- `BI-2-AC1` A metric "total visitors" defined once shows the same number in a bar chart, a KPI and the assistant's answer.
- `BI-2-AC2` A metric with a filter, beside an unfiltered measure on one chart, shows two different numbers that each match a direct query.
- `BI-2-AC3` A KPI on a metric shows previous period and same period last year.
- `BI-2-AC4` Changing the metric's formula lists the charts that use it and then changes all of them; deleting it while used is refused.
- `BI-2-AC5` A user without `metric:write` cannot define a metric; a failure shows a plain message.
- `BI-2-AC6` An alert rule "total visitors greater than 100,000" on the metric fires when the metric is above it and not when it is below; if the metric is deleted or cannot be read the rule is reported as failed with a reason, and a metric a rule watches cannot be deleted.
- `AI-7-AC1` Asked for a number a metric defines, the assistant uses the metric and says which.
