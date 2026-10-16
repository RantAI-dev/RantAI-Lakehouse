# Parameters

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-11`, with the `BI-11` part of `AI-6` (assistant parity) |
| Status | Planner defaults; the owner asked for best practice and confirms at the end-of-phase QA |
| Plan | `docs/superpowers/plans/2026-10-11-bi-11-parameters.md` |

## Problem

The only thing a viewer can change on a dashboard is a filter. They cannot
set a target, choose how many bars to show, or switch a chart from
visitors to revenue without an editor making a second chart.

## What the user can do when this is done

1. As an editor, define a named parameter: a number, a text, a date, or a choice from a list, with a default value.
2. Use it in a formula (`[visitors] / {target}`), as a chart's top-N, as a reference line on a chart, or to choose which column a chart groups by or measures.
3. Put its control in a dashboard's filter row; a viewer changes the value and every chart that uses it follows. The value travels in the URL.
4. Ask the assistant to add a parameter and use it.

## Not included

- Parameters inside the SQL of a SQL source or saved query: that is `QS-5`, which does not exist yet. This slice waits for it and says so in the product.
- A viewer changing a parameter in an embed or public link: they see the default (`BI-26`).
- Parameters that depend on each other, or a list filled from a query.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A parameter is defined once for the deployment and can be used on any dashboard, because formulas are shared across dashboards too (Tableau's model) | Planner default | Owner to confirm at QA |
| 2 | Defining, changing and deleting a parameter needs `dashboard:write`; changing its value on a dashboard needs only to see the dashboard | Planner default | Owner to confirm at QA |
| 3 | A parameter that a chart or formula uses cannot be deleted; the refusal names what uses it | Planner default | Owner to confirm at QA |
| 4 | A dashboard shows a parameter's control when an editor adds it to that dashboard; charts that use a parameter not shown there take its default | Planner default | Owner to confirm at QA |
| 5 | A parameter that switches a chart's column offers only the columns its editor listed; nothing else can be chosen, by any route | Security posture | 2026-10-11 |
| 6 | A number parameter may carry a minimum and maximum; a value outside them is refused, not clamped silently | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- A parameter's value is the viewer's own for that visit, like a filter; "Save as default" keeps it for the dashboard.
- A parameter used as top-N cannot exceed the chart type's own limit.
- Parameters do not reach SQL sources until `QS-5` is built.

## Acceptance checklist

- `BI-11-AC1` A number parameter "target" used in a formula changes the chart when the viewer changes it, and the value is in the URL.
- `BI-11-AC2` A list parameter switches a chart's measure between two columns.
- `BI-11-AC3` A parameter as top-N changes how many bars show; a reference line follows a parameter.
- `BI-11-AC4` A user without `dashboard:write` cannot define a parameter; a value outside its range or list is refused with a plain message.
- `AI-6-AC1` (the `BI-11` part) "add a target parameter of 20,000 and draw it as a line on this chart" works.
