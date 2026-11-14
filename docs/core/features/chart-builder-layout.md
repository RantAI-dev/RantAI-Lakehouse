# The chart builder's layout: a canvas with a data panel and an inspector

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-58` |
| Kind | Feature |
| Spec | `docs/core/specs/bi-58.md` |
| Status | Building |
| Priority | P1 |
| Owner | The module's owner, in the base. |
| Acceptor | The product owner. |
| Plan | `docs/superpowers/plans/2026-11-14-bi-58-chart-builder-layout.md` |
| Started | 2026-11-14 |
| Shipped | |
| Evidence | |

## Problem

See the spec: three cramped columns, an empty preview, accordions of equal weight,
overlapping header labels, and an assistant bar that covers controls.

## What the user can do when this is done

1. Open **New chart** or **Edit chart** and see the chart as the page. The preview sits in
   the centre, with the title and description edited in place above it.
2. Pick a source in the **data panel** on the left. Its columns are grouped as Dates, Text and
   Numbers. One click on a column sets it as the X axis or the value, and a small menu offers
   split, filter or "use as value".
3. Pick the chart type from icons above the preview. **More** opens a gallery where every type
   is a small picture of the chosen data. A type that cannot draw this data is greyed out,
   with one line saying why.
4. Change everything else in the **inspector** on the right, in three tabs:
   - **Data**: X axis, value and aggregate, split into series, bins and date grouping, a second
     source, parameters, KPI compare;
   - **Sort & filter**: sort, top N and "Other", filters, steps;
   - **Style**: the format section that exists today.
5. Choose the dashboard and width, then Save or Cancel, from one header row.

## Not included

- The sentence-and-assistant builder (mockup C). It needs language support first (owner,
  2026-11-14).
- New chart features, and a change to the chart palette.
- A change to the saved chart spec, the API or the assistant's tools.

## Asking the assistant

Nothing changes for the assistant: the builder's state and the saved spec are the same
(assistant parity, AGENTS.md). The page context the builder sends today stays the same.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| L1 | Layout A, Canvas, from the mockups of 2026-11-14 (branch `design/chart-builder-mockups`, `6ede5307`, local) | Owner's choice | Owner, 2026-11-14 |
| L2 | The chart-type gallery draws each type from the chosen data (from mockup B) | Owner's choice | Owner, 2026-11-14 |
| L3 | Applies to both New chart and Edit chart | Planner | Owner to confirm at QA |
| L4 | Below 1280 px wide, the data panel folds into a "Pick data" button; at phone width the page stacks | Planner (mockup A) | Owner to confirm at QA |
| L5 | Known builder findings from the 2026-11-14 QA are fixed in the same change, because they are in the same files (plan §4) | Planner | Owner to confirm at QA |

## Limits

- The gallery draws at most the first 200 rows of the preview result, so it costs no extra
  query.

## Acceptance checklist

- [ ] `BI-58-AC1`: at 1366×900 the preview is at least half the page's width, and at
  1190×800 nothing overlaps and the assistant bar covers no control.
- [ ] `BI-58-AC2`: the type gallery shows the chosen data in each type; types that cannot draw
  it are greyed out with a reason.
- [ ] `BI-58-AC3`: with a source picked, one click on a date column and one on a number
  column give a preview.
- [ ] `BI-58-AC4`: every setting of today's builder is in the inspector, and editing a saved
  chart and saving it unchanged leaves its spec the same.
- [ ] `BI-58-AC5`: light and dark mode both look finished, and the phone width stacks with no
  horizontal scroll.
