# RantAI Lake — On-Premise Sales Playbook

Written against the code at `4e02361` (the rebased P0–P6 + R1 + Gold-export
stack, 84 commits ahead of `main`), not against `PRODUCT_SPECS.md`, the
README, or the current deck. Every claim below was checked against source,
compose, ADRs, or gate tests. Where the product is not there yet it says so,
because on-prem buyers run a technical evaluation and a claim that fails on
their hardware costs more than the claim was worth.

Companion: `GTM/GTM-AUDIT.md` audits the *deck*; this document is the
*strategy*. **Updated 2026-09-09:** digital employees and agent runs now
execute for real (`docs/adr/0012-copilot-tool-governance.md`) — a
scheduled or "Run now" employee run works through the same 47-tool
registry and approval gate the copilot uses, and `/agents/runs` shows real
history, not seeded fixtures. It is still a narrower story than "lead with
autonomous agents": an employee is capped by its own granted permissions
and can only call the registered tools, and every write it attempts is
still inline-confirmed or queued for human approval like any other
copilot action. Lead with the human-approval story before the autonomy
angle.

---

## 1. What you are actually selling

One sentence, for a CIO:

> A self-hosted lakehouse that turns your PostgreSQL applications into live,
> governed analytics on open Apache Iceberg tables you own, with dashboards,
> alerts, and natural-language SQL on top, deployable on one Docker host
> inside your data center.

The shape of the product, as built:

| Layer | What runs | Real today |
| --- | --- | --- |
| Sources | PostgreSQL via logical replication (Debezium, ~1–2 s to visibility) or scheduled batch (dlt) | Yes, both gated in CI |
| Bronze | Apache Iceberg v2 tables on S3-compatible object store (RustFS default, SeaweedFS verified), Lakekeeper REST catalog, OpenFGA authorization enforced by default | Yes |
| Analytics engine | ClickHouse 26.8 LTS reading Iceberg through the catalog; Silver/Gold marts as ClickHouse SQL | Yes |
| Gold back to open format | Rust service exports any ClickHouse mart to a `gold` Iceberg namespace (append-only) | Yes, gated |
| Maintenance | Dagster: orphan-file removal daily, replication-slot health every 15 min; Trino cron compacts small files | Yes, opt-in Trino profile |
| API | Rust/axum, ~100 routes, deny-by-default per-route policy, 533 tests, SSRF guard, secret-reference allowlist, read-only SQL guard | Yes |
| Console | Next.js: catalog, connectors, pipelines, query studio, dashboards with public/signed embeds, alerts, governance (policies, quality, maintenance, CDC health), identity admin | 10 of 12 domains real |
| AI | NL→SQL with self-repair loop; copilot with 47 real tools across data, dashboards, alerts, connectors, pipelines, governance, ops, and Gold export — every write inline-confirmed or routed to a human-approval inbox, all of it audited; digital employees run the same tools on a schedule; any OpenAI-compatible endpoint, so a self-hosted vLLM/Ollama works | Yes |

Reference deployment: a Portainer stack of 25 pinned images on a single host
(`deploy/187`), running next to live Dinas Pariwisata DKI Jakarta data.
That is your proof that it lives on customer hardware, not a cloud.

## 2. Who buys this on-prem, and why

**Ideal customer profile.** Indonesian government agencies, BUMN, and
regulated enterprises (health, finance, utilities) that:

- run operational systems on PostgreSQL (SIM apps, ERP, e-office), and
- are told by law or policy that the data cannot leave their premises
  (UU PDP, SPBE, sectoral rules), and
- have a data team of one to five people who currently answer questions
  with pgAdmin exports and Excel, and
- already operate Docker or Portainer, or have an SI partner who does.

**The wedge.** "Your PostgreSQL apps become live dashboards without
touching the apps." Logical replication with a scoped publication means
no schema change, no app change, no batch window, and a measured
one-to-two-second lag. That is the demo moment that closes a technical
buyer, and it is fully real.

**What they are choosing against.**

| Alternative | Why they hesitate | Your honest answer |
| --- | --- | --- |
| Databricks / Snowflake / Fabric | Cannot run in their data center; data residency; USD billing | Same open Iceberg format, runs on their host, rupiah contract |
| Self-assembled ClickHouse + dbt + Airflow + Metabase + Ranger | Four vendors, four auth models, nobody owns the seams | One stack, one login, catalog authorization on by default, CDC wired, tested as a unit in CI |
| "Just PostgreSQL read replica + Metabase" | Cheap and familiar | Fine until the first join across systems or the first ten-million-row aggregate. Iceberg + ClickHouse is the upgrade path that keeps the data open |
| Do nothing | No budget owner | The sovereignty angle creates the budget owner: this is a compliance line item, not an analytics one |

## 3. Positioning by audience

Lead with sovereignty and open formats for every audience, then tailor the
second message:

- **CIO / Kepala Dinas**: "Data stays in your building, in a format any
  engine can read, with a working console in a week." Show the Portainer
  stack and the Iceberg tables opened from a second tool.
- **Head of data / IT ops**: "CDC from Postgres in under two seconds,
  compaction and orphan cleanup on a schedule, health surfaced in the
  console." Show Governance → Ingestion and Maintenance dry-run.
- **Analysts**: "Ask in Indonesian, get SQL, get a chart, embed it."
  Show Query Studio NL→SQL and the copilot building a dashboard.
- **Security / compliance**: "Deny-by-default API policy, catalog
  authorization via OpenFGA, credentials referenced never stored, SSRF
  guard, read-only SQL." Hand over `docs/adr/0011` and the security tests.

## 4. Packaging and pricing model

On-prem buyers reject per-seat pricing and want a capital-style number.
Proposed structure:

1. **Community**: the AGPL-3.0 repo as is. Self-serve, no support. This is
   marketing and a top-of-funnel for SI partners.
2. **Enterprise licence** (annual, per deployment, not per user): a
   commercial licence that removes AGPL obligations for their internal
   forks and integrations, plus security patches, pinned-image updates,
   and a support SLA. Price by host tier (single node now; add a
   multi-node tier when Kubernetes exists).
3. **Implementation** (fixed-price phases, see §6). This is where most
   first-year revenue comes from and it is the part the code supports
   best today.
4. **Managed operations retainer**: monthly, runs the Dagster schedules,
   Trino compaction, upgrades, and the quarterly maintenance report.
   Sticky, and it hedges the operational gaps in §7.

The AGPL relicence already committed on `main` is the lever for item 2.
Do not sell the enterprise licence as "more features" yet; there is one
codebase. Sell it as indemnity, support, and upgrades.

## 5. The honest demo script

Run the compose stack with the `dagster` and `trino` profiles and set
`NEXT_PUBLIC_SHOW_PREVIEW=1` at build time, otherwise Connectors, Agents,
Policies, and Admin pages are hidden by a stale nav flag. Set the
`TENANT_*` variables so the console carries the prospect's name.

1. **Connectors**: show the PostgreSQL and S3 connectors, run "Test
   connection". Both genuinely dial. Do not click test on any other type;
   it correctly reports unsupported.
2. **Live CDC**: update a row in the source Postgres from psql. Refresh a
   Query Studio result. It is there in one to two seconds. Show
   Governance → Ingestion for slot lag.
3. **Query Studio**: type a question in Indonesian, show the generated
   SQL, run it, show the transparency panel.
4. **Copilot**: ask it to build a chart and add it to a board. This
   calls real tools against ClickHouse.
5. **Dashboards**: cross-filter, then create a public share link and a
   signed embed. Paste the iframe into a blank page.
6. **Alerts**: create a threshold rule and fire it to a webhook.
7. **Gold export**: `POST /api/gold/export/{mart}`, then read the same
   table from pyiceberg or Trino in a terminal. This is the "no lock-in"
   proof and no competitor demo does it.
8. **Maintenance**: Governance → Maintenance, run a dry-run, show the
   measured file counts.

Do not open Streaming or Semantic Search. Both are mocked. Do not
demonstrate SSO login; the backend verifies tokens but there is no login
redirect flow. Agent runs are real now — a genuine addition to this
script: ask the copilot to delete something (e.g. a connector) in Build
mode, show the pending item land on `/agents/approvals`, approve it as a
different role, then show the connector gone and the run/audit rows on
`/agents/runs` and Governance → Audit. That sequence — human approval
before anything destructive happens — is the moment to slow down on.

## 6. Engagement model that matches the code

Sell four fixed-price phases. Each phase's acceptance test is a gate that
already exists in the repo; run it on the customer's hardware and attach
the output to the invoice.

| Phase | Weeks | Deliverable | Acceptance |
| --- | --- | --- | --- |
| 0 Discovery | 1–2 | Source inventory, host sizing, residency map | Signed scope |
| 1 Connect | 2 | Stack on their host, one Postgres source into Bronze via CDC, one batch source | G4 and G3a gates green on their host |
| 2 Model and serve | 3–4 | Silver/Gold marts as reviewed SQL, first three dashboards, alerts, embeds | Gold-export gate green; dashboards signed off |
| 3 Govern and operate | 2 | OIDC token verification against their IdP, roles mapped, maintenance schedules, runbook | Route-auth test suite green; maintenance report |

Total: roughly eight to ten weeks to a governed, operated deployment. Be
clear that Bronze→Silver→Gold transformation is authored SQL delivered by
the implementation team, not a self-generating pipeline. That is normal
for this market and it is billable.

## 7. Do not oversell: what the code says is missing

Put these in the proposal's "roadmap" column. If a buyer asks and you say
yes, the evaluation will find out.

- **No streaming engine.** No Kafka, Flink, or materialised streaming
  views. CDC is a change pipe, not stream processing.
- **No RAG, vector search, or embeddings.** Knowledge sources and vector
  jobs are metadata records. The `lakehouse-embed` crate is dashboard
  embedding, not text embeddings.
- **Agents execute the copilot's registered tools — they are not general
  autonomous workers.** A digital employee (scheduled or "Run now") runs
  the same 47-tool registry interactive chat uses, capped by that
  employee's own granted permissions. It cannot call arbitrary code, hit
  an external API you haven't wired in as a tool, or do anything a human
  with the same permissions couldn't do through the console. There is
  still no document retrieval / RAG — see above.
- **Every copilot write is inline-confirmed or human-approved — sell this,
  don't bury it.** Low-risk writes (create a chart, trigger a pipeline)
  need an explicit Confirm in the chat. Destructive ones (delete a
  connector, kill a query, run Bronze maintenance) create a pending item
  on an approvals inbox that a *different* (or the same, re-checked)
  human must decide before anything executes; approving replays the exact
  call, rejecting executes nothing. No copilot or scheduled-employee
  action bypasses this.
- **No row-level multi-tenancy.** One deployment is one tenant. A
  passing test documents that tenant membership is not enforced on
  queries. Sell one stack per agency; that is what the reference site
  does anyway.
- **The audit-log sink covers copilot and digital-employee actions, not
  every console mutation.** Governance → Audit unions Dagster/pipeline
  history with an append-only Postgres table of every copilot tool call
  and gate decision (who, what tool, what happened, redacted arguments).
  Login/logout and other console-side mutations outside the copilot are
  not yet written to it.
- **No SSO login flow.** OIDC is a resource server. No rate limiting,
  no CORS layer, no request-size limit.
- **No Kubernetes, Helm, or HA.** Single-node compose. Single Trino
  coordinator.
- **No snapshot expiry or retention policy.** ClickHouse rejects
  `expire_snapshots` on catalog tables; Bronze metadata grows until the
  team adds another tool. Orphan removal and compaction do work.
- **Connector dial-testing covers PostgreSQL and S3 only.** Every other
  connector type is a catalog entry that reports unsupported.
- **Storage tiers**: Cold and AI are hard-coded zero; Warm is an estimate;
  "restore to hot" records an operation and moves nothing.
- **Gold export is append-only** and unscheduled by design; consumers
  filter on `_exported_at`.
- **Not air-gapped out of the box.** Images are pinned, but init
  containers install packages from Alpine, PyPI, and Debian at start. A
  fully offline install needs a pre-baked image set (a two-day task).
- **RustFS is a release candidate.** SeaweedFS is the verified
  alternative. Real AWS S3 and MinIO were never tested.

## 8. Fix before the next customer meeting

Ordered by sales impact per engineering day.

1. **Collateral**: apply `GTM-AUDIT.md` items B-1 to B-5, then rewrite
   the capability slides from §1 of this document. Drop digital
   employees, streaming, and RAG from the capability set; keep them on a
   roadmap slide.
2. **Pre-baked offline image set** so the stack starts with no internet.
   Government evaluators test this on day one.
3. **Brand and tenant via env** end to end: the sidebar logo and name are
   still hard-coded; the built-in dashboard tiles are Dispar-specific.
4. **Flip the stale `preview` flags** in the nav config so real pages
   show by default.
5. **OIDC authorization-code login** against Keycloak. Enterprise
   security reviews expect a working SSO button, and Keycloak is what
   on-prem shops run.
6. **Login rate limiting and an access-audit table.** Both are small and
   both are checklist items in every government security questionnaire.
7. **Snapshot expiry via the Trino cron** so the maintenance story is
   complete.
8. **A Helm chart** only once a prospect with Kubernetes appears. Do not
   build it speculatively.

## 9. Channels and proof

- **Reference**: Dinas Pariwisata DKI Jakarta is the live site. Get a
  written reference and a co-presented case study; one named government
  reference is worth more than the whole deck in this market.
- **SI partners**: the compose-to-Portainer generator makes the stack
  partner-deployable. Recruit two SIs with government e-catalogue
  standing; give them the Community edition and a margin on the
  Enterprise licence.
- **Technical credibility content**: publish the measured findings
  (ClickHouse 26.8 Iceberg behaviour, small-file 22× planning penalty
  and the 280→14 compaction result, CDC latency). Engineers trust a
  vendor that documents its own engine's limits; that is rare and it is
  already written in `docs/plans`.
- **Proof-of-value offer**: two weeks, their host, one Postgres source,
  three dashboards, gates green, fixed price. Convert on the phase-2
  contract.

## 10. Objections you will hear

- *"It is version 0.1."* Yes. Say so, and point to the 533 tests, the
  CI gates, and the running reference site. Price the pilot accordingly
  and position the enterprise licence as the maturity roadmap they fund.
- *"AGPL scares legal."* That is what the enterprise licence is for.
- *"We already have ClickHouse."* Good; the value is the catalog, CDC,
  authorization, console, and Gold export around it. Offer to run on
  their ClickHouse if it is 26.8 or newer.
- *"Can it do AI?"* NL→SQL and a tool-using copilot, yes, against any
  OpenAI-compatible endpoint including one on their GPU. Autonomous
  agents and document search, not yet; do not imply otherwise.
- *"What if you disappear?"* The data is Iceberg on S3-compatible
  storage with a standard REST catalog. Trino, Spark, and pyiceberg read
  it today; the demo proves it in step 7.
