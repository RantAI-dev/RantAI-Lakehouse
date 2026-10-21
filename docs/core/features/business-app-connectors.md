# Connectors for business apps

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-3` |
| Kind | Feature |
| Spec | `docs/core/specs/src-3.md`. This page follows it row by row; anything the spec does not say is a decision below. The spec has no *(proposed)* number; its one *(proposed)* item, dropping a no-code connector builder, is decision 14 |
| Status | Building, in #214 (the one pull request for phase 4). `DEC-9` decided 2026-10-19 (dlt verified sources where they exist, dlt's `rest_api` source for the rest). Decisions signed 2026-10-19 by the product owner ("setuju dengan plan default itu, selagi sejalan dan sesuai dengan specs"): every default, with decision 3 corrected to the spec's "OAuth where offered" before signing |
| Priority | P3 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Plan | `docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md` |
| Started | 2026-10-19; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | The phase 4 pull request, with the first run of the SaaS gate |

## Problem

There are no connectors for business apps: no CRM, support desk, analytics,
HR, finance or document source. A customer who wants Salesforce or Zendesk
data in the lakehouse has to configure the generic REST connector by hand,
and that connector cannot follow most vendors' paging, cannot load only
what changed, and cannot list a vendor's objects. Read in the code on
2026-10-19 (`main` at `ee8589c3`):

- **No SaaS code at all.** dlt is used only as the loader; no dlt verified
  source and no dlt `rest_api` source is installed. The REST adapter is a
  hand-written generator (`dagster/dispar_orchestrate/adapters/rest.py`).
- **Next-page links are refused.** The REST adapter never dials a URL taken
  from a response (its rule Z5, against server-side request forgery).
  Salesforce, Microsoft Graph, Dynamics 365, ServiceNow, Zendesk and
  Confluence all page by returning the next URL, and Salesforce returns the
  host to use after sign-in.
- **Only databases load incrementally.** Incremental loads and column choice
  are refused for every non-SQL source.
- **A REST source lists only what the user typed.** It has no way to ask a
  vendor which objects exist.
- **OAuth sign-in works** for REST sources since `SRC-12`, with
  `client_secret_post` only.

Checked against the vendors and dlt on 2026-10-19 (sources in the plan):

- Only four of the ten have a dlt verified source: Salesforce, HubSpot,
  Zendesk, Google Analytics. Each has defects we must fix in our copy:
  Salesforce declares no primary key, so "merge" appends duplicates;
  HubSpot reads every record on every run; Zendesk's incremental export has
  a reported duplicate bug. The verified-sources repository installs by
  copying code into the project; there is no package to pin.
- Salesforce's username + password + security-token sign-in stops working
  in Summer '27 (SOAP `login()` retired).
- Workday and NetSuite offer no free developer account. ServiceNow,
  Atlassian, HubSpot, Zendesk, Salesforce and Google do; Microsoft's
  developer tenants need a paid subscription or partner membership.

## What the user can do when this is done

| Spec row | Spec target | What they can do |
| --- | --- | --- |
| Reuse route | `DEC-9` | (No user-visible change; decided 2026-10-19: dlt verified sources where they exist, dlt `rest_api` for the rest.) |
| First wave | Databricks' ten GA SaaS connectors | 1. Pick Salesforce, ServiceNow, Workday Reports, Google Analytics 4, HubSpot, Zendesk, Confluence, Dynamics 365, NetSuite or SharePoint in the connector picker, each with its own form asking only what that vendor needs. |
| Each connector: OAuth where offered | `SRC-12` | 2. Connect by signing in to the vendor in a browser window where the vendor offers it; otherwise with the vendor's integration credential (decision 3). |
| Each connector: table and column choice | `SRC-11` | 3. See the vendor's objects listed by the vendor (decision 5) and choose objects and columns. |
| Each connector: incremental loads | — | 4. Have each run load only what changed since the last, for every object whose API can say what changed; see which objects are re-read in full each run, and why. |
| Each connector: schema change handling | `SRC-8` | 5. Get told when the vendor adds or removes a field, with the connector's schema-change rule applied (decision 9). |
| Each connector: a gate test | `SRC-9` | 6. (operator) See every one of the ten proven by a gate in CI on every change. |
| (failures) | principles 2 and 4 | 7. See a plain message when a sign-in is refused, a permission is missing at the vendor, or the vendor limits the rate; never the vendor's own error text. |

## Not included

- The second wave (Databricks' beta list, about 45): by customer demand,
  each as its own backlog item, after this one.
- A no-code builder for custom connectors (decision 14).
- Writing back to the vendor.
- Files stored in SharePoint or Confluence attachments: SharePoint lists and
  Confluence pages only (decision 5). Google Drive and SharePoint files are
  file sources, a separate item.
- Google Analytics raw events (the BigQuery export Databricks reads):
  reports through the GA4 Data API only (decision 2).

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in `AI-16`. The assistant's connector tools list adapters; the
new one is added there and `tests/fixtures/tool_schemas.json` regenerated, as
for any adapter.

## Decisions

The planner's default is used until signed. Signed 2026-10-19 on the condition that each follows the spec; decision 3 was corrected to the spec before signing.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Where the verified-source code lives. It installs by copying, and three of the four have defects. | Copied once from a pinned upstream commit into the repo, with its Apache-2.0 notice; our fixes are made in the copy, each cited; no automatic re-sync. A later upstream fix is taken by hand, with a review. | 2026-10-19 |
| 2 | Which vendor API each connector reads. Databricks reads a different path for four. | GA4: the GA4 Data API (reports the user defines: dimensions and metrics), not the BigQuery raw-event export. Dynamics 365: the Dataverse Web API, not Synapse Link exports. NetSuite: SuiteQL over REST, not SuiteAnalytics JDBC (needs a licence). Workday: Reports-as-a-Service (one custom report per object). Others: the vendor's main REST API. | 2026-10-19 |
| 3 | How each connector signs in. The spec asks for OAuth sign-in wherever the vendor offers it, and AC1 for "one sign-in". | Sign-in in the browser (OAuth authorisation code, `SRC-12`) for every vendor that offers it: Salesforce, HubSpot, Zendesk, GA4 (Google), ServiceNow, Confluence (Atlassian), Dynamics 365 and SharePoint (Microsoft Entra), NetSuite, and Workday where the tenant has registered an API client. Second way, for customers whose admins use integration credentials: HubSpot private-app token, Zendesk email + API token, GA4 service-account key, ServiceNow and Dynamics/SharePoint client credentials, Confluence email + API token, Workday integration user + password, NetSuite client credentials (certificate). Where `SRC-12`'s sign-in cannot serve a vendor as it stands (it sends the client secret in the form body only), it is extended for that vendor, not dropped. | 2026-10-19 |
| 4 | Salesforce's password + security-token sign-in, retired by Salesforce in Summer '27. | Not offered. | 2026-10-19 |
| 5 | Which objects each connector offers. | Listed by the vendor where it can list them: Salesforce standard and custom objects; HubSpot CRM objects (companies, contacts, deals, tickets, products, quotes, owners, pipelines); Zendesk Support (tickets, ticket events, users, organizations, groups and the rest of the verified source's list; Chat and Talk not offered); GA4: the reports the user defines; ServiceNow: any table by name, with incident, change request, problem, user and CMDB item suggested; Workday: each custom report the user names; Confluence: spaces, pages, blog posts, comments; Dynamics 365: any Dataverse table; NetSuite: any record type SuiteQL can read; SharePoint: the lists of the sites the user picks. | 2026-10-19 |
| 6 | Incremental loads. | Every object whose API filters by a "last modified" value loads incrementally (merge on the vendor's id); the others are re-read in full each run, and the connector page says which and why. HubSpot is made incremental in our copy. | 2026-10-19 |
| 7 | Records deleted at the vendor. | Applied only where the vendor reports deletes in the same read (Salesforce `IsDeleted`, Graph and Dataverse delta, Zendesk deleted status). Elsewhere a deleted record stays until a full reload, and the page says so. | 2026-10-19 |
| 8 | Following a vendor's "next page" link and Salesforce's returned host (the REST adapter refuses both today, rule Z5). | Followed only when the link points to the same host as the request it came from, or for Salesforce, when the returned host ends in `.salesforce.com` or `.force.com`; every host is checked against the internal-address rules (`SEC-15`) before it is dialled. Anything else stops the run with a plain message. | 2026-10-19 |
| 9 | Schema changes. | Found before the load where the vendor describes its fields (Salesforce, HubSpot, ServiceNow, Dynamics 365, NetSuite), so "ask first" and "pause" work for them; after the load for the others, as for other API sources today. | 2026-10-19 |
| 10 | A connector we cannot test on a real account (Workday and NetSuite have no free developer account). | Built and proven on the gate's mock, but listed as **not supported**, with the reason, until its acceptance row passes on a real account. Same for any vendor whose real-account row is not run. | 2026-10-19 |
| 11 | Who provides the vendor test accounts for the acceptance checklist. | The product owner arranges them: Salesforce Developer Edition, HubSpot developer account, Zendesk trial, a GA4 property, ServiceNow developer instance, Atlassian free site, Power Apps Developer Plan; Microsoft 365, Workday and NetSuite through a partner or a customer. | 2026-10-19 |
| 12 | Build order (one pull request for the phase). | Shared parts first (the SaaS adapter, link rule, incremental and column choice for SaaS, the gate), then HubSpot, Zendesk, Salesforce, GA4, ServiceNow, Confluence, SharePoint, Dynamics 365, Workday, NetSuite. | 2026-10-19 |
| 13 | Rate limits. | Honour the vendor's `Retry-After` and back off; a run that is still limited after the `SRC-11` retries fails with "the vendor limited the rate", never partial data reported as complete. | 2026-10-19 |
| 14 | A no-code builder for custom connectors (the spec's proposed drop; also in `DEC-10`). | Dropped from `SRC-3`. | 2026-10-19 |
| 15 | The gate. | One new CI job with a mock server per vendor that behaves like the vendor (paging, sign-in, rate limit, a changed field), all ten in it, within the 20 minutes signed for connector gates. | 2026-10-19 |

## Spec targets this feature does not meet

Reported, not lowered (`AGENTS.md`, step 0):

- **Second wave** (the spec's "match Databricks' beta list, about 45, by
  customer demand"): not in this pull request. Each beta connector becomes
  its own backlog item when a customer asks for it.
- **Incremental loads for every connector**: Workday Reports cannot filter a
  report by a change date (only by prompts the customer's admin defines), so
  a Workday report is re-read in full each run. Other vendors' objects
  without a "last modified" filter are listed on the connector page
  (decision 6).
- **Real-account proof for Workday and NetSuite** (`SRC-3-AC1`): no free
  developer account exists; they stay "not supported" until a partner or
  customer account is used (decision 10).

## Limits to tell a customer

To be confirmed when the plan is built. Expected from the defaults:

- Google Analytics loads reports you define, not raw events.
- Records deleted at the vendor are marked or removed only for Salesforce,
  Zendesk, SharePoint and Dynamics 365. A deleted Zendesk ticket is not removed
  from Bronze: it stays, and its row says `status = deleted` (Zendesk keeps it in
  its export with that status, scrubs its content after 30 days and drops it from
  exports 90 days after a permanent deletion). HubSpot reports no deletes in the
  same read, so a record archived there stays until a full reload.
- Salesforce: an object with so many fields that its query is longer than about
  12,000 characters (Salesforce refuses a request address over 16,384 bytes) fails with a
  fixed message and loads nothing; leaving columns out does not shorten the query. The
  connector reads Salesforce REST API version 62.0; a move to another version is a reviewed
  change. A record removed from the recycle bin is not reported as deleted, and a used-up
  daily API limit fails the run as "the vendor limited the rate" without retrying it the
  same day.
- Google Analytics: a report's last three days are read again on every run, because Google
  may still change them for 24 to 48 hours; the dates are in the property's reporting time
  zone.
- Some objects are re-read in full each run; the connector page names them.
- Workday and NetSuite stay "not supported" until tested on a real account.
- SharePoint and Confluence files and attachments are not loaded.

## Acceptance checklist

Run on a real deployment with real vendor accounts (decision 11). Mark each
Pass, Fail, or Not run with the reason. A step not performed is never Pass.
Steps needing a terminal are marked (operator). Rows 1 to 10 are run once
per vendor; record the vendor with the result.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `SRC-3-AC1` | For each of the ten vendors: add a connector, sign in (or enter the integration credential), run the connection test | The test passes and names the account it reached | Not run |
| `SRC-3-AC2` | Open the connector's object list | The vendor's objects are listed by the vendor (custom objects included for Salesforce) | Not run |
| `SRC-3-AC3` | Choose two objects and leave one column out of one; run a load | Both tables appear in Bronze with the vendor's rows; the left-out column is absent | Not run |
| `SRC-3-AC4` | Change one record and add one at the vendor; run again | Only the changed and new records are read for incremental objects (the run says how many); the table has the change and no duplicate | Not run |
| `SRC-3-AC5` | Read the connector page | It names which objects load incrementally and which are re-read in full | Not run |
| `SRC-3-AC6` | Add a field to an object at the vendor; run again | The connector's schema-change rule applies and the change is shown | Not run |
| `SRC-3-AC7` | Revoke the app's access at the vendor; run again | The run fails with a plain message to sign in again; no vendor text | Not run |
| `SRC-3-AC8` | Sign in as a user without `connector:manage` and try to add or change one | Refused; the page says access is missing | Not run |
| `SRC-3-AC9` | Pause the connector | No run starts until it is resumed | Not run |
| `SRC-3-AC10` | Look at Workday and NetSuite in the picker before their real-account row passes | Listed as not supported, with the reason | Not run |
| `SRC-3-AC11` | (operator) Open the CI run of the pull request | The SaaS gate passed for all ten | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
