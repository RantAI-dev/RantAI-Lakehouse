# Sharing, alerts and permissions

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `BI-19`, with `AI-11` (assistant parity: share links, with approval) |
| Spec | `docs/core/specs/bi-19.md`, `docs/core/specs/ai-11.md` |
| Status | Ready. The owner signed every "before building" decision on 2026-10-18 (all 19 recommendations accepted); the rest are confirmed at the phase 4 QA |
| Plan | `docs/superpowers/plans/2026-10-18-bi-19-sharing.md` |
| Blocked by | `RPT-1` for part F only (digest schedules) |

## Problem

Who may see a dashboard is decided by role, for every dashboard at once. An
Analyst cannot open any dashboard. A Dashboard Viewer sees every shared
dashboard; nobody can keep one folder for the finance team only.

A public link never expires and has no password. Anyone who has the link
sees the dashboard for as long as the link is on.

A digest is sent every 15 minutes, with no schedule. An alert watches one
number over a whole table, and sends again on every check while the number
stays over the line.

A chart cannot be saved as an image, nobody can comment on a dashboard, and
a dashboard exported as a file cannot be brought back in.

## What the user can do when this is done

Part A, public links:

1. Make a public link that expires: in 1, 7, 30 or 90 days, or on a chosen date up to a year away. 30 days is chosen for you.
2. Add a password to a public link. Visitors type it once per browser session.
3. See when a link expires, change its expiry or password, and switch it off at once.
4. As an administrator, switch public links off for the whole company.

Part B, permissions:

5. Give a person or a role "view", "edit" or "manage" on one dashboard or on a folder. What is given on a folder applies to everything in it.
6. Mark a dashboard or a folder "only the people listed". Then only those people, and administrators, see it.
7. Give an Analyst "view" on one dashboard: they see that dashboard and nothing else.

Part C, alerts:

8. Set an alert on any chart or saved metric, with its own filters (for example only Bali).
9. Choose when it fires: the value goes above a line, goes below a line, or the chart returns any rows.
10. Get one message when the value crosses the line, and one when it is back to normal. Nothing in between.

Part D, images and import:

11. Download a chart as a PNG image.
12. Import a dashboard file exported from this or another deployment, choosing which of your tables each of its tables becomes.

Part E, comments:

13. Comment on a dashboard, and mention a colleague with @. They are told in the console and by email.

Part F, digests (after `RPT-1` is merged):

14. Give each digest its own schedule, like a scheduled report.

And ask the assistant: "Make a public link for this dashboard" (it asks a
person to approve first).

## Not included

- Write-back (rejected in the spec).
- Slack, Teams, Office and Google channels (owner decision, backlog row).
- Scheduled reports (`RPT-1`), PDF and Excel (`BI-1`), embedding (`BI-26`).
- Groups of people. A permission is given to a person or to a role.
- Comments on a single chart, threads, reactions, or comments on a public link.
- Alerts checked more often than every 15 minutes.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| A1 | Every public link expires. You choose 1, 7, 30 or 90 days or a date; 30 days is preselected; a year is the most | Spec (30 days, proposed); the one-year cap is a planner default | Signed by the owner, 2026-10-18 |
| A2 | Links that exist today get an expiry 30 days after the upgrade. The Share dialog says so | Planner default | Signed by the owner, 2026-10-18 |
| A3 | A password is optional, at least 8 characters, and is stored only as a one-way hash. The owner cannot read it back, only replace it | Planner default | Owner to confirm at QA |
| A4 | Five wrong passwords on one link within 15 minutes lock that link's password page for 15 minutes. The lock looks the same as a wrong password | Planner default (the sign-in lock's numbers) | Owner to confirm at QA |
| A5 | An expired, switched-off or unknown link all show the same "This link does not work" page. Nobody can tell which | Planner default | Owner to confirm at QA |
| A6 | One public link per dashboard. Making a new one ends the old one | Planner default | Owner to confirm at QA |
| A7 | An administrator can switch public links off for everyone (Admin > Settings). It is on at first, as today. Switching it off stops every link at once; switching it on again brings back the links that have not expired | Planner default | Signed by the owner, 2026-10-18 |
| A8 | Creating, changing and ending a link is written to the audit log | Planner default | Owner to confirm at QA |
| B1 | Three levels. View: open it. Edit: change its charts, filters and layout. Manage: edit, plus share it (people, public link, embed) and delete it | Spec | Decided by the spec |
| B2 | Nothing changes until someone gives or restricts. Today's role rights stay as the starting point: `dashboard:read` is view and `dashboard:write` is manage on every dashboard that is not restricted | Planner default | Signed by the owner, 2026-10-18 |
| B3 | "Only the people listed" removes the starting point for that dashboard or folder and everything in it. Administrators (who manage users) always see everything | Planner default | Signed by the owner, 2026-10-18 |
| B4 | A folder's permissions apply to its subfolders and dashboards. A dashboard can add to them, never take away: the higher level wins | Planner default (Metabase collections) | Owner to confirm at QA |
| B5 | Permissions go to a person or a role, never to "everyone outside". The personal space cannot be shared this way; sharing a draft still means moving it to a folder (layout page, decision 6) | Planner default | Owner to confirm at QA |
| B6 | Who can give permissions: someone with manage on the item. A person cannot give a level higher than their own | Planner default | Owner to confirm at QA |
| B7 | Moving a dashboard into another folder changes who sees it. The dialog says who gains or loses access before you confirm | Planner default | Owner to confirm at QA |
| B8 | Part B ships in two steps: view and "only the people listed" first (B1), then edit and manage (B2) | Planner default | Owner to confirm at QA |
| B9 | A public link or an embed on a dashboard marked "only the people listed" keeps working: a manager made it on purpose. The People tab says so next to the restriction | Planner default | Signed by the owner, 2026-10-18 |
| C1 | An alert reads one number. A KPI or gauge gives its value; a line or bar over time gives its latest point of the chosen series. Any other chart can only use "returns any rows" | Planner default | Owner to confirm at QA |
| C2 | It fires once when the value crosses the line, then once when it is back. A check that cannot run is recorded and changes nothing | Spec | Decided by the spec |
| C3 | Anyone who can view a chart can set an alert on it for themselves. Sending it to other people, outside addresses or a webhook needs the alert permission (`alert:write`), as today | Planner default | Signed by the owner, 2026-10-18 |
| C4 | An alert reads the data as the person who set it (their masking and row filters), like a scheduled report. If they lose access, the alert stops and says why | Planner default (`RPT-1` decision 1) | Owner to confirm at QA |
| C5 | Alerts are still checked every 15 minutes | Planner default (today) | Owner to confirm at QA |
| D1 | The PNG is made on the server, like the PDF (`BI-1`): same masking, same audit, works for the assistant. Without the PDF service it says "Images are not set up on this deployment" | Planner default | Owner to confirm at QA |
| D2 | The image is the chart with its title and the filters applied written under it, 1200 pixels wide, sharp on high-resolution screens | Planner default | Owner to confirm at QA |
| D3 | Import always makes a new dashboard; it never overwrites one. You choose its folder | Planner default | Owner to confirm at QA |
| D4 | Import reads a new "dashboard file" (.json) that the export menu also offers. The YAML export stays for reading but cannot be imported | Planner default | Owner to confirm at QA |
| D5 | A chart whose table, column, metric or calculated field is missing after mapping is left out. The import says which charts and why | Planner default | Owner to confirm at QA |
| E1 | Comments are plain text up to 2,000 characters. Links are clickable; nothing else is formatted | Planner default | Owner to confirm at QA |
| E2 | You can mention only people who can view the dashboard, so a mention never shows them something they may not see | Planner default | Signed by the owner, 2026-10-18 |
| E3 | A mention appears in the bell in the header and, when email is set up, as an email with a link (not the comment's data) | Planner default | Owner to confirm at QA |
| E4 | You can edit or delete your own comment; a manager can delete any. A deleted comment says "deleted" | Planner default | Owner to confirm at QA |
| F1 | A digest gets the same schedule choices as a scheduled report (hourly, daily, weekly, monthly, at a time and time zone) and is sent by the same sender | Spec ("as RPT-1") | Decided by the spec |
| F2 | Digests that exist today become daily at 08:00 in the report time zone, and their owners are told | Planner default | Signed by the owner, 2026-10-18 |

## Limits to tell a customer

- A public link with a password can be locked for 15 minutes by someone guessing. The owner can make a new link.
- Permissions are per person or per role. There are no groups.
- An alert looks at one number every 15 minutes. A spike that comes and goes between two checks is not seen.
- A PNG needs the PDF service (`BI-1`).
- Import cannot read the old YAML file. Export again as a dashboard file.

## Asking the assistant

`AI-11`, folded in here as `AI-8`, `AI-9` and `AI-10` were folded into
their features' pages.

- "Make a public link for this dashboard." The tool `create_public_link`
  waits for a person to approve it (`DEC-8`). The link expires in 30 days
  unless the request names fewer days; the assistant cannot make a link
  last longer than 30 days, cannot set or read a password (passwords never
  pass through the chat), and refuses when public links are switched off.
- "Switch off the public link." `revoke_public_link` narrows access, so it
  asks for a confirm in the chat, not an approval (question 5 in
  `questions-for-owner.md`).
- "Who can see this dashboard?" `list_dashboard_access` reads the
  permissions, the public link's expiry (never its password) and the embed
  state. Only for someone with manage on the dashboard.
- The assistant never gives or removes permissions. Asked to, it says so
  and points to Share > People.
- `move_chart` changes who can see a chart when the two dashboards have
  different permissions. Then it waits for approval, like the four `DEC-8`
  actions.
- Approval rule for every share tool: the person asking must have manage
  on the dashboard, checked before the approval request is made; the
  approver must also have it, because the approved call runs as the
  approver.
- Alerts (part C): `create_alert_rule` takes a chart or metric, filters and
  the new conditions, with the same rule as the console (yourself only,
  unless you hold `alert:write`).
- Images (part D): `export_chart` (from `BI-1`) gains `format: png`.
- Comments (part E): `list_dashboard_comments` reads them. The assistant
  does not post comments or mention people.
- Import is not offered by the assistant: it needs a file.

The standard request set (`AI-15`) gains `AI-11-AC1` and "Alert me when
revenue drops below 1 billion". The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `BI-19-AC1` An Analyst given view on one dashboard sees it and nothing else (part B).
- `BI-19-AC2` A public link stops working after its expiry (part A).
- `BI-19-AC3` An alert fires once when crossing the threshold, not every check, and once more when it recovers (part C).
- `BI-19-AC4` A user without the permission is refused, and a failure shows an honest message (every part).
- `BI-19-AC5` A link with a password opens only with the password; five wrong tries lock it for 15 minutes; an expired, ended and made-up link all look the same (part A). *(proposed)*
- `BI-19-AC6` A folder marked "only the people listed" hides its dashboards from a Dashboard Viewer who is not listed, and shows them to one who is (part B). *(proposed)*
- `BI-19-AC7` An Analyst given edit on one dashboard can change a chart on it and on no other (part B). *(proposed)*
- `BI-19-AC8` An alert on a chart filtered to Bali uses only Bali's value (part C). *(proposed)*
- `BI-19-AC9` A chart downloads as a PNG with its title and filters; a masked column is masked in it (part D). *(proposed)*
- `BI-19-AC10` A dashboard file imports into a new dashboard with its tables mapped; a chart that cannot be mapped is named and left out (part D). *(proposed)*
- `BI-19-AC11` Mentioning a colleague in a comment tells them in the bell and by email; a person who cannot view the dashboard cannot be mentioned (part E). *(proposed)*
- `BI-19-AC12` A digest set to weekly on Monday 08:00 arrives then and not in between (part F). *(proposed)*
- `AI-11-AC1` "Make a public link for this dashboard." asks for approval, then makes a link that expires.
- `AI-11-AC2` A user without manage rights is refused.
