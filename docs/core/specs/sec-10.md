# `SEC-10` Alert webhooks cannot reach internal addresses

| | |
| --- | --- |
| Backlog | `SEC-10` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security |
| When | Now |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

The webhook sender checks only that a URL starts with http(s), so an alert can call services inside the customer's network. AGENTS.md requires the allowlisted resolver for outbound calls.

## What users get

An alert can only call addresses outside the private network, unless an admin allows one explicitly.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Resolver check before sending | Prefix check only | resolve_checked on the host before every send; private, loopback and link-local addresses refused |
| Redirects | Followed | Not followed (or every hop re-checked) |
| Address pinning | None | The request goes to the address that passed the check |
| Admin allowlist | None | An explicit allowlist setting for internal webhook targets *(proposed)* |

## Benchmark

Re-checked by the planner: lakehouse-notify.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A webhook to 127.0.0.1, 10.0.0.0/8, 169.254.169.254 or a name resolving there is refused with a fixed message
- [ ] A public URL answering 302 to an internal address is not followed
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
