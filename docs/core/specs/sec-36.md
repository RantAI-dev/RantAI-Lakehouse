# `SEC-36` The history scan checks `main` and the pull request, not every branch

| | |
| --- | --- |
| Backlog | `SEC-36` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | S |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The `gitleaks (full git history)` job in `.github/workflows/security.yml`
runs `git fetch --all` and lets `gitleaks` read every ref. A test value
pushed on one open branch turned the job red on `main` and on every pull
request (22 `generic-api-key` findings on 2026-10-15, all on
`feat/phase-1-rest`, #144). A check that is red for reasons outside the
pull request stops meaning anything.

The repository is public, so a secret on any branch is public. Scanning
every branch still matters; it must not decide whether someone else's pull
request is green.

## Decisions (product owner, 2026-10-15)

| # | Decision |
| --- | --- |
| D1 | On a push to `main` and on a pull request, the history scan reads the history of the checked-out commit only: `main`'s history, plus the pull request's commits on a pull request. |
| D2 | A separate scheduled job scans every branch once a day and fails loudly on a finding. It is not a pull-request check. |
| D3 | `.gitleaksignore` keeps its rule: exact fingerprints with proof, never a wildcard. |

## Target

| Capability | Today | Target |
| --- | --- | --- |
| A finding on an unrelated branch | Red on `main` and every pull request | Not seen by `main`'s or another pull request's check; seen by the daily all-branch scan |
| A finding in a pull request's own commits | Red | Red (unchanged) |
| A finding anywhere on `main` | Red | Red (unchanged) |

## Acceptance checklist

- `SEC-36-AC1` The pull request that makes this change has a green history scan while #144's branch is unmerged and `main`'s `.gitleaksignore` lacks its entries
- `SEC-36-AC2` A local run shows the difference: scanning `HEAD` finds nothing new; scanning `--all` with #144's branch fetched finds its 22 commits' findings
- `SEC-36-AC3` The daily workflow exists, runs on `workflow_dispatch` too, and `docs/CI.md` says what it covers and who sees a red run

## Not included

Removing anything from history. Changing the rules in `.gitleaks.toml`.
