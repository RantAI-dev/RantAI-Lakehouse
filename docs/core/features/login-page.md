# Sign-in page

What the person sees before signing in (`/login`). Backlog `WEB-15`; spec [`web-15.md`](../specs/web-15.md).

## What a user can do

- Sign in with a username or email and a password, or with SSO when the operator turned it on.
- Show the password, see a Caps Lock warning, and press Enter to submit.
- See a small live sample dashboard of what the console does while signing in.

## Decisions

| # | Decision | Who | When |
| --- | --- | --- | --- |
| 1 | Layout A of four mockups: a live sample dashboard left, the form right | Owner | 2026-11-14 |
| 2 | The sample is labelled sample data and uses no customer data and no API call | Planner | 2026-11-14 |
| 3 | The sample dashboard loads after the form and never delays it | Planner | 2026-11-14 |

## Limits

- The sample's numbers are fixed sample values, not the install's data.

## Acceptance checklist

- [ ] `/login` at 1366×900, light and dark, matches mockup A.
- [ ] A wrong password shows the designed error; the right one signs in and goes to `next`.
- [ ] On a phone (390×844) the form is fully visible without scrolling past the sample.
- [ ] With reduced motion, nothing moves.
