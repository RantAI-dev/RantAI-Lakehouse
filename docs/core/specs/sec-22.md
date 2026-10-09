# `SEC-22` Adding a governance rule needs the permission changing one needs

| | |
| --- | --- |
| Backlog | `SEC-22` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Security, with Data |
| When | Now |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Feature page [`governance-rule-create.md`](../features/governance-rule-create.md), decisions signed 2026-10-09. Plan written |

## Why

Any signed-in user can add a quality rule, a classification rule or a residency rule. Changing or removing one needs `governance:write`.

## What users get

Only people allowed to manage governance rules can add one, in the console, through the API and through the assistant.

## Target specs

"Today" is `main` at `c338862`, read from the code on 2026-10-09, not tested. There is no *(proposed)* number.

| Capability | Today | Target |
| --- | --- | --- |
| `POST /api/governance/{kind}` (quality, classification, residency) | Any signed-in user | `governance:write`, as for changing and removing |
| The assistant's tools that add a quality or classification rule | Call the same code with no permission check | Refuse a user without `governance:write`, with a plain message |
| Console "Add" and "Classify" buttons | Shown to everyone | Shown only with `governance:write`, like Edit and Delete |

## Benchmark

None. Found by reading the code while planning `DATA-14`.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A user without `governance:write` sees no Add button and is refused by the API with a fixed message
- [ ] A user with `governance:write` adds a quality rule and a classification as before
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Reading rules (`GET /api/governance/{kind}`) stays open to any signed-in user.
- Running a rule stays `query:read`.

## Asking the assistant

The assistant's rule-drafting tools are covered by this fix. The AI team reviews that part of the diff and that the tool schema snapshot is unchanged.
