# `SEC-15` Connection tests cannot reach internal addresses

| | |
| --- | --- |
| Backlog | `SEC-15` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Also involves | Data |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The internal-address block is off in the shipped compose, the REST test follows redirects after checking only the first host, no API-side test pins the resolved address, and CDC delete skips the check.

## What users get

A connection test cannot be used to map or attack services inside the customer's network.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Default in compose | Internal hosts allowed | Blocked by default; allowing them is an explicit, documented opt-in |
| REST test redirects | Followed | Not followed, or every hop re-checked |
| Address pinning | None | Every API-side test dials the address resolve_checked returned |
| CDC delete | Dials without the check | Uses resolve_checked |
| Error messages | Include resolved internal IPs | Fixed messages without addresses |

## Benchmark

Re-checked by the planner: docker-compose.yml CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS defaults to true; connector_probe.rs uses reqwest::Client::new().

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SEC-15-AC1` A REST test against a public URL that redirects to 169.254.169.254 is refused
- `SEC-15-AC2` A fresh compose install refuses a test against 127.0.0.1
- `SEC-15-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
