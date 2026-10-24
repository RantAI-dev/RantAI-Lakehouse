# Gantt charts, world and country maps, your own boundaries, spatial analysis

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-16` part C, with `AI-5` (each new chart type in the same change) |
| Status | Draft (2026-10-24). Decisions 9 and 10 wait for the owner's signature (questions-for-owner Q3, Q4); the rest are planner defaults the owner confirms at the phase 6 QA |
| Plan | `docs/superpowers/plans/2026-10-24-bi-16c-advanced-charts-maps.md`, in four parts: C1 Gantt, C2 world maps and uploaded boundaries, C3 spatial analysis, C4 custom charts (only if the owner says yes) |
| Spec | `docs/core/specs/bi-16.md` part C |

## Problem

There is no Gantt chart. Maps cover Jakarta and Indonesia only: a customer
outside Indonesia, or one with their own sales regions, cannot draw a region
map. Points cannot be measured against each other or assigned to the region
they fall in. There is no way to add a chart type of one's own.

## What the user can do when this is done

1. **(C1)** Make a Gantt chart: one bar per task from its start to its end, grouped and coloured by a column (team, project), with a line at today.
2. **(C2)** Colour a world map by country, or a country's provinces, using maps that ship with the product.
3. **(C2)** As an administrator, upload a GeoJSON boundary file (up to 50 MB), name it, choose which property holds the region name, and use it for region, pin and grid maps like a built-in map.
4. **(C3)** In a calculated field, measure the distance between two points (`Distance`) and find the region a point falls in on any map (`RegionOf`), then group, filter or colour a region map by it (a spatial join).
5. **(C4, only if the owner says yes)** Add a custom chart from a template that has no code in it.
6. Ask the assistant: "Show the project plan as a Gantt chart", "Colour the world map by revenue", "Which stores are in each of our sales regions?".

## Not included

- Map tiles or satellite backgrounds from outside the deployment (maps stay self-contained, as today).
- Routing, travel time or drive-time areas.
- Editing boundaries in the console (upload a corrected file instead).
- Custom chart code that runs in the browser (see decision 9).

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | **Gantt shape.** A task column, a start and an end (dates or date-times), an optional group. One bar per task and group: from its earliest start to its latest end. At most 1,000 bars; more says "Showing the first 1,000 tasks by start". Rows whose end is before their start are left out and counted in a note on the tile. | Planner default — owner to confirm at QA |
| 2 | **Today line** in the report time zone, on by default, can be switched off; a progress column (0–100 %) is optional and shades the bar. | Planner default — owner to confirm at QA |
| 3 | **World and country maps** ship with the product from Natural Earth (public domain): the world by country, and, per country, its first-level regions (states, provinces), loaded only when chosen. Region names are matched as today, and also by ISO country code. | Planner default — owner to confirm at QA |
| 4 | **Uploaded boundaries.** GeoJSON (a FeatureCollection of polygons) in WGS 84 coordinates; the uploader picks the name property. The file is checked (shape, coordinates in range, at most 20,000 regions) and simplified for drawing to at most 5 MB (each region on its own, so neighbouring borders can show hairline gaps when zoomed in); the full boundaries are kept on the engine for `RegionOf` (decision 7), not as a file. A map is visible to everyone who can make charts; only `settings:write` uploads, renames or deletes. Deleting a map in use is refused and lists the charts. | Planner default — owner to confirm at QA |
| 5 | **Grid maps** are the existing point grid (`grid` on point and heat maps) on any map, built-in or uploaded. | Planner default — owner to confirm at QA |
| 6 | **Distance** is the distance on the Earth's surface (WGS 84 ellipsoid) in kilometres (or metres, chosen in the formula), from latitude and longitude columns. | Planner default — owner to confirm at QA |
| 7 | **RegionOf(map, lat, lon)** names the region of a map a point falls in, or empty when none; it is computed on the engine against the map's full (unsimplified) boundaries, stored on the engine for this purpose when the map is uploaded. | Planner default — owner to confirm at QA |
| 8 | **Masks and row filters** apply to the point data as for every chart; boundaries are not data and have no masks. | Planner default — owner to confirm at QA |
| 9 | **Custom charts.** Recommended: no plugin that runs customer code in the console; if wanted, a later part offers a custom chart from a declarative template (an ECharts option with placeholders for the chart's columns, no scripts, no functions), checked on save. | **Signed by the owner, 2026-10-24** (Q3) |
| 10 | **Upload size.** 50 MB per boundary file (the spec's proposed number), simplified to 5 MB for drawing. | **Signed by the owner, 2026-10-24** (Q4) |

## Limits to tell a customer

- Uploaded borders are drawn simplified; very fine borders lose detail on screen, but `RegionOf` uses the full borders.
- 1,000 bars per Gantt chart.
- Country maps are first-level regions only (states, provinces); finer levels need an uploaded file, as Indonesia's regencies are today.

## Asking the assistant

`AI-5`: the chart tools' schemas gain the `gantt` kind with `start`, `end`,
`group` and `progress`; the map fields accept the new map ids (built-in and
uploaded, listed by a new read tool `list_maps`); `list_formula_functions`
lists `Distance` and `RegionOf` (they come from the formula catalog, so the
assistant's calculated-field tools use them unchanged). Uploading a boundary
file is not an assistant action. Fixture regenerated; the standard request
set gains "Show the project plan as a Gantt chart" and "Colour the world map
by revenue". The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-16-AC5` A Gantt chart shows tasks by start and end, grouped, with a today line. (C1)
- `BI-16-AC7` *(proposed)* A world map is coloured by country from a column of country names or ISO codes. (C2)
- `BI-16-AC4` An uploaded province boundary file colours a region map. (C2)
- `BI-16-AC8` *(proposed)* A calculated field `RegionOf` on store points, grouped by region, colours the uploaded map (a spatial join); `Distance` between two points matches a known distance within 1 %. (C3)
- `BI-16-AC6` A user without `settings:write` cannot upload or delete a map, a malformed file is refused with a plain sentence, and a failure shows an honest message.
- `AI-5-AC2` Each part's pull request includes the assistant schema change; "Show the project plan as a Gantt chart" makes one.
