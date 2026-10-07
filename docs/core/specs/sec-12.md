# `SEC-12` Safer embed tokens

| | |
| --- | --- |
| Backlog | `SEC-12` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security, with the dashboards team |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Embed tokens can be made without an expiry, cannot be revoked, the signing secret can sit in plain text, and pages can be framed by any site.

## What users get

Embed links expire and can be withdrawn at once.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Expiry | Optional | Required on every token; maximum lifetime set by an admin, default 24 hours *(proposed)* |
| Revocation | None | Revoke one token or all tokens of a dashboard; effective within 1 minute *(proposed)* |
| Signing secret | Plain text in console.app_kv when EMBED_SECRET is unset | EMBED_SECRET required (${X:?}) or stored encrypted |
| Framing | No frame-ancestors policy | An allowlist of host domains per dashboard, sent as frame-ancestors |

## Benchmark

Tableau connected apps and Metabase guest embeds both expire tokens; Metabase renews them automatically (Pro+).

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A token without exp is refused
- [ ] A revoked token stops working within a minute
- [ ] The embed page refuses to render inside a site not on the allowlist
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- An embedding SDK (BI-26 keeps embedding small)

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
