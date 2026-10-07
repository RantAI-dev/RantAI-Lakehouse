# RantAI Lakehouse — Product

The one product document. Read this to know what the product is, what it
has, what it lacks, and what comes next.

Last updated 2026-10-07, against commit `ee0251e`. Vision confirmed with the
product owner on 2026-10-02. Statuses come from reading the repository, not
from testing a running system; anything unconfirmed says *Not verified*.

On 2026-10-07 the Dashboards, Data and Query Studio rows were re-graded
under the product owner's strict rule: **Have** means we match or beat the
best competitor; anything less is **Partial**. The full matrices (148 BI
rows, 69 Data rows, 29 Query Studio rows) are in
`reference/competitive-comparison.md`; every task they produced has a spec
with target numbers in [`specs/`](specs/).

---

## 1. What it is and who it is for

**RantAI Lakehouse is a lakehouse with business intelligence built in, and
an agentic-first way of working.** One product where a company would
otherwise buy a data platform and a separate BI tool.

### Why someone buys it

1. **A capable lakehouse.** Load data from their systems, store it in open
   tables, transform it, query it fast, control who sees what.
2. **BI included.** Dashboards, charts, sharing, embedding and alerts are
   part of the product. No separate Tableau or Power BI licence.
3. **Easy, because it is agentic-first.** A user asks the assistant to
   answer a question, build a chart or run a pipeline, and it does the work
   with the same tools the console has. Risky actions wait for a person's
   approval.

Running on the customer's own servers is an option that matters to some
buyers. It is not the main reason to buy.

### Who it is for

Companies that would otherwise use Snowflake, Databricks or Microsoft
Fabric together with a BI tool, and want one simpler product. The aim is
that such a company could replace that setup with ours.

Mid-sized workloads first. The product runs on one server today; modest
multi-server operation is a goal (section 5).

### What we measure ourselves against

| Purchase we replace | Competitors |
| --- | --- |
| Data platform | Snowflake, Databricks, Microsoft Fabric |
| BI tool | Tableau, Power BI, Metabase |

### What "agentic-first" means here

A working rule, not a hard gate: most things a user can do by clicking, they
should also be able to do by asking the assistant. When a feature is built,
we decide whether the assistant gets a tool for it, and say so if not.
Destructive actions always go through approval.

AI can use an external model provider or a self-hosted one. Scheduled
"digital employees" are parked.

### Business

Open source under AGPL-3.0. An enterprise edition is planned later; a hosted
cloud version is possible in future. Neither exists today.

### Stage

Version 0.1, in heavy development, built by one product owner working with
AI agents (`AGENTS.md`).

---

## 2. What is in it

The modules are the groups in the console's navigation
(`src/components/app-shell/nav-config.ts`).

| Module | What a user does there | State |
| --- | --- | --- |
| **Home** | Starts work: ask, instruct, see what needs attention | Built |
| **History (Ask AI)** | Returns to past conversations with the assistant, which answers questions and performs actions; a conversation starts on Home (PR #68) | Built; needs a model endpoint |
| **Dashboards** | Builds and views dashboards; filters, drills, shares, embeds | Built; behind Metabase on filters, calculations, formatting and delivery (BI matrix: 39 have, 39 partial, 68 missing) |
| **Data** | Browses the catalog and assets; registers sources; uploads a file; switches a Gold table's open-format publishing on or off from its page | Built; publishing (PR #64) and file upload (PR #71) are merged but not yet accepted |
| **Query Studio** | Writes SQL or asks in plain language; saves queries; downloads results | Built; in the Data menu today. A main module of its own, shared by BI and the lakehouse (product owner, 2026-10-07). Matrix: 4 have, 11 partial, 11 missing |
| **Build** | Creates, schedules and operates pipelines | Built |
| **Governance** | Sets policies, classifies and masks data, checks quality, views lineage, approves AI actions | Built; policies and lineage are partial |
| **Monitoring** | Checks health, alerts, audit log, workloads, services, capacity, table maintenance, ingestion | Built; some pages partial |
| **Intelligence** | Digital employees and their runs | Parked; shown as "Soon" |
| **Administration** | Manages users, roles, tenants, service identities, sign-in | Built |

"Built" means the page exists and is backed by the real API. It does not
mean accepted: no feature has been formally accepted by the product owner
yet. That is the first blocker in section 4.

### Coverage against competitors

The capabilities a buyer coming from a competitor would expect, and where we
stand. This table is the source for the backlog.

**Ours:** Have, Partial, Missing, Not verified.
**Competitors** is desk research (sources in
`reference/competitive-comparison.md` and at the end of this file), not
hands-on testing.

#### Getting data in

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Connect to common databases | **Partial.** PostgreSQL, MySQL, SQL Server and MongoDB load end to end (gate `ops/g6`). "Test connection" really connects for PostgreSQL, MySQL, SQL Server, REST and S3 storage; other types say unsupported | Yes, all three | Oracle, MariaDB, Teradata (`SRC-4`) |
| Files and APIs | **Have.** CSV files and REST sources (gate `ops/g6`). Adapters for SFTP, Google Sheets and Oracle exist; *Not verified* end to end | Yes | Verify the three |
| Business-app connectors (CRM, ads, support tools) | **Missing** | Yes: Databricks has 10 generally available plus about 45 in beta; Airbyte 602 | Large (`SRC-3`) |
| Live change capture | **Partial.** Proven for PostgreSQL (gate `ops/g4`); MySQL and SQL Server exist but are not gate-tested | Yes: PostgreSQL, MySQL, SQL Server, Oracle; MongoDB at Airbyte | MySQL, SQL Server, Oracle, MongoDB (`SRC-4`) |
| Message queues | **Partial.** Kafka loads into tables; no processing over streams | Yes, with stream processing | Stream processing |
| Upload a file from the console | **Partial.** One CSV or TSV file of up to 50 MB, every column stored as text; merged (PR #71), not accepted | Yes: Databricks 10 files up to 2 GB with types; Snowflake 250 files of 250 MB | Formats, size, types (`DATA-17`, `DATA-18`) |

#### Storing data

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Open table format | **Partial.** Raw data is Iceberg. Curated (Gold) data is in the engine's own format; an Iceberg copy is kept automatically for tables switched on (merged, not yet accepted) | Yes; most keep one open copy as the source of truth | Open-first storage, later |
| Query a table as it was at an earlier time | **Partial.** A version picker exists (PR #71); no test yet proves masking applies to past versions | Yes (Snowflake Time Travel and equivalents) | Prove it safe (`DATA-16`) |
| Instant copies of a table for testing | **Missing** | Yes (zero-copy clone, branching) | Yes |
| Share data with another organisation | **Missing** | Yes (built-in sharing) | Yes |
| Retention and cleanup of old versions | **Missing** for raw tables; small-file cleanup is optional | Yes, automatic | Yes |

#### Transforming data

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Pipelines with schedules, dependencies, retries, alerts, version history | **Have** | Yes | — |
| Transformations authored in the console | **Partial.** Five operations: dedupe, filter, rename, cast, select. Joins and aggregations are written as SQL outside the builder | Yes: full SQL, visual and code | Richer transformations |
| Tables that refresh themselves when their inputs change | **Missing.** Not found; pipelines that run after their upstreams are the nearest thing | Yes (Dynamic Tables, materialized views) | Yes |
| Notebooks, Python, Spark | **Missing** | Yes, all three | Large; decide if wanted |
| Works with dbt | **Missing** | Yes | Decide if wanted |

#### Querying

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| SQL editor, saved queries, result download | **Partial.** Autocomplete knows table names only; 2,000 rows shown; saved queries cannot be edited or shared; downloads are a browser CSV | Yes: Databricks shows 64,000 rows and downloads up to 5 GB | `QS-1` to `QS-4` |
| Ask in plain language, get SQL | **Partial.** The chat's SQL runs inside the user's permissions with masking and row filters. It learns what a table means from the source's registry and from drafted descriptions a person can correct. | Yes, inside the user's permissions | `AI-16` |
| Scale out across servers | **Missing.** One server | Yes: elastic | Modest multi-server is a goal |
| Query other systems in place, without loading | **Missing** | Yes | Yes |

#### Governance

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Roles and permissions | **Have** | Yes | — |
| Classify and mask sensitive columns | **Have.** Proven end to end (gate `ops/g8`) | Yes | — |
| Row-level rules | **Partial.** The policy engine applies row filters and masks on read; masking is proven end to end (gate `ops/g8`), row filters are not gate-tested | Yes | Prove row filters |
| Catalog and search | **Partial.** Search matches table names only; no certification | Yes: columns, tags, certification | `DATA-11`, `DATA-12` |
| Lineage | **Partial** | Yes, including outside systems | Depth |
| Data quality checks | **Partial.** Hand-written rules; freshness against a set target | Yes: ready check libraries, anomaly detection | `DATA-14` |
| Audit trail | **Partial.** Pipeline runs and assistant actions; not every console change | Yes | Full coverage |
| Single sign-on | **Partial.** The sign-in start, callback and provider-list routes exist in the API; not tested against a real identity provider. `README.md` still describes this as unbuilt | Yes | Test it |

#### Business intelligence

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Dashboards with filters, cross-filter and drill | **Partial.** One year filter; no relative dates, number, text or linked filters; drill-down missing for nine chart types | Yes, every plan | `BI-18`, `BI-9` |
| Calculated fields | **Missing** | Yes: Metabase about 100 functions | `BI-8` |
| Chart types | **Partial.** About two dozen, including maps, sankey, sunburst, box plot, calendar; no pivot table, KPI comparison, raw-row table or Gantt | Yes | `BI-16` |
| Chart formatting | **Partial.** The saved number format is ignored; one palette | Yes | `BI-17` |
| Share by link and embed in other sites | **Partial.** Public links never expire; embed tokens can lack an expiry | Yes; often a paid extra | `BI-19`, `BI-26`, `SEC-12` |
| Alerts on a threshold | **Partial.** Email and webhook; one aggregate per table, re-fires every check | Yes | `BI-19` |
| Scheduled reports with PDF or spreadsheet attached | **Missing.** Text digests only | Yes, all three BI tools | `RPT-1` |
| Export to PDF or spreadsheet | **Partial.** Browser print; CSV | Yes | `BI-1` |
| Shared business definitions (a governed metrics layer) | **Partial.** Built-in dashboard cards are defined once in code; users cannot define and reuse their own metrics | Yes, all three BI tools | User-defined metrics |
| Mobile app | **Missing** | Tableau and Power BI | Decide if wanted |
| Import dashboards from another BI tool | **Missing** | Databricks imports Tableau and Power BI | Helps switching |

#### AI

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Assistant that performs actions, not only answers | **Have** | Yes, recently | — |
| Human approval before a risky AI action | **Have**, built in | Varies | A strength |
| Assistant builds charts and dashboards | **Partial.** Builds them; cannot rename, arrange or delete them | Yes | `AI-1`, then one assistant item per BI item |
| Assistant aware of the page you are on | **Have** | Yes | — |
| AI functions inside SQL | **Missing** | Yes | Decide if wanted |
| Search over documents | **Missing** | Yes | Yes |
| Build and run your own agents | **Parked** (digital employees) | Yes | Parked on purpose |
| Train and serve models | **Missing** | Databricks, Fabric | Likely out of scope |

#### Running it

| Capability | Ours | Competitors | Gap |
| --- | --- | --- | --- |
| Hosted cloud service | **Missing** | Yes; it is their main form | Possible later |
| Self-hosted | **Have** | Mostly no | A strength |
| High availability | **Missing** | Yes | Yes |
| Backup and restore | **Partial.** Procedures exist; no tested restore time | Yes, managed | Test it |
| Usage and cost view | **Missing.** No usage page or API route | Yes | Yes |
| API for everything | **Have** | Yes | — |
| Several organisations in one install | **Partial.** One organisation's data per install | Yes | Later |

### Where we are ahead

- BI is included in the same product and login.
- Approval of AI actions is built in.
- It can run on the customer's own servers.
- It shows "Not measured" instead of inventing a number, and says
  "unsupported" instead of pretending.

### Where the biggest gaps are

1. Open security findings in the Data module and Query Studio (section 3).
2. Dashboard depth: filters, calculated fields, formatting, scheduled
   reports.
3. Business-app connectors.
4. Query Studio basics: autocomplete, results, saved queries.
5. Scale beyond one server.
6. Richer transformations in the console (joins, aggregations).
7. Notebooks and Python, if we decide we want them.

---

## 3. What it does not do

Stated plainly, for anyone describing the product to a customer. More detail
in `README.md` ("Status / Known limitations").

**Data**
- Curated tables are in open format only when publishing is switched on for
  that table; it is off by default. A published table holds every past
  copy; outside readers must pick the latest. Tables over 5,000,000 rows
  are refused. A background merge in the engine can cause one extra copy.
- Raw table history is never trimmed.
- A plain row count on a change-captured table can overcount on some engine
  versions. The product's own queries avoid this; a hand-written one may not.

**Queries and dashboards**
- One engine; no querying other systems in place. Querying a table as it
  was earlier works from a version picker, but no test yet proves masking
  applies to past versions (`DATA-16`).
- File upload takes one CSV or TSV file of up to 50 MB, and every column is
  stored as text (merged, not accepted).
- Dashboards filter by year only; there are no calculated fields.
- Query Studio shows at most 2,000 rows. A chart built on a saved query is
  limited to 2,000 rows and 30 seconds.
- No scheduled reports with attachments. "Export PDF" is the browser's print.

**AI**
- Needs a model endpoint. Works over tables, not documents.
- Digital employees are not available.

**Security**
- Login rate limiting and session cleanup are merged (PR #67), not yet
  accepted. Single sign-on exists in the API but has not been tested
  against a real identity provider.
- Open findings from the 2026-10-07 code audits, fixed before feature work:
  a connector's stored password can be sent to a new host (`SEC-14`);
  connection tests and alert webhooks can reach internal addresses
  (`SEC-15`, `SEC-10`); raw database errors reach the screen on dashboard
  tiles and embeds (`SEC-11`); queries are not scoped to the tenant
  (`SEC-20`). The full list is `SEC-10` to `SEC-21` in `BACKLOG.md`.
- The audit trail does not cover every console change.
- The backend was ported and reviewed by AI agents, with no full human
  security review. A previously internal key is in the public git history
  (`SECURITY.md`).

**Running it**
- One server. No high availability.
- If the application database is unreachable the service still starts, and
  affected pages return errors.
- Workspace settings cannot be changed.

Earlier documents (`PRODUCT_SPECS.md`, `docs/FEATURE_COVERAGE.md`,
`docs/UX_FLOWS.md`, the sales playbook) describe an older product and
disagree with the build in places. Where they differ from this file, trust
this file and the code.

---

## 4. What blocks production

"Ready for production" means: it can be installed for a second customer and
run without the product owner fixing things by hand.

| # | Blocker | What clears it |
| --- | --- | --- |
| 1 | No feature has been accepted. "Built" means merged, not checked by a person | Run the acceptance checklist for each feature, starting with what a demo shows |
| 2 | A leaked key is in the public git history | Confirm it was rotated; decide whether to rewrite history |
| 3 | No human security review | Commission one |
| 4 | The main branch can be changed without review | Apply the settings drafted in `docs/CI.md` |
| 5 | No support commitment to put in a contract | Decide one (`reference/support-model.md`) |
| 6 | Open high-severity security findings: connector password theft (`SEC-14`), connection tests reaching internal addresses (`SEC-15`) | Fix the security items under Now in `BACKLOG.md`; their specs are in `specs/` |

Not blockers, but will be found in a customer's evaluation: full audit
coverage, single sign-on, tested backup restore, cleanup of old table
versions.

### Risks worth knowing

| Risk | Why it matters |
| --- | --- |
| We claim something that fails in a customer's test | Costs more than the claim was worth. Section 3 exists to prevent it |
| A wrong number is shown without an error | The most damaging kind of failure for a data product |
| Everything depends on one person and AI agents | Decisions must be written down; that is what this folder is for |
| Chasing every competitor feature | We cannot match three large platforms. Section 5 picks |

---

## 5. What is next

Proposed order. The product owner decides. No dates: nothing is estimated.

The direction: **be a credible replacement for a data platform plus a BI
tool, for a mid-sized company.** That means closing the gaps a switching
customer would hit first, not matching every feature.

### Now

| Item | Why |
| --- | --- |
| Accept the merged features: Gold publishing (`DATA-1`), login protection (`SEC-2`, `SEC-5`), file upload (`DATA-9`) | Blocker 1; merged is not accepted |
| Fix the security findings under Now: `SEC-10`, `SEC-11`, `SEC-13`, `SEC-14`, `SEC-15` | Blocker 6; they come before feature work |
| Clear blockers 2 and 4 | Small, and they are embarrassing if found |

### Next

One task list per module, each task with a spec in [`specs/`](specs/) and
an order in its roadmap phase.

| Module | Item | Why |
| --- | --- | --- |
| Dashboards | Filters, time grain, calculated fields, tables and KPIs first (`BI-18`, `BI-9`, `BI-8`, `BI-16`); then the builder, joins, metrics, formatting, layout, reports and sharing | The gaps a Metabase user hits on day one |
| Assistant for dashboards | `AI-1`, `AI-12`, `AI-15` now; each other `AI-` item ships with its BI item | Agentic-first: what a user can click, they can ask for (`AGENTS.md`) |
| Data | Fix and test every connector (`SRC-6`, `SRC-9`), failure alerts and schema changes (`SRC-7`, `SRC-8`), more databases with live change capture (`SRC-4`), then the catalog (`DATA-11` to `DATA-16`) and upload depth (`DATA-17` to `DATA-19`) | Make what we advertise work before adding more |
| Query Studio | Its security fixes (`SEC-19` to `SEC-21`), then editor, results, saved queries and history (`QS-1` to `QS-4`) | Every SQL user expects these |
| Security and platform | `SEC-12`, `SEC-16` to `SEC-18`, single sign-on (`SEC-3`), audit coverage (`GOV-1`), row filters (`VER-1`), table cleanup (`OPS-1`), usage view (`OPS-8`) | Found in every evaluation |
| Pipelines | Joins and aggregations in the builder (`BLD-1`) | Today these need SQL written outside the builder |

### Later

| Item | Why later |
| --- | --- |
| Run on several servers | A stated goal; larger work |
| Business-app connectors (`SRC-3`): Databricks' ten generally available ones first | Large; waits for the reuse decision `DEC-9` |
| Parameters, snippets and charts from SQL results (`QS-5`) | The bridge from Query Studio to dashboards, after the basics |
| Instant copies of a table; self-refreshing tables | Expected by Snowflake users |
| Open-first Gold: store curated data in Iceberg, serve from a fast copy | Removes publishing entirely; needs a fresh measurement |
| Import dashboards from other BI tools | Eases switching |
| Search over documents | Needs a vector store |
| Hosted cloud version; enterprise edition | Business decisions |

### Decide whether we want these at all

Notebooks and Python, dbt support, AI functions in SQL, model training, a
mobile app, data sharing between organisations. Competitors have them. Each
is large. Saying no to some is how the product stays simple.

---

## 6. Decisions waiting on the product owner

| # | Decision | Recommendation |
| --- | --- | --- |
| 1 | Which role may publish a table in open format | Platform admins only for now. Today the Data Engineer role cannot |
| 2 | What switching publishing off does | Stop updating; keep the table |
| 3 | The "decide whether we want these" list in section 5 | Say no to model training and mobile app; defer the rest |
| 4 | Which competitor is the one to beat | Per module, as the specs already use: Metabase for dashboards (Tableau as the ceiling), Databricks for Data and Query Studio, Airbyte for connectors |
| 5 | What happens to the old documents | Archive `PRODUCT_SPECS.md` and `docs/UX_FLOWS.md`; rewrite `docs/FEATURE_COVERAGE.md` from section 2 |
| 6 | Rewrite the sales playbook around the new positioning | Yes; it leads with on-premises, which is no longer the main message |
| 7 | Which assistant actions need human approval (`DEC-8`) | Also making a dashboard public or embeddable, changing who can see it, and email outside the company |
| 8 | How to get business-app connectors (`DEC-9`) | Reuse dlt verified sources first; our loader already runs on dlt |
| 9 | Data and Query Studio edge cases (`DEC-10`) | Drop or defer all six |
| 10 | The four login defaults (5 failures in 15 minutes, 5-minute lock, 30-day retention, no off switch) | Keep them |
| 11 | Every target marked *(proposed)* in `specs/` | Confirm or change on each feature page before its plan is written |

Decided on 2026-10-06 and 2026-10-07: the BI scope (write-back, curated
datasets, data preparation, sets, PowerPoint export and automatic dashboards
rejected; see `reference/competitive-comparison.md`), Query Studio as its
own module, and the strict Have rule above.

---

## Sources for the competitor columns

- [Snowflake Complete Guide 2026](https://sqlyard.com/2026/04/24/snowflake-complete-guide-2026-architecture-pipelines-cortex-ai-and-genai-in-production/)
- [Snowflake Data Engineering in 2026](https://sparkeighteen.com/blog/snowflake-data-engineering-in-2026-the-platform-the-pipeline-and-the-governance-layer/)
- [Everything Databricks announced at Data + AI Summit 2026](https://qubika.com/blog/everything-databricks-announced-dais-data-ai-summit-2026/)
- [Lakeflow Connect](https://www.databricks.com/blog/unify-your-marketing-data-lakeflow-connect)
- [What is Microsoft Fabric](https://learn.microsoft.com/en-us/fabric/fundamentals/microsoft-fabric-overview)
- [Microsoft Fabric enterprise guide 2026](https://www.epcgroup.net/microsoft-fabric-enterprise-guide-2026)
- [Power BI vs Tableau vs Metabase (2026)](https://valiotti.com/powerbi-vs-tableau-vs-metabase/)
- [Power BI vs Metabase (2026)](https://ecosire.com/blog/power-bi-vs-metabase-comparison)
