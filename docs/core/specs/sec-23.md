# `SEC-23` Adding a governance rule needs the permission changing one needs

| | |
| --- | --- |
| Backlog | `SEC-23` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code on 2026-10-09. Feature page [`governance-rule-create.md`](../features/governance-rule-create.md) |

*This item was first numbered `SEC-22`; `main` gave that id to another item on 2026-10-09, and ids are never reused.*

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

- `SEC-23-AC1` A user without `governance:write` sees no Add or Classify button for quality and classification rules
- `SEC-23-AC2` That user is refused by `POST /api/governance/{kind}` with 403 and nothing is stored, for quality, classification and residency
- `SEC-23-AC3` That user asking the assistant to add a rule is refused, and no rule appears
- `SEC-23-AC4` A user with `governance:write` adds a quality rule and a classification as before
- `SEC-23-AC5` Without a login the route answers 401

## Not included

- Reading rules (`GET /api/governance/{kind}`) stays open to any signed-in user.
- Running a rule stays `query:read`.

## Asking the assistant

The assistant's rule-drafting tools are covered by this fix. The AI team reviews that part of the diff and that the tool schema snapshot is unchanged.
