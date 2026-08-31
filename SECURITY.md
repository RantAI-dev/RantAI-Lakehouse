# Security Policy

## Reporting a Vulnerability

If you believe you've found a security vulnerability in RantAI Lakehouse,
please report it privately rather than opening a public issue.

**Preferred channel: GitHub private vulnerability reporting.** This
repository has private vulnerability reporting enabled. Go to the
[Security tab](https://github.com/RantAI-dev/RantAI-Lakehouse/security) →
["Report a vulnerability"](https://github.com/RantAI-dev/RantAI-Lakehouse/security/advisories/new)
to open a private advisory. This notifies maintainers directly and does
not disclose the report publicly, and it does not require you to have an
email address for the project.

*(Optional, not yet set up: the maintaining org may add a monitored
security-contact email address here in the future. None is published today
— do not send reports to a guessed address such as `security@rantai.dev`;
it is not confirmed to be monitored.)*

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
security team, and **no formal SLA is offered yet.** Reports will be
triaged on a best-effort basis by whoever is available; there is currently
no committed acknowledgement or fix timeline. If that changes, this
section will be updated with a real, kept target rather than an aspirational
one.

## Known exposure: leaked secret in git history

Being direct about this because the repository is public and anyone who
looks at `git log` will find it anyway:

- A previously-internal LLM API key is present in git history, on **2
  commits reachable from `main`**. Internal LAN hostnames are also present
  in history, across roughly 10 commits.
- Both predate this repository going public.
- **The key must be treated as compromised and rotated at its provider.**
  Removing it from git history does not un-leak it — it was already
  world-readable the moment the repo went public, and likely cached by
  forks, clones, and crawlers before any history rewrite could happen.
- CI's `gitleaks (full git history)` job (`security.yml`) is **intentionally
  red** because of this — it is a custom rule specifically added so the
  scan reports the leak truthfully instead of silently passing. See
  [docs/CI.md](docs/CI.md) for the full, honest account, including why it
  is deliberately excluded from required status checks.
- Actually clearing the history requires a `git filter-repo` (or BFG)
  rewrite plus a force-push to every affected ref — a destructive,
  cross-cutting operation that invalidates every existing clone and open
  PR based on the old history. That is a maintainer decision, deliberately
  not taken automatically as part of routine hardening work; see
  [docs/CI.md](docs/CI.md) for what it would involve.

If you find this key still active, please report it via the private
vulnerability reporting channel above rather than opening a public issue.

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
