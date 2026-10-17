# Scheduled reports by email

| | |
| --- | --- |
| Module | Dashboards |
| Backlog | `RPT-1`, with `AI-10` (the schedule half) |
| Status | Planner defaults, built at the owner's request while phase 3 is in QA; the owner confirms or changes them at the phase 4 QA |
| Plan | `docs/superpowers/plans/2026-10-17-rpt-1-scheduled-reports.md` |

## Problem

The only thing that leaves the console on its own is a text digest of KPI
tiles, sent every 15 minutes with no schedule, no attachment, no filter and
no record of whether it arrived.

## What the user can do when this is done

1. From a dashboard's menu, subscribe: every hour, day, week or month at a chosen time and time zone.
2. Choose what arrives: a PDF of the dashboard, and the data of chosen charts as Excel or CSV.
3. Give the subscription its own filter values (for example only Bali).
4. Send it to people in the console and to outside addresses an administrator allows.
5. Choose "skip when empty": no email when every chart has no rows.
6. See each send with its status, send one now to check it, pause or delete a subscription.
7. Be told when a send fails; after 5 failures in a row the subscription pauses itself.
8. As an administrator, list the outside email domains reports may go to.
9. Ask the assistant: "Send this dashboard to me every Monday as a PDF."

## Not included

- Slack, Teams and Google channels (owner decision).
- Changing the existing digests' schedule (`BI-19`).
- A link in the email that opens the data without signing in.

## Decisions

| # | Decision | Default |
| --- | --- | --- |
| 1 | A report is made **as the person who created the subscription**: their masking and row filters, as Metabase does. Every recipient, inside or outside, sees what that person sees. The dialog says so above the recipient list. | Planner |
| 2 | Anyone who can view a dashboard and is allowed to download (`BI-1` decision 9) can subscribe. Only the owner of a subscription or an administrator can change, pause or delete it. | Planner |
| 3 | Outside addresses are allowed only for the domains an administrator lists in Admin > Settings; the list is empty at first, so no report leaves the company until an administrator adds a domain. Console users must be active. At most 50 recipients. | Planner |
| 4 | Schedules: hourly (at a minute), daily (at a time), weekly (a weekday and a time), monthly (a day 1 to 28, or the last day, and a time); in the report time zone unless another is chosen. | Planner |
| 5 | Attachments: the dashboard PDF (`BI-1`) and up to 10 chart files; 20 MB in all. A file that does not fit is left out and the email says which and why. | Planner |
| 6 | "Skip when empty" skips the send when every data chart returns no rows; the skip is recorded. | Planner |
| 7 | Each send is recorded: time, status (sent, failed, skipped), the recipients' count and a plain reason. A failure tells the owner in the console and by email; after 5 failures in a row the subscription pauses and says why. Records are kept 90 days. | Planner |
| 8 | Sends run inside the API: each minute it takes the subscriptions that are due with a database lock, so two API processes never send the same report twice. A send that misses its time (the API was down) runs once when it is back, not once per missed time. | Planner |
| 9 | Without email set up (`SMTP_HOST` empty), subscribing says "Email is not set up on this deployment". Without the PDF service, the PDF attachment is unavailable and the dialog says so. | Planner |
| 10 | The assistant may subscribe the person to a report without approval; adding anyone else as a recipient needs approval (spec `AI-10`). Deleting a subscription needs approval, like every deletion. | Planner |

## Limits to tell a customer

- Everyone on a subscription sees the data as its creator sees it.
- No outside address until an administrator lists its domain.
- 20 MB of attachments per email.
- Missed sends are caught up once, not repeated.

## Asking the assistant

Tools `create_report_subscription`, `list_report_subscriptions`,
`update_report_subscription`, `delete_report_subscription`. The standard
request set gains `AI-10-AC1` ("Send this dashboard to me every Monday as a
PDF."). The AI team reviews this part.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never a pass.

- `RPT-1-AC1` A weekly Monday 08:00 PDF arrives with the dashboard's charts.
- `RPT-1-AC2` A subscription filtered to Bali shows only Bali.
- `RPT-1-AC3` With "skip when empty", no email is sent for an empty result, and the skip is recorded.
- `RPT-1-AC4` A user who may not download is refused; an outside address on a domain not listed is refused; a failure shows a plain sentence and tells the owner.
- `RPT-1-AC5` "Send now" delivers a test email and records it.
- `AI-10-AC1` "Send this dashboard to me every Monday as a PDF." creates a Monday PDF subscription for the person.
