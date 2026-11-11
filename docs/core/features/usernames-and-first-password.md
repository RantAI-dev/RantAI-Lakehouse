# Usernames, optional email, and an admin-set first password

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-11` |
| Kind | Feature |
| Spec | `docs/core/specs/adm-11.md` |
| Status | In Acceptance |
| Priority | P2 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | `docs/superpowers/plans/2026-11-09-adm-11-usernames-and-first-password.md` |
| Started | 2026-11-09 |
| Shipped | 2026-11-09 |
| Evidence | PR #442 |

## Problem

Sign-in is by email only and is case-sensitive. An email is required even where there is no mail server. The first password is always generated and must always be changed. The server checks no password rule.

## What the user can do when this is done

1. Every user has a unique username; existing users get their email, lowercased.
2. The sign-in box says "Username or email" and accepts either, in any letter case.
3. An administrator invites a user with a username, an optional email, roles and tenants picked from checkboxes, and either a typed first password or a generated one.
4. The administrator chooses whether the user must change the password at first sign-in (on by default).
5. A password that breaks the rules is refused by the server with the rule it broke, on invite, reset and change.
6. An administrator can change a user's username and email.
7. A user with no email gets no mail; the alert, report or mention records why.

## Not included

- Forgot-password by email.
- Complexity classes, history, expiry.

## Asking the assistant

No.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | Unique username, optional email, sign-in with either | As stated | Owner, 2026-11-09 |
| D2 | Existing users' username is their email, lowercased | As stated | Owner, 2026-11-09 |
| D3 | Username characters `a-z 0-9 . _ @ + -`, 1 to 254 long, stored lowercase | *(proposed)* | |
| D4 | Password at least 8 characters, at most 1024, not equal to username or email | *(proposed)* | |
| D5 | A username may not equal another user's email, and the reverse | As stated | Planner |

## Limits to tell a customer

- Without an email a user receives no alerts, reports or mention notices by mail.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `ADM-11-AC1` | After upgrade, sign in as an existing user with their email in capitals | Signed in | Not run |
| `ADM-11-AC2` | Users list | Shows a Username column; existing users' username is their email in lowercase | Not run |
| `ADM-11-AC3` | Invite `ana` with no email, a typed password `short` | Refused: "at least 8 characters" | Not run |
| `ADM-11-AC4` | Invite `ana` with no email, typed password of 12 characters, "must change" off | Created; `ana` signs in with it and is not asked to change it | Not run |
| `ADM-11-AC5` | Invite `budi` with "generate" and "must change" on | Password shown once; first sign-in forces a change | Not run |
| `ADM-11-AC6` | Invite a second user named `ANA` | Refused: username taken | Not run |
| `ADM-11-AC7` | Invite a user whose username equals an existing user's email | Refused | Not run |
| `ADM-11-AC8` | Rename `ana` to `ana.s` | `ana.s` signs in; `ana` no longer does | Not run |
| `ADM-11-AC9` | Change password to 7 characters as a signed-in user (operator, curl, bypassing the form) | 400 with the rule | Not run |
| `ADM-11-AC10` | Subscribe `ana` (no email) to a report and send now | Not sent; the run says the user has no email address | Not run |
| `ADM-11-AC11` | Wrong password for `ana` | "Invalid username/email or password"; the lockout from `ADM-8` still applies | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
