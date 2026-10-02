# Backlog

The one list of known work. Look at it weekly; pick what is next.

- **When:** Now, Next, Later, or Decide (needs a yes/no first).
- **Source:** where the item comes from, so it can be checked. "PRODUCT §2"
  is the coverage table in `PRODUCT.md`.
- IDs are never reused. A finished or rejected item moves to the bottom with
  the date and reason; it is not deleted.

## Now

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `DATA-1` | Gold tables publish to open format automatically, as an option per table. **Built and merged (PRs #60, #63 and the slice C+D PR); waiting for the product owner to run the acceptance checklist** | Build | `features/gold-publish-per-mart.md` |
| `QA-1` | Accept each built feature with a checklist, starting with what a demo shows | All | PRODUCT §4 blocker 1 |
| `SEC-1` | Confirm the leaked key is rotated; decide on rewriting git history | Security | `SECURITY.md` |
| `REL-1` | Protect the main branch | Delivery | `docs/CI.md` |
| `DOC-1` | Archive or rewrite the old product documents that contradict the build | Docs | PRODUCT §6 |
| `GOV-2` | Decide which role may publish a table in open format | Governance | PRODUCT §6 |

## Next

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `RPT-1` | Scheduled reports with PDF and spreadsheet attachments | Dashboards | PRODUCT §2 |
| `BI-1` | Export a dashboard as a server-made PDF and as Excel | Dashboards | PRODUCT §2 |
| `BLD-1` | Joins and aggregations in the pipeline builder | Build | PRODUCT §2 |
| `VER-1` | Verify SFTP, Google Sheets and Oracle sources end to end; test row filters | Data, Governance | PRODUCT §2 |
| `BI-2` | Metrics users can define once and reuse across dashboards | Dashboards | PRODUCT §2 |
| `DATA-9` | Upload a file from the console | Data | PRODUCT §2 |
| `OPS-8` | Usage and cost view | Monitoring | PRODUCT §2 |
| `SEC-2` | Login rate limiting. **Planned** | Security | `features/login-protection-and-session-cleanup.md` |
| `SEC-3` | Test single sign-on against a real identity provider; correct `README.md`, which says it is unbuilt | Security | PRODUCT §2 |
| `SEC-7` | A monitored security contact address | Security | `SECURITY.md` |
| `REL-2` | Adopt a version and release policy; cut a release from the unreleased work | Delivery | `reference/release-policy.md` |
| `GOV-1` | Audit trail covers every console change | Governance | PRODUCT §3 |
| `OPS-1` | Trim old versions of raw tables | Monitoring | `README.md` |
| `DATA-2` | Trim old copies in published Gold tables | Build | `features/gold-publish-per-mart.md` |
| `DATA-10` | A merge-proof "has this mart changed" signal for publishing, so background merges do not trigger extra copies | Build | Slice B review, gold-publish plan |
| `DATA-6` | A "latest copy only" view for outside readers of published tables | Build | `features/gold-publish-per-mart.md` |
| `SEC-4` | Human security review | Security | PRODUCT §4 blocker 3 |
| `SUP-1` | Decide a support commitment | Business | PRODUCT §4 blocker 5 |
| `OPS-7` | Test and time a restore from backup | Monitoring | PRODUCT §2 |
| `SEC-5` | Clean up old sessions and tokens. **Planned** | Security | `features/login-protection-and-session-cleanup.md` |
| `SEC-6` | Stop older API handlers from returning internal error text | Security | `AGENTS.md` |

## Later

| ID | Item | Area | Source |
| --- | --- | --- | --- |
| `OPS-3` | Run on several servers; high availability | Platform | PRODUCT §1, §2 |
| `SRC-3` | Connectors for business apps (pick by customer demand) | Data | PRODUCT §2 |
| `SRC-4` | Live change capture for databases other than PostgreSQL | Data | PRODUCT §2 |
| `DATA-7` | Query a table as of an earlier time | Data | PRODUCT §2 |
| `DATA-8` | Instant copies of a table for testing | Data | PRODUCT §2 |
| `BLD-2` | Tables that refresh themselves when their inputs change | Build | PRODUCT §2 |
| `SRC-5` | Real connection test for MongoDB, Kafka, SFTP, Sheets and Oracle sources | Data | `connector_probe.rs` |
| `DOC-2` | Step-by-step user guide checked against a running console | Docs | `reference/user-guide.md` |
| `DATA-3` | Store Gold in open format first; serve from a fast copy | Build | ADR 0010 |
| `DATA-4` | Publish Silver in open format | Build | `features/gold-publish-per-mart.md` |
| `DATA-5` | Measure outside readers of published tables | Build | `routes::gold::consumers` |
| `BI-3` | Import dashboards from Tableau and Power BI | Dashboards | PRODUCT §2 |
| `QRY-1` | Query other systems in place, without loading | Data | PRODUCT §2 |
| `SRC-2` | Processing over streams | Data | PRODUCT §2 |
| `INT-2` | Search over documents | Ask AI | PRODUCT §2 |
| `GOV-3` | Lineage that reaches outside systems | Governance | PRODUCT §2 |
| `ADM-1` | Several organisations in one install | Administration | PRODUCT §2 |
| `ADM-2` | Workspace settings that can be changed | Administration | `README.md` |
| `OPS-2` | Install with no internet access | Platform | Sales playbook (older; re-check) |
| `OPS-4` | Fail loudly when the application database is unreachable | Platform | `docs/OPERATIONS.md` |
| `OPS-6` | Log and trace viewer | Monitoring | `docs/FEATURE_COVERAGE.md` |
| `BIZ-1` | Enterprise edition | Business | PRODUCT §1 |
| `BIZ-2` | Hosted cloud version | Business | PRODUCT §1 |

## Decide

Competitors have these. Each is large. A "no" is a valid answer.

| ID | Item | Recommendation |
| --- | --- | --- |
| `DEC-1` | Notebooks and Python | Defer |
| `DEC-2` | dbt support | Defer |
| `DEC-3` | AI functions inside SQL | Defer |
| `DEC-4` | Train and serve models | No |
| `DEC-5` | Mobile app | No |
| `DEC-6` | Share data with another organisation | Defer |
| `INT-1` | Digital employees | Parked by the product owner, 2026-10-02 |

## Done and rejected

| ID | Item | Outcome | Date |
| --- | --- | --- | --- |
| `PM-1` | Customer-facing product name | Decided: RantAI Lakehouse | 2026-10-02 |
| `SEC-8` | Dependency security checks green on `main` | Done: PR #62. Three TLS-library vulnerabilities cleared by updating the SQL Server client; licence check fixed | 2026-10-02 |
