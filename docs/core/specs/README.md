# Specs, one per task

One file per backlog task: why, what users get, target specs with numbers, the competitor benchmark, an acceptance checklist and what is left out. Targets come from the best competitor's documented numbers; a number marked *(proposed)* is the planner's and is confirmed by the product owner on the feature page.

A spec is not a plan. Before building, a task still needs a feature page in [`../features/`](../features/) and a plan (`AGENTS.md`). The spec files are generated together, so they share one format; edit a spec by hand once its feature page exists.

### Security

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`SEC-9`](sec-9.md) | Plain-language queries respect masking | Now | S |
| [`SEC-10`](sec-10.md) | Alert webhooks cannot reach internal addresses | Now | S |
| [`SEC-11`](sec-11.md) | No raw database errors on screen | Now | S |
| [`SEC-12`](sec-12.md) | Safer embed tokens | Next | M |
| [`SEC-13`](sec-13.md) | Check the staging file pushed to main | Now | S |
| [`SEC-14`](sec-14.md) | Connector passwords cannot be stolen by re-pointing a connector | Now | M |
| [`SEC-15`](sec-15.md) | Connection tests cannot reach internal addresses | Now | M |
| [`SEC-16`](sec-16.md) | No cross-tenant leaks in the Data module | Next | M |
| [`SEC-17`](sec-17.md) | Upload hardening | Next | S |
| [`SEC-18`](sec-18.md) | No default credentials in compose | Next | S |
| [`SEC-19`](sec-19.md) | The query cost estimate runs only safe SQL | Next | S |
| [`SEC-20`](sec-20.md) | Queries are scoped to the tenant | Next | M |
| [`SEC-21`](sec-21.md) | Downloads and big results are safe | Next | S |
| [`SEC-22`](sec-22.md) | Adding a governance rule needs the permission changing one needs | Now | S |

### Dashboards (BI)

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`BI-18`](bi-18.md) | Dashboard filters, interactivity and layout | Next | L, in three parts |
| [`BI-9`](bi-9.md) | Group by day, week, month, quarter, year | Next | M |
| [`BI-8`](bi-8.md) | Calculated fields | Next | L |
| [`BI-16`](bi-16.md) | Chart types | Next | L, in three parts |
| [`BI-6`](bi-6.md) | A more capable chart builder | Next | L |
| [`BI-7`](bi-7.md) | Joins without writing SQL | Next | M |
| [`BI-13`](bi-13.md) | Combine two sources on one chart | Next | M |
| [`BI-10`](bi-10.md) | Bins, groups and hierarchies | Next | M |
| [`BI-11`](bi-11.md) | Parameters | Next | M |
| [`BI-2`](bi-2.md) | Reusable metrics | Next | L |
| [`BI-17`](bi-17.md) | Chart formatting, forecasting and clustering | Next | L, in two parts |
| [`RPT-1`](rpt-1.md) | Scheduled reports with attachments | Next | L |
| [`BI-1`](bi-1.md) | Export PDF and Excel | Next | M |
| [`BI-19`](bi-19.md) | Sharing, alerts and permissions | Next | L |
| [`BI-26`](bi-26.md) | Better embedding, not bigger | Next | M |
| [`BI-4`](bi-4.md) | Caching and parallel tiles | Next | M |
| [`BI-5`](bi-5.md) | Timeouts and limits you can set | Next | S |
| [`BI-14`](bi-14.md) | Lineage from a chart to its sources | Next | M |

### Assistant for dashboards

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`AI-1`](ai-1.md) | Manage the dashboards that exist today | Next, ready now | M |
| [`AI-2`](ai-2.md) | Dashboard filters | Next, with BI-18 A | S |
| [`AI-3`](ai-3.md) | Time grain | Next, with BI-9 | S |
| [`AI-4`](ai-4.md) | Calculated fields | Next, with BI-8 | M |
| [`AI-5`](ai-5.md) | Every new chart type | Next, with each BI-16 part | S |
| [`AI-6`](ai-6.md) | Richer charts | Next, with each BI phase-2 item | M |
| [`AI-7`](ai-7.md) | Named metrics first | Next, with BI-2 | M |
| [`AI-8`](ai-8.md) | Chart formatting | Next, with BI-17 | S |
| [`AI-9`](ai-9.md) | Tabs and templates | Next, with BI-18 C | S |
| [`AI-10`](ai-10.md) | Reports and exports | Next, with RPT-1 and BI-1 | S |
| [`AI-11`](ai-11.md) | Share and embed, with approval | Next, with BI-19, BI-26 and DEC-8 | S |
| [`AI-12`](ai-12.md) | Help inside the SQL editor | Next, ready now | M |
| [`AI-13`](ai-13.md) | Where does this number come from | Next, with BI-14 | S |
| [`AI-14`](ai-14.md) | Page awareness keeps up | Next, ongoing | S |
| [`AI-15`](ai-15.md) | The standard request set | Next, ready now | M |
| [`AI-16`](ai-16.md) | Hand-off: AI features for the Data module | Next, AI team decides | n/a |

### Data module

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`SRC-6`](src-6.md) | Fix the broken connectors | Next, ready now | S |
| [`SRC-7`](src-7.md) | Alerts when a load fails | Next | M |
| [`SRC-8`](src-8.md) | Schema changes at the source | Next | M |
| [`SRC-9`](src-9.md) | Test every advertised connector end to end | Next | M |
| [`SRC-10`](src-10.md) | More load modes | Next | M |
| [`SRC-11`](src-11.md) | Connector operations | Next | M |
| [`SRC-12`](src-12.md) | Credentials done properly | Next | L |
| [`SRC-13`](src-13.md) | Table discovery for every source type | Next | M |
| [`DATA-11`](data-11.md) | Search that finds columns and tags | Next | M |
| [`DATA-12`](data-12.md) | Certification and governed tags | Next | M |
| [`DATA-13`](data-13.md) | Column-level lineage | Next | L |
| [`DATA-14`](data-14.md) | Data quality in depth | Next | L |
| [`DATA-15`](data-15.md) | Asset page depth | Next | M |
| [`DATA-16`](data-16.md) | Time travel proven safe | Next | S |
| [`DATA-17`](data-17.md) | More formats, more files, bigger files | Next | M |
| [`DATA-18`](data-18.md) | Column types on upload | Next | M |
| [`DATA-19`](data-19.md) | Uploaded tables feed pipelines and dashboards | Next | M |
| [`SRC-4`](src-4.md) | More databases, with live change capture | Next | L |
| [`SRC-3`](src-3.md) | Connectors for business apps | Later | L |

### Query Studio

| ID | Spec | When | Size |
| --- | --- | --- | --- |
| [`QS-1`](qs-1.md) | Editor basics | Next | M |
| [`QS-2`](qs-2.md) | Results you can use | Next | M |
| [`QS-3`](qs-3.md) | Saved queries done properly | Next | M |
| [`QS-4`](qs-4.md) | History and query profile | Next | M |
| [`QS-5`](qs-5.md) | Parameters, snippets and charts from results | Next | L |
