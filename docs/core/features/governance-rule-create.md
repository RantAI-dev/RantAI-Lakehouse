# Adding a governance rule needs the right to manage rules

| | |
| --- | --- |
| Module | Security (Governance) |
| Backlog | `SEC-22` |
| Spec | `docs/core/specs/sec-22.md` (no *(proposed)* numbers) |
| Status | Decisions signed 2026-10-09. In build |
| Plan | `docs/superpowers/plans/2026-10-09-sec-22-governance-rule-create.md` |

## Problem

Any signed-in user can add a quality rule, a classification rule or a
residency rule: the route asks only for a login. Changing or removing a rule
needs `governance:write`. The assistant's rule tools call the same code with
no check. Read from `main` at `c338862` on 2026-10-09; evidence is in the
plan, section 2.

## What the user can do when this is done

1. With `governance:write`: add a quality rule, classify a table or column,
   and add a residency rule, as today.
2. Without it: see the rules, and see no button to add one.
3. Without it, calling the API or asking the assistant to add a rule: be
   refused with a plain message, and nothing is stored.

## Not included

- Who may read rules, and who may run a quality rule.
- Any new role or grant. `governance:write` is held by the roles that hold
  it today.

## Asking the assistant

Yes, for a user with `governance:write`: "add a check that order_id is never
empty". Without it the assistant says the user may not add rules.

## Decisions

Signed by the product owner on 2026-10-09 ("run as proposed").

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Which permission adding a rule needs. | `governance:write`, the one changing and removing a rule needs. | 2026-10-09 |
| 2 | Classification and residency rules use the same route and have the same gap; the owner was asked about quality rules. | Fixed together, same permission. | |
| 3 | People who add rules today without `governance:write` (for example a Data Engineer) lose the button. | Accepted; an administrator gives them the role if they should keep it. | |

## Limits to tell a customer

- Adding, changing and removing governance rules all need
  `governance:write`.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | As a user with `governance:write`, add a quality rule on an asset's Quality tab and on the Data Quality page | Saved | |
| 2 | As the same user, classify a column on an asset's Access tab and add a rule on the Classification page | Saved | |
| 3 | As a user without `governance:write`, open the same four places | The rules are listed; there is no Add or Classify button | |
| 4 | (operator) As that user, `POST /api/governance/quality` with a valid body | 403 with a fixed message; no rule appears | |
| 5 | As that user, ask the assistant to add a quality rule | It says the user may not add rules; no rule appears | |
| 6 | (operator) Without a login, `POST /api/governance/quality` | 401 | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
