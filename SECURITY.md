# Security Policy

## Reporting a Vulnerability

If you believe you've found a security vulnerability in RantAI Lakehouse,
please report it privately rather than opening a public issue.

**Contact:** `security@rantai.dev` — **TODO: confirm.** This address has not
yet been verified as monitored; until this line is updated, please instead
open a [private security advisory](https://github.com/RantAI-dev/RantAI-Lakehouse/security/advisories/new)
on this repository (GitHub Security Advisories), which notifies maintainers
without disclosing the report publicly.

Please include:

- A description of the vulnerability and its potential impact.
- Steps to reproduce (a minimal repro is very helpful).
- The affected version/commit.
- Any suggested remediation, if you have one.

Please do not disclose the issue publicly (including in GitHub issues,
discussions, or pull requests) until we've had a chance to investigate and
respond.

## Response Expectations

This is an early-stage, actively developed project without a dedicated
security team. As a best-effort target (**TODO: confirm** against whatever
SLA the maintaining org eventually commits to):

- **Acknowledgement:** within 5 business days.
- **Initial assessment** (severity, whether it's accepted as a
  vulnerability): within 10 business days.
- **Fix or mitigation timeline:** communicated once the assessment is
  complete, prioritized by severity.

## Supported Versions

This project has not yet cut a `v0.1.0` release; `main` (and the
`feat/rust-backend` integration branch, pre-merge) is the only supported
line. Once tagged releases begin, this table will be updated to reflect
which versions receive security fixes.

| Version | Supported |
| --- | --- |
| `main` (pre-release) | :white_check_mark: |

## Known, Already-Disclosed Issues

The following issues were identified and fixed during development, prior to
this repository's initial release; see [CHANGELOG.md](CHANGELOG.md) for
details:

- Unauthenticated API surface (fixed by introducing the `lakehouse-auth`
  authentication core and wiring it into the router).
- Privilege escalation on `/api/identity/*` (fixed by permission-gating
  those routes instead of relying on auth-only checks).
- The embed HMAC signing secret being returned over HTTP (fixed).
- `ai/chat` executing write tools even in read-only mode (fixed by
  enforcing the write-tool block at dispatch time).

## Scope Notes

This project has known, intentional limitations that are **not** considered
vulnerabilities to report (they're already tracked as accepted risk — see
the "Status / Known limitations" section of [README.md](README.md)):

- No login rate limiting beyond logging.
- The service does not refuse to boot when Postgres is unreachable;
  dependent routes return `503` instead.
- Sessions and service tokens have no automatic rotation/cleanup job.

If you find a way to escalate one of these into something more severe
(e.g. an actual authentication bypass, not just "no rate limiting"), that
**is** worth reporting.
