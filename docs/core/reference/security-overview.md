# Security and compliance overview

What to hand a customer's security reviewer. It states what is in place,
with the file that proves it, and what is not. It is a summary; the sources
are the authority.

Assessed 2026-10-02 from the repository. Not a penetration test, and not a
certification.

## Deployment model

| Question | Answer | Source |
| --- | --- | --- |
| Where does it run? | On the customer's own host, as a container stack | `docker-compose.yml`, `docs/OPERATIONS.md` |
| Does data leave the premises? | Not by the product's own design. Two things can send data out if the customer configures them: an external language-model endpoint, and alert delivery by email or webhook | `docs/OPERATIONS.md`; `lakehouse-notify` |
| Can AI features stay on premises? | Yes, with a self-hosted model endpoint | Sales playbook |
| What format is data stored in? | Raw data in Apache Iceberg on S3-compatible storage. Curated data in the analytics engine's own format, with an optional Iceberg copy | `docs/ARCHITECTURE.md`, ADR 0010 |

## Access control

| Control | Status | Source |
| --- | --- | --- |
| Every API route has a declared policy; undeclared routes are refused | In place | `rust/crates/lakehouse-api/src/policy.rs`; `tests/route_auth.rs` |
| Role-based permissions | In place | `rust/migrations/0002_seed_identity.sql`; Administration module |
| Service identities for scheduled jobs, with the narrowest scope | In place | `main.rs` bootstrap functions |
| Catalog authorization enforced by default | In place | ADR 0011 |
| No default administrator account | In place | `docs/OPERATIONS.md` "Bootstrap admin" |
| Single sign-on | Partial, not verified end to end | `README.md` |
| Login rate limiting | Absent | `README.md`, `SECURITY.md` |
| Session and token cleanup | Absent | `README.md` |
| Tenant isolation at query level | One tenant per deployment | Sales playbook (dated) |

## Data protection

| Control | Status | Source |
| --- | --- | --- |
| Classification and masking | In place | Governance module; `docs/FEATURE_COVERAGE.md` |
| Policies applied to queries, alerts, digests and exports | In place | `routes/alerts.rs` tests; `gold_export.rs` |
| Read-only guard on user SQL | In place | Sales playbook; `CHANGELOG.md` |
| Secrets held by reference, never returned by the API | In place | ADR 0002; `AGENTS.md` |
| Outbound connections restricted to an allowlist | In place | `AGENTS.md`; `ssrf_guard*.py` |
| Encryption in transit between components | Not verified | |
| Encryption at rest | Not verified; depends on the customer's storage | |

## AI governance

| Control | Status | Source |
| --- | --- | --- |
| Assistant can only call registered tools | In place | ADR 0012 |
| Low-risk writes need inline confirmation | In place | ADR 0012 |
| Destructive actions need approval by a person | In place | ADR 0012; Approvals page |
| Every tool call and decision is recorded | In place | ADR 0012; Audit Log |
| Read-only mode blocks write tools | In place | `SECURITY.md` |

## Audit

| Control | Status | Source |
| --- | --- | --- |
| Pipeline run history | In place | Audit Log |
| AI-agent actions | In place | ADR 0012 |
| All console configuration changes | Partial | Sales playbook (dated) |
| Login and logout events | Not verified | Sales playbook (dated) says absent |

## Secure development

| Control | Status | Source |
| --- | --- | --- |
| Automated tests, lint and type checks on every change | In place | `docs/CI.md` |
| Dependency advisories and licence checks | In place | `docs/CI.md` |
| Secret scanning of the working tree | In place | `docs/CI.md` |
| Secret scanning of history | Red on purpose; see below | `docs/CI.md`, `SECURITY.md` |
| Planner and developer are separate agents; reviewer re-runs checks | In place | `AGENTS.md` |
| Branch protection on `main` | Absent | Verified 2026-10-02 |
| Independent human security review | Not done | `README.md` |

## Disclosures a reviewer will find anyway

Stated plainly, as `SECURITY.md` does:

1. **A previously internal API key is in the public git history**, with
   internal hostnames. It must be treated as compromised and rotated. The
   history scan in CI is deliberately red so it reports this truthfully.
2. **The backend was ported from TypeScript to Rust by AI agents and
   reviewed by AI reviewers.** There has been no full human security or
   architecture review.
3. **Issues fixed before release** are listed in `SECURITY.md`
   (unauthenticated API surface, privilege escalation on identity routes,
   an embed signing secret returned over HTTP, write tools running in
   read-only mode).

## Vulnerability handling

| Question | Answer |
| --- | --- |
| How to report | GitHub private vulnerability reporting (`SECURITY.md`) |
| Response commitment | None; best effort |
| Supported versions | `main` only |

## Compliance

| Topic | Position |
| --- | --- |
| Data residency | Supported by design: the product runs on the customer's premises |
| Certifications (ISO 27001, SOC 2, others) | None claimed |
| Regulatory mapping | Not written. The sales playbook names the regulations that motivate buyers; no document maps product controls to their clauses |
| Data processing agreement | Not applicable to self-hosted use; would apply to any managed service |
| Licence | AGPL-3.0-or-later; see `LICENSE`, `NOTICE` |

## What a customer should do before production

1. Run their own security review; ours has gaps we have named.
2. Put the console behind their network controls; there is no rate limiting.
3. Use their own identity provider only after testing the login flow.
4. Decide who may publish data in open format; published copies sit outside
   the console's access controls.
5. Point AI features at an endpoint they trust, or leave them off.
6. Set up and test backups.
