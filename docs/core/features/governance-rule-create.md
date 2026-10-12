# Adding a governance rule needs the right to manage rules

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `SEC-23` |
| Spec | `docs/core/specs/sec-23.md` (no *(proposed)* numbers) |
| Kind | Task (it has an acceptance checklist because it changes what a user sees) |
| Status | In Progress (the pull request is open; `BACKLOG.md` holds the status) |
| Priority | P1 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-09; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | PR #97 |
| Plan | `docs/superpowers/plans/2026-10-09-sec-23-governance-rule-create.md` |

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

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `SEC-23-AC1` | As a user with `governance:write`, add a quality rule on an asset's Quality tab and on the Data Quality page | Saved | Not run |
| `SEC-23-AC2` | As the same user, classify a column on an asset's Access tab and add a rule on the Classification page | Saved | Not run |
| `SEC-23-AC3` | As a user without `governance:write`, open the same four places | The rules are listed; there is no Add or Classify button | Not run |
| `SEC-23-AC4` | (operator) As that user, `POST /api/governance/quality` with a valid body | 403 with a fixed message; no rule appears | Not run |
| `SEC-23-AC5` | As that user, ask the assistant to add a quality rule | It says the user may not add rules; no rule appears | Not run |
| `SEC-23-AC6` | (operator) Without a login, `POST /api/governance/quality` | 401 | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
