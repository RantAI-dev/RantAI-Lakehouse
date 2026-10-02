# Support model

**Status: proposed structure, no commitments set.** The repository offers no
support commitment today: `SECURITY.md` states "no formal SLA is offered
yet" and that reports are triaged best-effort. A customer contract needs
more than that. This document gives the structure; the product owner fills
in the targets (backlog `SUP-1`).

Response and resolution times are deliberately blank. A target should be set
only once there are people to meet it.

## Severity levels

| Level | Definition | Examples in this product |
| --- | --- | --- |
| **S1 — Critical** | Production is unusable, or data is wrong, lost or exposed | Console unreachable; dashboards show wrong figures; change capture stalled and filling the source database's disk; data visible to someone who should not see it |
| **S2 — High** | A main function is unusable with no workaround | Pipelines cannot run; ingestion stopped; logins fail for everyone; published tables not updating |
| **S3 — Medium** | A function is impaired, or a workaround exists | One pipeline fails; one dashboard tile errors; an alert is not delivered; a scheduled job fails but can be run by hand |
| **S4 — Low** | Cosmetic, a question, or a request | Wording, layout, how-to questions, feature requests |

Wrong data is always S1, even when everything appears to be working. The
product's own rule is that a silent wrong answer is the most dangerous kind
of failure.

## Targets to be set

| Level | First response | Workaround or fix | Updates |
| --- | --- | --- | --- |
| S1 | To be decided | To be decided | To be decided |
| S2 | To be decided | To be decided | To be decided |
| S3 | To be decided | To be decided | To be decided |
| S4 | To be decided | To be decided | To be decided |

Also to be decided: support hours, time zone, language, and whether S1
cover is around the clock.

## How a customer reaches us

| Channel | Status |
| --- | --- |
| Security reports | GitHub private vulnerability reporting (`SECURITY.md`) |
| Bug reports and questions | Public repository issues for the community edition |
| Contracted support channel | To be decided |
| Monitored security email | None published (`SECURITY.md`); backlog `SEC-7` |

## What a customer should send

So that a report can be acted on without a round trip:

1. Version or commit deployed.
2. What they did, what they expected, what happened.
3. Time it happened, and whether it is still happening.
4. Which module and page.
5. For pipeline problems: the pipeline name and run.
6. Output of the relevant acceptance gate, if they can run it.

No passwords, tokens or personal data in a report.

## First-line diagnosis

Where to look before escalating. Pages are in the Monitoring module.

| Symptom | Look at |
| --- | --- |
| Pages return errors, service appears up | Services; application database reachability (`docs/OPERATIONS.md`, "Postgres-down is quiet") |
| Data is stale | Ingestion (CDC); the pipeline's run history; Alerts |
| A pipeline failed | The pipeline's run inspector; Alerts |
| Queries are slow | Workloads; Table Maintenance |
| A published table is out of date | The mart's page; its publish history |
| An AI action did not happen | Approvals; Audit Log |
| Something changed and nobody knows who | Audit Log, noting its coverage limit in [PRODUCT.md](../PRODUCT.md) |

## Escalation

| Step | Who | To be decided |
| --- | --- | --- |
| First line | Customer's operator or integrator partner | Training and runbook |
| Second line | Our support | Staffing |
| Engineering | Planner and developer agents, with a human owner | Who the human owner is |

## What support covers

To be decided with packaging (sales playbook section 4). The proposal there
is: no support for the community edition; an annual enterprise licence with
security patches, updates and a support commitment; optional managed
operations.

## Supported versions

Depends on [release-policy.md](release-policy.md). Today only `main` is
supported.
