# User guide

How to do the everyday jobs in the RantAI Lakehouse console, step by step.

**How these steps were checked.** Every button, tab and message quoted
here was read from the console's source at commit `fdf357ce`
(2026-10-31). They have not yet been walked through on a running console;
that pass is the rest of backlog item `DOC-2`. If a step does not match
what you see, the console is right and this page is wrong. Please report it.

Words in **bold** are the exact labels you will see. A button you lack the
permission for is either hidden or greyed out, and hovering it usually says
why. Your administrator grants permissions through roles.

## Contents

1. [Sign in](#1-sign-in)
2. [Home and the assistant](#2-home-and-the-assistant)
3. [Bring data in](#3-bring-data-in)
4. [Find and query data](#4-find-and-query-data)
5. [Build pipelines](#5-build-pipelines)
6. [Dashboards](#6-dashboards)
7. [Governance](#7-governance)
8. [Monitoring](#8-monitoring)
9. [Administration](#9-administration)
10. [What the console will always tell you](#10-what-the-console-will-always-tell-you)

The left navigation, top to bottom: Home, History, Dashboards, Data,
Build, Governance, Monitoring, Intelligence, Administration.

## 1. Sign in

1. Open the console. The page reads **Sign in to Rantai Lake**.
2. Enter your **Email** and **Password**, then click **Sign in**.
   - A wrong email or password shows *Invalid email or password.*
   - *Caps Lock is on* appears if it is.
3. If your organisation uses single sign-on, click **Continue with SSO**
   instead. This button only appears when your operator has switched SSO on.
4. If you were given a temporary password, the console asks you to
   *Choose a new one to continue.* Fill **Current password**, **New
   password** (at least 8 characters) and **Confirm new password**, then
   click **Update password**.

**To sign out:** click the person icon at the top right, then **Sign out**.
The same menu has **Change password** and **Theme** (Light, Dark, System).

There is no default administrator account. Your operator creates the first
one at installation.

## 2. Home and the assistant

### Home

Home shows a greeting, a status line, the assistant's message box, and
quick-start buttons: **Connect a source**, **Create a pipeline**, **Build a
dashboard**, **Write a query** and **Browse the catalog**. The sparkle icon
on each one (*Let AI do it*) hands the job to the assistant.

- The status line reads **All clear**, or lists what needs attention, each
  with **Details** and **Ask AI**.
- Further down are cards: **Recent** and **Pipeline runs**. Click
  **Customize** to add **Sources**, **Open alerts** or **Saved queries**,
  or to hide cards. Finish with **Done**, or undo with **Reset to default**.

### Ask the assistant

1. Choose a mode:
   - **Ask** answers questions and changes nothing.
   - **Build** can create and change things, and asks you first.
2. Optional: open **Tools** to choose what the assistant may use (for
   example **Query Data**, **Dashboard Builder**, **Pipeline Builder**).
3. Type your question in the box and press Enter. The conversation opens
   on its own page.

What happens when the assistant wants to change something:

- **A normal change** shows a card, **Action requires confirmation**, with
  a summary. Click **Confirm** to carry it out, or **Cancel**.
- **A chart draft** shows **Add to dashboard**, **Edit in builder** and
  **Discard**.
- **A risky change** is not carried out. The card says *This action
  requires approval.* and links to **View in Approvals →**, where a person
  decides ([section 7](#approvals)). Every report delete, and a scheduled
  email report sent to anyone but you, waits there.

The assistant only ever has your own permissions. It can be wrong; the
page says so at the bottom. It works over tables, not over documents. It
needs a language model your operator has set up.

### History

**History** in the navigation lists your conversations. Only you can see
them.

- Search with the box, or filter with the **All**, **Ask** and **Build**
  tabs. Click a row to continue that conversation.
- The **⋯** menu on a row has **Rename** and **Delete**. A deleted
  conversation cannot be brought back.
- **New chat** starts a fresh conversation.

## 3. Bring data in

**Sources** in the navigation opens the **Connectors** page. It has two
tabs: **Connectors** and **Uploaded files**.

### Connect a system

1. Click **New Connector**. The form has four steps: **Type**,
   **Connection**, **Tenant and residency** and **Review**. **Next** stays
   greyed out until the step is complete.
2. **Type:** choose from **Databases**, **Change data capture**, **Files
   and storage**, **APIs and streams** or **Business apps**. A type that is
   not ready yet is greyed out; hover it to see why.
3. **Connection:** give it a **Name**, choose the **Direction**, and fill
   **Connection details** and **Credential**.
4. Click **Create connector**. The console tests the connection straight
   away and says one of three things:
   - *Connection test passed*;
   - *Connection test failed*, with the reason;
   - *This connector type cannot be tested yet*.

   Click **Run test again** after fixing a problem.

### Choose what to load, and when

Open the connector, then the **Ingest** tab.

1. Under **Tables to ingest**, pick the tables and a load mode for each:

   | Load mode | What it does |
   | --- | --- |
   | **Replace all rows** | Each run replaces the table. |
   | **Add only new rows** | Adds rows newer than the last run (SQL sources and business apps only). |
   | **Add all rows again** | Appends everything each run. |
   | **Replace all rows, one per key** | Replaces, keeping one row per key. |
   | **Add only new rows, one per key** | Adds new rows, keeping one row per key. |
   | **Keep history** | Keeps every version of each row. |

2. Under **Schedule**, choose **Manual only**, **Every hour**, **Every day
   at 02:00 UTC** or **Custom cron…**. Change-data-capture connectors run
   continuously and have no schedule.
3. Click **Save tables and schedule**.
4. Click **Run now** to load straight away. It is greyed out until you
   have saved at least one table.

Each table's menu also has **Refresh, keeping rows**, **Refresh, removing
rows** and **Clear**.

The connector's other tabs: **Overview**, **Schema changes** and
**Connection tests**. **Delete** stays greyed out while a pipeline still
uses the connector.

### When the source's columns change

On the **Schema changes** tab, set **When the source changes** to:
- **Apply non-breaking automatically**;
- **Apply all**;
- **Ask first**;
- **Pause**.

Then click **Save policy**. A change that is waiting for you appears under
**Waiting for a decision**, with **Approve and resume** and **Dismiss**.

### Upload a file

1. On the Connectors page, click **Upload file**.
2. Drop files on the page, or click **Choose files**.
   - Accepted: CSV, TSV, JSON, JSON Lines, Avro, .xls, .xlsx, .parquet,
     .gz and .zip.
   - Not accepted: .xlsm, .xlsb, .ods, .tar, .7z.
   - Up to 10 files at once, and 2 GB in all. An Excel workbook may be up
     to 50 MB, and a .zip may hold up to 10 files.
3. Go through **File**, **Check**, **Table** and **Review**, then click
   **Load**.

## 4. Find and query data

### Catalog

**Catalog** lists the namespaces (groups of tables).

1. Search with **Search namespaces…**.
2. Click a row to see its owner, residency and assets.
3. Click **Browse in Data Explorer** to see its tables.

### Data Explorer

1. Pick a layer:

   | Layer | What it holds |
   | --- | --- |
   | **Bronze (raw)** | Data as it arrived from the source. |
   | **Bronze (curated)** | Raw tables your deployment marks as curated. |
   | **Silver** | Cleaned, row-level data. |
   | **Gold** | Curated tables ready for dashboards. |

   Or pick **All**.
2. Search by name, column, tag, description or owner.
3. Click a row to open the table. Its tabs:

   | Tab | What it shows |
   | --- | --- |
   | **Overview** | Owner, description, and for a Gold table the **Publish in open format (Iceberg)** card ([section 5](#publish-a-gold-table-for-other-tools)). |
   | **Schema** | The columns, and **Schema versions**. |
   | **Sample** | Up to 1,000 rows, and **Open in Query Studio**. |
   | **Quality** | The checks on this table; **Run checks** and **Add rule**. |
   | **Access** | **Your access**, its **Classification** and its **Policies** ([section 7](#hide-sensitive-columns)). |
   | **Lineage** | Where the data came from and what depends on it. |
   | **Activity** | **Snapshots**, **Change history** and **Freshness**. |

If you lack access to a table, a **Request access** button appears. The
request goes to Approvals.

**See the table as it was:** on the **Activity** tab, click **Query this
version** next to a snapshot.

### Query Studio

Query Studio has two modes: **SQL** and **Natural language**.

**Write SQL:**
1. Choose the engine, **ClickHouse** or **Trino**.
2. Write your query and click **Run query** (Ctrl+Enter or ⌘+Enter). With
   several statements, click **Run all**. **Stop** cancels.
3. **Explain** shows the query plan. **Format** tidies the SQL.

Queries only read data. When a result is cut short, a banner says so and
gives the limit: either a number of rows, or 10 MB.

**Ask in plain language:**
1. Type a question, for example *What was revenue by region last quarter?*
2. Click **Ask (agentic)** to get an answer, or **Generate SQL only** to
   review the SQL before running it.

**Query a past version:** choose **Namespace**, **Table** and **Version**,
then **Query this version**. This works on ClickHouse only.

**Save a query:**
1. Click **Save query**. It is greyed out when the editor holds more than
   one statement.
2. Give it a **Title** and **Tags**, then click **Save**. Saved queries
   are shared with your team, with you as the author.
3. Find them under **Saved Queries**, and open one with **Open in
   Studio**.

Saved queries cannot be scheduled yet; the page says so.

## 5. Build pipelines

A pipeline reads a table, transforms it, and writes the result to another
layer. Pipelines need the orchestrator, which your operator runs.

### Create a pipeline

1. In **Pipelines**, click **Create Pipeline**. Or click **Agentic
   Builder**, describe what you want under **Instruction**, and click
   **Draft pipeline**.
2. **Source:**
   - give it a **Pipeline name**;
   - choose the **Kind** (batch or incremental);
   - choose the **Connector** and **Source table**.

   Two switches make it run by itself: **Run after this connector loads
   the table**, and, for an uploaded table, **Run again after this table
   is re-uploaded**.
3. **Transform:** choose a **Verb** (dedupe, filter, rename, cast or
   select), fill it in, and click **Add transform**. Repeat as needed. A
   pipeline with no transform is a plain copy of its source, and is
   allowed only with **Copy the source as is** switched on.
4. **Target:** choose the **Target zone** and **Target table**.
5. **Schedule:** choose **On demand only**, **Every 15 minutes**,
   **Hourly**, **Daily at 02:00**, **Weekly, Monday 02:00** or **Custom
   cron**. Times are UTC.
6. **Review:** click **Create pipeline**.

### Run it

1. Open the pipeline. A new pipeline is a draft: click **Mark ready**.
2. Click **Run now**. **Pause schedule** and **Resume schedule** stop and
   restart its timetable.

If the orchestrator has not loaded the pipeline yet, the page says so,
and **Run now** asks it to reload first.

### See what happened

The pipeline's **Runs** tab shows **Run history**. Open a run to see:
- the **Step timeline**;
- the **Event log**. Filter it by **Info+**, **Warnings** or **Errors**,
  search it, or download all of it.

**A failed or cancelled run:** click **Re-run**, then choose:
- **From failure** re-runs only the steps that failed or never ran;
- **All steps** re-runs everything.

A run in progress has **Cancel run** instead.

The other tabs are **Flow & code** and **Settings**. **Settings** links to
this pipeline's lineage.

### Keep a table fresh within a target lag

Instead of a fixed schedule, a pipeline can refresh whenever its table
would fall more than a set time behind its source.

1. Open the pipeline. In the **Freshness** panel, click **Configure target
   lag**.
2. Choose **Target lag (minutes)** and enter 1 to 10080. Or choose
   **Downstream** to refresh only when a pipeline that reads this table
   needs it. **None (clear)** removes the target lag.
3. Click **Save target lag**. While a target lag is set, the schedule is
   not used.

To be told when a pipeline falls behind, create a **Pipeline behind target
lag** alert ([section 8](#alerts)).

### Publish a Gold table for other tools

1. Open the Gold table in Data Explorer. On **Overview**, find **Publish
   in open format (Iceberg)** and switch it on.
2. Click **Publish now**. The card lists the last 5 publish runs.

Only a Platform Admin may switch publishing on or off.

## 6. Dashboards

The assistant can build dashboards for you ([section 2](#ask-the-assistant));
everything below can also be done by hand.

### Create a dashboard

1. Go to **Dashboards**, then **Browse all dashboards**.
2. Click **New dashboard**. Fill **Name**, **Description** and **Folder**,
   then click **Create**.

To organise dashboards, click **Folders** (up to 4 levels deep). To move a
dashboard into a folder, use **Move to folder…** in its menu.

### Add a chart

1. On the dashboard, click **New chart**. On an empty dashboard you can
   also click **Ask Copilot**.
2. Under **Data**, choose the **Data source**:
   - a table under **Gold marts**. It can be joined with other Gold
     tables.
   - a saved query under **SQL sources**;
   - **Write custom SQL…**, for people allowed to write SQL. A SQL source
     has a row and time limit, shown on screen, and its records cannot be
     drilled into.
3. Fill **What to show**, then **Sort and filter**, **More options** and
   **Format** as needed.
4. Click **Create chart**.

**Calculated fields:** under **Data**, click **Custom fields**, then
**New calculated field**. Give it a **Name** and a **Formula**, and click
**Create**. The same menu has **New value group** and **New hierarchy**.

**Shared definitions:** under **Browse all dashboards**, click
**Metrics**. A metric is one definition of a number, used by every chart,
KPI and the assistant. **Segments** works the same way for groups of rows.

### Filter and drill

- Click **Add filter** to filter the whole dashboard. **Save as default**
  makes the current filters the dashboard's default.
- Click a value in a chart to get:
  - **Filter dashboard by this**;
  - **Drill down to …** (needs a hierarchy);
  - **Add note…**;
  - **View records**.

### Share

Open the **⋯** menu (**More actions**), then **Share…**. A dashboard in
your personal space cannot be shared: move it to a folder first.

- **Public link:**
  1. Choose when it expires (between 1 hour and 365 days).
  2. Optionally set a password.
  3. Click **Make a public link**. **Switch off now** withdraws it.
- **Signed embedding:** switch it on, then copy a token for your
  application. **Withdraw all embed tokens** revokes them all.

An administrator can turn public links off for everyone, and signed
embedding needs your operator to configure it. The dialog tells you when
either is the case.

### Download and print

- **Whole dashboard:** in the **⋯** menu, choose **Download PDF…**, pick
  the paper size and orientation, then click **Download**. **Print…** uses
  your browser's print dialog and always works.
- **One chart's data:** in the chart's menu, choose **Download data…**,
  then **Excel (.xlsx)** or **CSV**.

### Send it on a schedule

1. In the **⋯** menu, choose **Subscriptions…**, then **New
   subscription**.
2. Choose how often under **Repeat**: every hour, day, week or month.
   Attachments are limited to 10 files and 20 MB in all.
3. Click **Subscribe**.

A subscription that emails anyone but you waits for approval first.
Subscriptions need email to be set up on your deployment, and the dialog
tells you if it is not.

## 7. Governance

### Policies

1. In **Policies**, click **Create Policy**.
2. **Basics:** give it a **Policy name** and choose its **Kind**:
   - **Row filter**;
   - **Column mask**;
   - **Agent autonomy**;
   - **Retention**;
   - **Residency**.
3. **Scope:** fill **Subjects** (who it applies to) and **Resources**
   (what it covers).
4. **Rules:** choose the **Effect**, and add **Conditions** if needed.
5. **Review:** switch on **Activate on create** to enforce it straight
   away, or leave it off to keep a draft. Then click **Create draft** or
   **Create & activate**.

Later, click a policy to **Enforce**, **Stop enforcing** or **Delete** it.

### Hide sensitive columns

Do this on the table itself: open it in Data Explorer and go to its
**Access** tab.

1. **Label the data:** click **Classify**. Choose **Applies to** (one
   column, or the whole table) and a classification (**Public**,
   **Internal**, **Confidential** or **Restricted**). Click **Save**.
2. **Mask it:** click **Add policy**.
   1. Choose the **Roles it applies to**.
   2. Tick the **Columns to mask**.
   3. Optionally add a row filter.
   4. Click **Add and enforce**, or **Save draft**.

People in those roles then see `***` in place of the values.

The **Classification & Masking** page lists labels across all tables, and
**Add Rule** labels a column from there. To make sure a value is hidden,
use the table's **Access** tab as above.

### Tag keys

Tags describe tables with values from a fixed list.

1. In **Tag keys**, click **New tag key**.
2. Fill **Key** (lower-case letters, digits, `_` and `-`, up to 64
   characters) and **Allowed values** (up to 50).
3. Click **Define key**.

A key cannot be renamed. It cannot be deleted while any table still uses
it.

### Data quality

1. In **Data Quality**, click **Add Quality Rule**.
2. Fill **Name** and **Asset**. Then pick a **Check**, or click **Write the
   threshold myself**.
3. Choose the **Dimension** and **Severity**, then click **Create**.

Open a rule to **Run check**, **Edit rule** or **Delete rule**. A failing
check opens an incident, listed under **Open incidents**, with
**Resolve**.

### Lineage

In **Lineage**, type a table name. The page shows its **Graph**,
**Connections**, **Column mappings**, and **What this graph covers**.

### Approvals

Approvals holds the requests waiting for a person:
- risky actions the assistant was asked to do;
- access requests.

1. Click a request to see what it would do.
2. Click **Approve** or **Reject**. Add a comment for the audit trail if
   you like, then confirm.

Approving carries out exactly what was requested. Rejecting carries out
nothing. Either way the decision is recorded. You cannot approve your own
access request: another Governance Admin must decide it.

## 8. Monitoring

### Alerts

1. In **Alerts**, open **Rules & Digests** and click **New rule**.
2. Give it a **Name**, then choose its **Type**:
   - **Threshold alert**: a value crosses a limit.
   - **Dashboard digest**: a dashboard sent on a schedule.
   - **Dataset freshness**: a table has not been updated in time.
   - **Pipeline behind target lag**.
   - Connector and upload problems.
3. Choose **Delivery**: **Webhook (Slack/Discord)** or **Email (SMTP)**.
   Then choose the **Severity**.
4. For a threshold alert, choose what to **Watch**, the **Aggregate**, the
   **Operator** and the **Threshold**.
5. Enter the **Webhook URL** or **Recipient email**, then click
   **Create**.

Use **Test run now** on a rule to try it. **Active Incidents** shows what
has fired.

### Audit Log

The **Audit** page records what happened. Search by actor, action,
resource or tenant, or use **Filter**.

Each entry's outcome is one of:
- **Success**;
- **Denied**: refused or rejected;
- **Error**: it failed;
- **Pending**: a decision still waiting on a person, such as an
  unconfirmed assistant change or an open approval.

It does not yet record every change made in the console.

### The other monitoring pages

| Page | What it shows |
| --- | --- |
| **Health** | Platform health across storage, pipelines, queries, agents and governance. |
| **Activity** | Recent actions from pipelines, queries, schema changes, policies, connectors, agents and approvals. |
| **Workloads** | Running and queued requests, with their cost. |
| **Observability** | Service levels, lag, cache behaviour, policy latency and agent performance. There is no log or trace explorer. |
| **Services** | Whether each component is up, with its version. A component that was never set up shows "unknown", not "down". |
| **Capacity** | Storage per bucket, from a daily snapshot, plus ClickHouse's live disk use. |
| **Table Maintenance** | Each table's upkeep and the nightly maintenance runs. The **Dry run (would delete)** column shows what the next run would remove. A table's own page sets **Snapshots to keep**, **Compact small files** and the **Schedule**. |
| **Ingestion (CDC)** | The health of live change capture from each source database, including how far behind it is. |

## 9. Administration

These pages are for platform administrators.

| Page | What you can do |
| --- | --- |
| **Users** | **Invite User**: give a **Name**, **Email**, **Roles** and **Tenants**, then click **Invite**. Roles cannot yet be changed here once a person is invited. |
| **Teams & Roles** | **Create Role**: give a **Name**, its **Permissions** (for example `catalog:read, query:run`) and a **Description**. |
| **Tenants** | **Create Tenant**: an organisation, with its **Name**, **Slug**, **Plan** and **Residency**. |
| **Service Identities** | **Create Service Identity**: an account for scheduled jobs. Its secret is shown once, when you create a credential. Copy it then. |
| **SSO** | See the single sign-on setup and click **Test sign-in**. Your operator configures it on the server. |
| **Sessions** | See who is signed in, and **Revoke** a session. |
| **Settings** | Report time zone, first day of the week, number format, company palette and fonts, allowed outside sites and email domains, downloads, public links, remembered results, and query limits per role. |
| **Maps** | **Upload a map** (a GeoJSON file of polygons) for map charts. A map that a chart uses cannot be deleted. |

**Intelligence** (Digital Employees, Agent Runs) is marked **Soon** and
cannot be opened yet.

## 10. What the console will always tell you

- A value it does not know reads "Not measured", never zero.
- A function it does not support says "unsupported", or is greyed out
  with the reason.
- A failed action shows as failed, with its reason.

A number you doubt is worth reporting. A wrong figure is treated as the
most serious kind of problem.

## Where to get help

See [support-model.md](support-model.md). For installation and
operation, see `docs/OPERATIONS.md`.
