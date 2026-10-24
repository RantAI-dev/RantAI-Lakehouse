# Export a dashboard as PDF and its data as Excel

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-1`, with `AI-10` (the export half; the schedule half ships with `RPT-1`) |
| Status | Decision 1 signed by the owner (2026-10-17). The rest are planner defaults, built at the owner's request to build the phase while the previous one is in QA; the owner confirms or changes them at the phase 4 QA |
| Plan | `docs/superpowers/plans/2026-10-17-bi-1-export.md` |

## Problem

"Export PDF" is the browser's print dialog: what comes out depends on the
browser, the window size and the tab that is open, and nothing can make a
PDF without a person at a browser (so scheduled reports cannot attach one).
A chart's data downloads only as CSV, only from a raw table, and only up to
100,000 rows (1,999 over a SQL source). There is no Excel file.

## What the user can do when this is done

1. From a dashboard's menu, download a PDF of the whole dashboard: every tab, A4 or Letter, portrait or landscape, with the filters that are applied listed at the top.
2. From any chart's menu, download its data as Excel (.xlsx) or CSV, with the values formatted as on the chart or as raw values.
3. Download up to 1,000,000 rows of a raw table, including one over a SQL source.
4. As an administrator, in Admin > Settings, switch downloads off for chosen roles.
5. Ask the assistant: "Give me this dashboard as a PDF" or "Download this table as Excel".

## Not included

- PowerPoint (owner decision, `BI-24`).
- PNG of a chart (`BI-19`).
- Downloads from public links and signed embeds (as today).
- Scheduled sending (`RPT-1`).

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | The PDF is made by a Gotenberg container (Chromium) in the deployment, which opens a print view of the dashboard in the console over the internal network. | **Owner, 2026-10-17** |
| 2 | The print view reads the data **as the person who asked** (their masking and row filters), not as the embed viewer role: a short-lived render token names the person, and the API re-reads that person's roles when the view asks for data. The token lives 2 minutes and is good only for that dashboard and those filter values. | Planner |
| 3 | Every tab is printed, each starting on a new page with its title; sections print unfolded. Image and web-page cards print as "Shown in the console only": the PDF renderer may open only the console itself, so it never fetches anything from outside the deployment. | Planner |
| 4 | The first page lists the dashboard's name, the filters and parameters applied, who downloaded it and when (in the report time zone). Pages are numbered. | Planner |
| 5 | A PDF that is not ready in 60 seconds fails with a plain sentence; at most 2 PDFs are made at once per API process, a third waits its turn. | Planner |
| 6 | When the deployment has no Gotenberg (`GOTENBERG_URL` unset), the PDF item says "PDF export is not set up on this deployment" and the browser's print stays available as before. | Planner |
| 7 | Excel keeps numbers as numbers: "formatted" sets each column's Excel number format from the chart's value format (`BI-17` A); "raw" writes plain values. CSV "formatted" writes the text shown on the chart; CSV "raw" the plain values. Cells that start like a formula are neutralised in both (as the CSV export does today). | Planner |
| 8 | A raw table exports up to 1,000,000 rows (Metabase's limit), also over a SQL source; a grouped chart exports its own result. A file that reached the limit says so in its name and in the download message. | Planner |
| 9 | Downloads stay governed by `dashboard:read`. An administrator can switch them off per role in Admin > Settings ("Roles that may not download"); a user whose every role is switched off cannot download, and the menu items are hidden. Nothing changes until an administrator chooses. | Planner |
| 10 | Every download (PDF, Excel, CSV) is written to the audit log before the file is sent, as the CSV export does today; with the audit log down, the download is refused. | Planner |
| 11 | The assistant's export tool returns a download link to the same export the menu uses; the file is made when the person opens it, with their permissions. Nothing is stored. | Planner |

## Owner decisions at the phase 4 QA (2026-10-23)

- The PDF stacks every tile full width, one per row; it does not copy the dashboard's columns. The owner accepted this for now.

## Limits to tell a customer

- PDFs need the Gotenberg container; without it, only the browser's print.
- Image and web-page cards do not appear in the PDF.
- 1,000,000 rows at most per download.
- A PDF takes a few seconds per tab and fails after 60 seconds.

## Asking the assistant

A new tool `export_dashboard { board, format: pdf, paper?, orientation? }`
and `export_chart { chart, format: xlsx|csv, values: formatted|raw }` return
a link the person opens. They read, so they need no approval; the link works
only for the person and their permissions. The standard request set gains
"Give me this dashboard as a PDF". The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-1-AC1` A dashboard downloads as a PDF matching the screen, with every tab and the applied filters listed.
- `BI-1-AC2` A 200,000-row chart downloads as Excel.
- `BI-1-AC3` A masked column is masked in the PDF, the Excel and the CSV.
- `BI-1-AC4` A user whose roles are switched off cannot download, a deployment without Gotenberg says so, and a failure shows a plain sentence.
- `BI-1-AC5` Excel "formatted" shows Rp 1,2 M style values that are still numbers in Excel.
- `AI-10-AC2` "Give me this dashboard as a PDF" returns a link that downloads it.
