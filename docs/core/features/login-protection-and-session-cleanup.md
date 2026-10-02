# Login protection and session cleanup

| | |
| --- | --- |
| Module | Administration (sign-in); touches Monitoring (audit log) |
| Backlog | `SEC-2`, `SEC-5` |
| Status | Draft. Decisions 1–4 use defaults, not yet signed |
| Plan | `docs/superpowers/plans/2026-10-02-login-throttle-session-cleanup.md` |

## Problem

Two gaps that every customer security review finds, both listed in
`README.md` and `SECURITY.md` as known limitations:

- **Nothing slows down password guessing.** A failed login is logged and
  that is all. Someone can try passwords against an account for as long as
  they like.
- **Old sessions and revoked service tokens are never removed.** Their
  records stay in the database forever.

## What the user can do when this is done

1. After too many wrong passwords for one account in a short time, further
   attempts for that account are refused for a few minutes, with a message
   saying how long to wait.
2. The refusal looks the same whether or not the account exists, so it
   cannot be used to find out which emails are registered.
3. A correct login clears the count.
4. Signing in through single sign-on is not affected.
5. An admin can see in the Audit Log that an account was locked. The entry
   does not contain the email or the password.
6. Expired and signed-out sessions, and revoked service tokens, are deleted
   automatically after a retention period. Nothing the Sessions page shows
   today disappears: it lists live sessions only.

## Not included

- Limiting by network address. The API sits behind the console's own
  server, so it cannot see the real client address reliably.
- A button for an admin to unlock an account early. The lock expires on its
  own; an operator can clear it in the database (documented).
- Expiry or forced rotation of **active** service tokens. Only revoked ones
  are cleaned up.
- Throttling the "change password" form.
- CAPTCHA, two-factor sign-in.

## Asking the assistant

No. Sign-in protection is not something a user asks for; it is always on.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | How many wrong passwords before a lock, and within what time | 5 within 15 minutes | |
| 2 | How long the lock lasts | 5 minutes | |
| 3 | How long dead sessions and revoked tokens are kept before deletion | 30 days | |
| 4 | Whether an operator can switch the protection off | No. The numbers can be changed; it cannot be disabled | |

The numbers in 1–3 are starting points, not measurements. They are settings
an operator can change.

## Limits to tell a customer

- The lock is per account, not per network address. Someone who knows a
  user's email can lock that user out for the lock period by failing
  logins on purpose. The lock is short for this reason.
- The first administrator account can be locked like any other.
- The protection applies to password sign-in only.
- Active service tokens never expire on their own.

## Acceptance checklist

**Before starting:** a running deployment; one normal user account whose
password you know; a Platform Admin login in a second browser; an operator
for the two marked steps.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Sign in with the correct password | Works as before | |
| 2 | Sign out. Enter a wrong password 5 times in a row | Each attempt says the credentials are wrong | |
| 3 | Try a 6th time, with the **correct** password | Refused, with a message saying to wait and for how long | |
| 4 | Repeat steps 2–3 with an email that is not registered | Exactly the same messages as for the real account | |
| 5 | Wait out the lock period. Sign in with the correct password | Works | |
| 6 | Enter a wrong password 3 times, then the correct one, then a wrong one 3 more times | No lock: the correct login reset the count | |
| 7 | As Platform Admin, open the Audit Log | An entry for the lock in step 3. It contains no email and no password | |
| 8 | If single sign-on is configured: sign in through it while the account is locked | Works | |
| 9 | (operator) Check the database for sessions that expired more than the retention period ago | None remain after the cleanup has run | |
| 10 | (operator) Check the API log after start-up | A cleanup line with counts, and no error | |
| 11 | As Platform Admin, open Administration → Sessions | Live sessions are listed as before | |

**Accepted** when 1–7 and 11 pass, and 8–10 pass or have an agreed
exception.

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md`: `SEC-2` and `SEC-5` moved to Done
- [ ] `CHANGELOG.md` entry a customer can read
