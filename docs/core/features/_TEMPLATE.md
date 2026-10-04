# <Feature name>

One page. The agents build from it; the product owner accepts with it.
Copy this file to `<feature-slug>.md` and fill it in. Keep it to what a user
can observe; how it is built belongs in the engineering plan.

| | |
| --- | --- |
| Module | <module> |
| Backlog | <ID> |
| Status | Draft / Decisions signed <date> / In build / Accepted <date> |
| Plan | `docs/superpowers/plans/<date>-<slug>.md` |

## Problem

What is wrong today, in two or three sentences, with the evidence.

## What the user can do when this is done

Short, separate, checkable statements. One per line.

1. ...
2. ...

## Not included

What this deliberately leaves out.

## Asking the assistant

Can the assistant do this too? If yes, what does the user ask? If no, why
not?

## Decisions

Anything the product owner must choose. Until signed, the default is used.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |

## Limits to tell a customer

Plain statements of what it will not do. Copied into `PRODUCT.md` section 3
when accepted.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Steps needing a terminal are marked
(operator).

Always include: the normal path, a failure being visible, a user without
permission, switching it off, and an unknown value shown honestly.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
