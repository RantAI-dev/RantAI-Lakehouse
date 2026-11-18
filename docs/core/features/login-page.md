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
| 2 | The sample uses no customer data and no API call; no visible "Sample data" badge (owner removed it 2026-11-14), the panel's accessible name says sample data | Planner, owner | 2026-11-14 |
| 3 | The sample dashboard loads after the form and never delays it | Planner | 2026-11-14 |
| 4 | The title is "Sign in to {workspace name}" (`ADM-2`), not a fixed "Sign in to Lakehouse": the logo, the title and the copyright line follow the workspace's own branding (Settings → General), and show the product's own name and mark when none is set. The "Powered by RantAI Lakehouse" byline (`ADM-2` decision 8) shows when the workspace has a name of its own; with none, the copyright already names the product, so the footer names it once (planner review, 2026-11-18). PR #536 review SHOULD-FIX 1 | Owner | 2026-11-18 |

## Limits

- The sample's numbers are fixed sample values, not the install's data.

## Acceptance checklist

- [ ] `/login` at 1366×900, light and dark, matches mockup A.
- [ ] A wrong password shows the designed error; the right one signs in and goes to `next`.
- [ ] On a phone (390×844) the form is fully visible without scrolling past the sample.
- [ ] With reduced motion, nothing moves.
- [ ] A workspace with its own name and logo (Settings → General) shows them on `/login`: "Sign in to {name}", its logo top-left, "© {year} {name} · Powered by RantAI Lakehouse" in the footer; with no name set, the footer reads "© {year}" and the product's name once.
