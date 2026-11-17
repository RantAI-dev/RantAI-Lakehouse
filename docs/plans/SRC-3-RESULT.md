# `SRC-3` result: connectors for business apps

Measured on pull request #216 (branch `feat/data-phase-4-connectors`), CI
run 39597521529 on commit `933de5e8`, 2026-10-21, GitHub-hosted
`ubuntu-latest` runners. Counts and lines below are read from the two gate
jobs' logs of that run; nothing is estimated. Every vendor in the gate is a
mock (`ops/g6/saas_stub.py`); no real vendor account was used.

## Gate time (spec row "Each connector: a gate test"; budget 20 minutes per job)

The gate (`g6-saas`) is one script run by two jobs on the same code, each
told its vendors (plan AD9 as amended, 2026-10-21). Job time is the job's
start to its end as GitHub reports it, stack bring-up included.

| Run | Commit | Job | Vendors | Time | Against 20 min |
| --- | --- | --- | --- | --- | --- |
| 39597521529 | `933de5e8` | "Rust · SRC-3 business-app connectors (A)" | HubSpot, Zendesk, Salesforce, Google Analytics 4, ServiceNow | 7 min 18 s (the script: 338 s) | within |
| 39597521529 | `933de5e8` | "Rust · SRC-3 business-app connectors (B)" | Confluence, SharePoint, Dynamics 365, Workday, NetSuite | 13 min 23 s (the script: 716 s) | within |
| 39585654845 | `81ec8621` | one job, before the split | not re-read | 14 min 45 s | within |
| 39572336905 | `7a8af090` | one job, before the split | eight vendors (planner addendum, 2026-10-21) | 16 min 17 s | within, close |

Only one completed run has the split. The two earlier runs are the
unsplit job that led to the split, kept for the trend; the run between
`81ec8621` and `933de5e8` (39596415789) was cancelled before the jobs
started. So the split's time rests on **one run**; the slowest job, B, is at
67% of the budget. The 13 product entries of the unsplit gate (the slice 6 handoff's count) are now 6 (A)
and 10 (B), counted in the logs; no step was removed to fit the budget.

## What the gate proved, per vendor

From the `[saas]` lines of the two logs. "Entry" is a product entry: the
same vendor once with the key or token credential, and (where two are listed)
once with the browser sign-in credential; the numbers are the same in both.

| Vendor | Job | Product step time per entry | Rows in Bronze after the load | Reload | Deletes |
| --- | --- | --- | --- | --- | --- |
| HubSpot | A | 43 s | 3 deals | the changed and the new record merged on the key, no duplicate | none: HubSpot's search omits archived deals, so none is claimed |
| Zendesk | A | 46 s | 6 tickets (the export sent 8 rows, duplicates at the page boundaries; each ticket read once) | the changed, the new and the deleted ticket, nothing older, nothing twice | the deleted ticket arrives with `status = deleted` |
| ServiceNow | A | 52 s | 3 incidents over 2 pages (Link header) | only the changed, the new and the cursor record | none claimed |
| Salesforce | A | 52 s | 3 accounts over 2 batches (`nextRecordsUrl`) | the changed, the new and the deleted record through `queryAll`; the unchanged one not read | the deleted account arrives with `is_deleted` |
| Google Analytics 4 | A | 62 s and 56 s | 10 rows (5 days x 2 countries) over 3 pages | the last four days read again with corrected numbers, merged on country and date | not applicable (a report) |
| Confluence | B | 64 s and 66 s | 5 pages over 3 cursor pages | the changed and the new page; the read stopped near the cursor after 2 of 3 pages | none claimed |
| SharePoint | B | 76 s and 76 s | 5 items over 3 pages of the delta feed | the changed, the new and the deleted item by the saved delta link; a 410 on the saved link made a full read and the new link was kept | the deleted item arrives with `is_deleted` |
| Dynamics 365 | B | 66 s and 66 s | 5 rows over 3 pages (`@odata.nextLink`) | a table without tracking: filtered by `modifiedon`, no delete; a tracked table: delta link, the deleted row marked, and a delta token Dataverse no longer serves made a full read | `is_deleted` on tracked tables only |
| Workday Reports | B | 72 s and 66 s | 3 rows from one response | the whole report again and the table replaced; the deleted row is gone | gone with the replace, not marked |
| NetSuite | B | 66 s and 66 s | 3 customers over 2 pages (`hasMore`); 3 items by key | the changed and the new customer by the lagged `lastmodifieddate` filter | none claimed |

Also asserted for every vendor, in both credentials where two exist: the API's
connection test names the account; discovery lists the objects (the counts in
the log: HubSpot 9, Zendesk 31, ServiceNow 5, Salesforce 2, Google Analytics 4
1 report, Confluence 4, SharePoint 1 list, Dynamics 365 2, Workday 1 report,
NetSuite 8); a 429 with `Retry-After` is waited out and a limit that does
not lift fails as `rate_limited`; a forbidden or revoked sign-in gives the
fixed failure text and none of the vendor's words reach the response.
Salesforce also: a 403 `REQUEST_LIMIT_EXCEEDED` fails as `rate_limited` and
is not retried, and an `instance_url` outside `.salesforce.com` or
`.force.com` stops the run. Schema change: new fields show in the vendor's
describe **before** any load for HubSpot, Salesforce, ServiceNow, Dynamics
365 and Google Analytics 4 (a custom dimension); for Zendesk, Confluence,
SharePoint, Workday and NetSuite the new column arrives in the records and the
observer reports it **after** the load (no describe call).

Preview: Zendesk, Confluence, SharePoint, Workday and NetSuite answer the
preview honestly as unsupported (no describe); HubSpot, ServiceNow,
Salesforce, Dynamics 365 and Google Analytics 4 preview 10, 5, 6, 6 and 4
columns.

## Supported or not (decision 10)

Eight types are `supported = true` after migration `0096`, each with all four
steps `real` in `ops/gates/coverage.json`: Salesforce, ServiceNow, Google
Analytics 4, HubSpot, Zendesk, Confluence, Dynamics 365, SharePoint.
Workday Reports and NetSuite pass the same steps against their mocks above
and stay unsupported until a test on a real account.

## Not measured

- **Any real vendor account** (spec check `SRC-3-AC1`). Every wire detail is
  checked against the mock, which the developer wrote from documentation; a
  vendor that behaves differently from its documentation fails only on a real
  account. Workday's documentation was not readable (slice 6 handoff), so its
  token request, `Report_Entry` shape and host forms are the least checked;
  NetSuite's revocation address, who-am-i path and 100,000-row limit were
  not confirmed.
- **Browser sign-in at a real vendor.** The gate's "browser sign-in" credential
  is a refresh grant against the mock; the redirect and consent screens of
  Salesforce, Microsoft, Google, Atlassian and the others were never shown.
- **NetSuite schema changes before the load** (decision 9). NetSuite has no
  `describe`: SuiteQL does not list a table's columns, so a changed column is
  seen after the load. The spec target is not met for this vendor.
- **Volume.** Each vendor loaded 3 to 10 rows. No load of a large object,
  no vendor rate limit met for real, no run longer than a minute.
- **More than one run of the split gate** (above), and the time of the gate on
  other runners.
- **The console in a browser** for the SaaS screens (type picker, object
  choice, sign-in buttons, report editor). The product owner's QA covers it.
- **Incremental load of Workday** (the report is always read in full), and
  the second wave: both are outside this phase.
