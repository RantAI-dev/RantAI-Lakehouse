# `ADM-11` Admin sets the first password; sign in with a username or an email

| | |
| --- | --- |
| Backlog | `ADM-11` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code at `dcaae8a5` on 2026-11-09. Feature page [`usernames-and-first-password.md`](../features/usernames-and-first-password.md) |

## Why

Sign-in works by email only (`password.rs:314-341`, `WHERE u.email = $1`, case-sensitive). `app_user.email` is `NOT NULL`, so an installation without a mail server must make up addresses. The first password is always generated and the change is always forced (`ADM-3`). The 8-character rule is checked only in the browser (`change-password-step.tsx:115`); the server accepts any password.

## What users get

Every user has a unique username. The email is optional. The sign-in box accepts either, in any letter case. When inviting, the administrator either types the first password (checked against the password rules by the server) or has one generated, and chooses whether the user must change it at first sign-in.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Sign-in identifier | Email, case-sensitive | Username or email, case-insensitive |
| Username | None | Required, unique ignoring case, 1 to 254 characters from `a-z 0-9 . _ @ + -`, stored lowercase *(proposed)* |
| Existing users after upgrade | n/a | Username = their email, lowercased; a case-only clash gets `-2`, `-3` (owner, 2026-11-09) |
| Email | Required | Optional; unique ignoring case when present |
| A username equal to another user's email | n/a | Refused, so one sign-in box never matches two people |
| First password | Always generated, 24 characters | Generated (as today) or typed by the admin |
| Forced change at first sign-in | Always | Admin's choice, default on |
| Password rules on the server | None | At least 8 characters *(proposed, today's browser rule)*, at most 1024, not equal to the username or email; applied to invite, reset, change |
| Renaming | n/a | An admin can change a user's username and email |
| Mail to a user with no email (alerts, reports, mentions, test email) | n/a | Skipped with a recorded reason; the test email says the account has no address |

## Benchmark

| Product | Identifier | Admin-set first password |
| --- | --- | --- |
| Superset | Username (email separate) | Yes, typed by the admin |
| Grafana | Username or email, either accepted at sign-in | Yes, typed by the admin |
| Metabase | Email only | No: an invite email, or a generated reset link |

Target: match Grafana (either identifier; admin types or generates; forced change optional).

## Acceptance checklist

The full list is on the feature page, [`usernames-and-first-password.md`](../features/usernames-and-first-password.md).

## Not included

- Self-service "forgot password".
- Password complexity classes, history or expiry.
- Usernames for service identities.

## Asking the assistant

No. User administration is not a dashboard feature (`AI-15` does not apply), and the assistant must never see a password.
