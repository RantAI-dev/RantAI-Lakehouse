# Bins, groups and hierarchies

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-10`, with the `BI-10` part of `AI-6` (assistant parity) |
| Status | Planner defaults; the owner asked for best practice and confirms at the end-of-phase QA |
| Plan | `docs/superpowers/plans/2026-10-11-bi-10-bins-groups-hierarchies.md` |

## Problem

A number column can be a measure but not a grouping: there is no "visitors
in ranges of 500". Values cannot be gathered under names of the user's own
("Java" for six provinces). A click cannot go from a province down to its
cities on the same chart.

## What the user can do when this is done

1. Group by a number column in bins: automatic, a set count (10, 50, 100) or a set width. A bin reads "500 – 1,000".
2. Bin latitude and longitude into a grid and draw one mark per cell on a map.
3. Gather chosen values of a column under names of their own, with the rest left as they are or under "Other", and save that as a field others can pick.
4. Define, once per table, an ordered list of columns (province > city > place) and drill down through it by clicking a value on any chart whose dimension is one of its levels, with a trail to climb back.
5. Click a bin to list its rows or to filter the dashboard to that range.
6. Ask the assistant for any of it.

## Not included

- Histogram as a chart type of its own (`BI-16` part B); a bar chart over bins is the histogram for now.
- Bins on a SQL source's column in automatic mode when the source cannot be read twice within its limits: refused plainly.
- Hierarchies across joined tables.
- Saving a drilled-down state on the dashboard: a drill lasts until the page is left.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Automatic bins aim at about ten bins with round edges | Planner default (Metabase) | Owner to confirm at QA |
| 2 | A bin includes its lower edge and excludes its upper edge; the last bin includes the maximum | Planner default | Owner to confirm at QA |
| 3 | A value group is a saved field of its table, like a calculated field, with the same permissions and the same rule against deleting one in use | Planner default | Owner to confirm at QA |
| 4 | A hierarchy has two to six levels, belongs to one table, and needs `dashboard:write` to define | Planner default | Owner to confirm at QA |
| 5 | Drilling down is a viewer's own, temporary view: it does not change the saved chart and is not shared in the URL | Planner default | Owner to confirm at QA |
| 6 | Grid cells are square in degrees: 0.01, 0.1, 1 or 10, or automatic | Planner default | Owner to confirm at QA |

## Limits to tell a customer

- Automatic bins are worked out from the data the viewer may see at that moment, so two viewers with different row filters can see different edges.
- A grid on a map is in degrees, so cells are not equal in area far from the equator.
- A value group lists values as they are spelled; a new spelling in the data lands in the rest.

## Acceptance checklist

- `BI-10-AC1` Visitors per place binned by width 500: the bins' counts add up to the table's row count.
- `BI-10-AC2` A group "Jawa" over the six Java provinces with "Other" for the rest charts as two bars adding up to the total.
- `BI-10-AC3` With a hierarchy province > city > place, clicking a province shows its cities, clicking a city its places, and the trail climbs back.
- `BI-10-AC4` A grid map at 1 degree shows one mark per cell and the marks' values add up to the total.
- `BI-10-AC5` A user without `dashboard:write` cannot define a group or a hierarchy; a failure shows a plain message.
- `AI-6-AC1` (the `BI-10` part) "visitors per place in bins of 500" is built by the assistant.
