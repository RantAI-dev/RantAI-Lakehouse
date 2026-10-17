# Chart formatting

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-17` (part A), with `AI-8` (assistant parity) |
| Status | Planner defaults, built at the owner's request to build phase 3 straight through (2026-10-15); the owner confirms or changes each decision at the phase 3 QA |
| Plan | `docs/superpowers/plans/2026-10-15-bi-17a-chart-formatting.md` |

## Problem

A chart shows numbers as whole numbers with no unit, and a number format
saved on a chart is ignored. Every chart uses one fixed palette. Axes cannot
be titled, scaled or hidden, values cannot be written on the bars, and
nothing marks a goal, an average or a trend. A table cell or a KPI cannot be
coloured by its value. A dashboard always follows the console's light or
dark mode and its font.

## What the user can do when this is done

1. Format each value of a chart: decimals (0 to 10), thousands separator, currency (any ISO code, shown as a symbol or a code), percent, scientific, compact (1.2 K, 3.4 M), a prefix and a suffix. Set it once for the chart or per value.
2. Format a date: date style, 12- or 24-hour time, short or full month and day names.
3. Colour a chart: a colour per series, a palette (built-in or the company palette an administrator sets), or colour by value (a range from one colour to another).
4. Shape the axes: titles, minimum and maximum, logarithmic scale, show or hide, label rotation, and a second Y axis for one or more values.
5. Write values on bars and points, in the value's format, without labels overlapping.
6. Colour table cells and KPI values by rules ("below 1,000 in red") or by a colour range.
7. Add reference lines: a goal line, an average, median, minimum or maximum line, a linear trend line, and a band between two values.
8. Pin a short note on a point of a chart.
9. Choose which fields a tooltip shows.
10. Give a dashboard a theme (follow the console, light or dark) and a font from the list an administrator allows.
11. As an administrator, in Admin > Settings: set the company palette, the allowed fonts, and the number locale.
12. Ask the assistant: "Show values in rupiah, make the bars green, add a target line at 1,000."

## Not included

- Forecasting and clustering (`BI-17` part B, roadmap phase 6).
- Fonts from the internet: only the fonts shipped with the console.
- Extra tooltip fields that the chart does not already query.
- Rich text or links in notes.
- Formatting of the built-in "Main" dashboard's fixed tiles.

## Decisions

Planner defaults, all confirmed by the product owner on 2026-10-17 (phase 3 QA).

| # | Decision | Default |
| --- | --- | --- |
| 1 | A chart has one format, and each value (measure or column) may override it. A chart saved before this keeps showing what it showed, except that a number format already saved on it is now honoured. | Planner, confirmed by the owner 2026-10-17 |
| 2 | Separators follow one deployment setting, "Number locale", in Admin > Settings: `en-US` (1,234.5) or `id-ID` (1.234,5). Default `en-US`, so no existing chart changes until an administrator chooses. | Planner, confirmed by the owner 2026-10-17 |
| 3 | Compact numbers use K, M, B, T in every locale (the spec's 1.2 K, 3.4 M), not the locale's own words (jt, rb). With currency: "Rp 1,2 M" in `id-ID`. | Planner, confirmed by the owner 2026-10-17 |
| 4 | Currency is any ISO 4217 code; an unknown code is refused at save with a plain sentence. | Planner, confirmed by the owner 2026-10-17 |
| 5 | Prefix and suffix are at most 16 characters each, plain text. | Planner, confirmed by the owner 2026-10-17 |
| 6 | Built-in palettes: the current default, plus four (categorical bright, categorical muted, sequential blue, diverging red-blue), the four new ones passing the 3:1 contrast check in light and dark themes. The current default does not (sky 2.77:1 on white; amber, lime, orange also short); the owner chose on 2026-10-17 to keep it unchanged so saved charts keep their colours. The company palette holds 2 to 12 colours; with it set, it is the default for new charts. | Planner, confirmed by the owner 2026-10-17 |
| 7 | Colour by value: a range of two or three colours over the measure's shown minimum and maximum, for single-series bar, column, row, pie, treemap and map charts. | Planner, confirmed by the owner 2026-10-17 |
| 8 | A logarithmic axis needs every value above zero; a chart with a zero or negative value shows the tile message "A logarithmic axis cannot show zero or negative values" instead of drawing a wrong axis. | Planner, confirmed by the owner 2026-10-17 |
| 9 | Reference lines that are computed (average, median, minimum, maximum, trend) use the rows the chart shows. When the result was cut off, the line's label says "of the N shown". | Planner, confirmed by the owner 2026-10-17 |
| 10 | At most 5 reference lines and bands, 20 notes and 20 conditional rules per chart. | Planner, confirmed by the owner 2026-10-17 |
| 11 | A note is pinned to a category or date value and, on a chart with several series, to one series; its text is at most 140 characters, plain text. A note whose point is no longer in the result is listed under the chart as "1 note not shown: its point is not in the result". | Planner, confirmed by the owner 2026-10-17 |
| 12 | Fonts: the console ships a fixed set (Inter, IBM Plex Sans, Source Serif, JetBrains Mono, the system font). An administrator chooses which of them dashboards may use; nothing is loaded from the internet. | Planner, confirmed by the owner 2026-10-17 |
| 13 | A dashboard's theme and font apply to its tiles everywhere it is shown, including public links and signed embeds. | Planner, confirmed by the owner 2026-10-17 |
| 14 | Changing a chart's format needs `dashboard:write`; changing the palette, fonts and number locale needs `settings:write`. | Planner, confirmed by the owner 2026-10-17 |
| 15 | The assistant gets one tool, `format_chart`, that changes only formatting and leaves everything else of the chart as it is; and `set_dashboard_theme`. Neither deletes, shares or publishes, so neither waits for approval. | Planner, confirmed by the owner 2026-10-17 |

## Limits to tell a customer

- One number locale for the whole deployment.
- Computed reference lines describe the rows on screen, not every row of the table, when a chart is cut off.
- Only the console's own fonts.
- A logarithmic axis cannot show zero or negative values.

## Asking the assistant

`format_chart` takes the same formatting fields as the chart builder and
changes only the ones it is given, so the assistant can change one setting
without resending the others; `set_dashboard_theme` sets a dashboard's
theme and font. The standard request set gains `AI-8-AC1`. The AI team reviews this
part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-17-AC1` A revenue column shows Rp 1,2 M with two decimals when so set (number locale `id-ID`, currency IDR, compact, two decimals).
- `BI-17-AC2` Bars take the chosen colours.
- `BI-17-AC3` A goal line at 1,000 appears on a bar chart.
- `BI-17-AC6` A user without `dashboard:write` cannot change a chart's format, a user without `settings:write` cannot change the palette, fonts or locale, and a failure shows a plain sentence.
- `BI-17-AC7` A table colours cells below 1,000 red, and a KPI turns green above its goal.
- `BI-17-AC8` A dashboard set to the dark theme stays dark in a public link while the console is light.
- `AI-8-AC1` "Show values in rupiah, make the bars green, add a target line at 1,000." applies all three changes.

`BI-17-AC4` and `BI-17-AC5` (forecast and clustering) belong to part B.
