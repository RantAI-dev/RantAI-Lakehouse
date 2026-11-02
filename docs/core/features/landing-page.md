# The landing page

| | |
| --- | --- |
| Module | Business |
| Backlog | `WEB-1` |
| Kind | Feature |
| Spec | none. `WEB-1` was planned with the engineering plan, not with a `docs/core/specs/` page |
| Status | In Acceptance (#107); the checklist below has not been run |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-11-landing-page.md` |
| Started | 2026-10-11; also in `BACKLOG.md` Dates |
| Shipped | |
| Evidence | #107 |

## Problem

The product has no page of its own. A visitor who finds the repository, a
post about it or a colleague's link arrives at the sign-in page and has no
way to see what the product is before making an account. The other RantAI
pages are light, card-and-screenshot pages; this one had to be its own thing:
a page that shows what the lakehouse does, with the same care the product
gets elsewhere, and without claiming anything the build does not do.

## What the user can do when this is done

1. Open `/landing` and see the product's own page, without signing in and
   without the console around it. There is no sidebar, no navbar and no
   sign-in redirect; the address stays `/landing`.
2. Read the hero: "Your data has a home now.", with "Open the console" and
   "Talk to us", and a cue to scroll.
3. Scroll the page and watch the story: the camera dives under the water,
   the sources fall through it, the three depths (Bronze, Silver, Gold) light
   up one after the other, and then the assistant, the dashboards, the
   governance and the open-source sections come into view.
4. See what can be brought in: PostgreSQL, MySQL, SQL Server, MongoDB, REST
   APIs, Kafka, CSV and TSV files, Excel workbooks and Parquet files.
5. See the three layers and what each one is for: Bronze raw and untouched,
   Silver cleaned and joined, Gold ready for answers.
6. See an example of the assistant at work: a question, an answer with a
   small chart, and a request to pause a pipeline that waits for a person to
   approve it. It is labelled "Illustration".
7. See the dashboards the product builds: a line chart, a map, a sankey, a
   KPI and a calendar heatmap, and what can be done with them (filter, drill
   down, share by link, embed, alert on a threshold). It is labelled
   "Illustrations, not customer data."
8. See the governance story: a table of names and e-mail addresses with a
   "View as: Analyst / Admin" switch. As an Analyst the name and the e-mail
   cells are masked; as an Admin they are not. It is labelled "Illustration
   · fictional people".
9. Read what the product keeps open: open source under AGPL-3.0, raw data in
   Apache Iceberg, and the choice of running it on your own servers.
10. Follow the links out: "Open the console" goes to the sign-in page,
    "Talk to us" opens a mail to the address in the footer, "Read the
    source" opens the public repository, and the footer links to the user
    guide, the repository, the mail address and the other RantAI sites.
11. Read the whole page on a browser without WebGL: the hero falls back to
    the logo over a gradient and every section is still there.
12. Read the whole page with JavaScript switched off, and with the system
    asking for less motion: the text is real text in both cases.

## Not included

- A price, a plan, a sign-up form or a free trial. There is none.
- Customer logos, user counts, uptime figures, benchmark numbers,
  testimonials or a "trusted by" row. There are none.
- Analytics, tracking scripts or a cookie banner on the page.
- A separate domain or a separate deployment. The page ships at `/landing`
  in the console's own app.
- A link to the landing page from inside the console. The page is reached by
  typing its address or from a link outside the console.
- A translation of the page. It is English only.
- The console itself: the page describes the product, it does not replace
  the sign-in.

## Asking the assistant

Not in this version. The landing page is for a visitor who has no session,
and the assistant needs one.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Where it lives | `/landing`, in the console's own app, on the public list of paths (`PUBLIC_PREFIXES` in `src/features/auth/auth-provider.tsx`) | Product owner, 2026-10-11, plan §2.1 |
| 2 | What it looks like | Dark, immersive, scroll-driven and 3D; the logo's creature becomes a mascot in a night lake | Product owner, 2026-10-11, plan §2.2 |
| 3 | What it claims | Only what `PRODUCT.md` section 2 marks as built. No customer names, no numbers, no prices. The review of round 1 removed two claims that went further (SFTP and S3-compatible storage) and reworded two more | Product owner, 2026-10-11, plan §2.6; review round 1, BLOCKER 1 |
| 4 | The calls to action | "Open the console" to `/login`, "Talk to us" to a mail address, "Read the source" to the public repository | Product owner, 2026-10-11, plan §2.7 |
| 5 | Without WebGL, and without motion | No WebGL: the DOM hero stays and the page reads end to end. `prefers-reduced-motion: reduce`: one still frame, no dive, no tracking | Product owner, 2026-10-11, plan §2.8 |

## Limits to tell a customer

- The page is in English only.
- The 3D needs WebGL 2. Without it the page is flat and still, and reads the
  same from top to bottom.
- Under `prefers-reduced-motion: reduce` there is no camera dive, no
  parallax and no eye that follows the pointer.
- The people, the numbers and the charts in the illustrations are made up.
  Each block that holds one says so.
- The page says nothing about prices, hosting, uptime or how many customers
  there are, because none of that is settled.
- The page loads in the console's app, so it is rendered per request with the
  console's security headers. It asks no API of its own.
- The console asks the API once who the visitor is, whatever page the
  visitor is on; on the landing page the answer changes nothing.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail or Not run with the reason. A
step not performed is never Pass. Do the first half of the list signed out
(in a private window), the second half signed in.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `WEB-1-AC1` | Signed out, open `/landing` | The page renders: the top bar with the wordmark and "Open the console", the hero "Your data has a home now." and the scroll cue. No sidebar, no navbar, and the address stays `/landing` | |
| `WEB-1-AC2` | Signed out, scroll the page from the top to the bottom | The hero gives way to the sources, then to the three depths, then to the assistant, the dashboards, the governance table, the open-source cards, the final call and the footer. The page never asks to sign in | |
| `WEB-1-AC3` | Move the pointer over the creature in the hero, then click it | Its eye follows the pointer and the dome turns a little toward it. Clicking the dome makes it dip under the water and come back up with a ripple | |
| `WEB-1-AC4` | Click "Open the console" in the top bar | The sign-in page opens | |
| `WEB-1-AC5` | Click "Talk to us" in the hero | A mail window opens, addressed to the address in the footer, with the product in the subject | |
| `WEB-1-AC6` | Click "Read the source" under the open-source cards | The public repository opens in a new tab | |
| `WEB-1-AC7` | Follow the footer's "User guide" link | The user guide in the public repository opens in a new tab | |
| `WEB-1-AC8` | Read the assistant mock and the dashboards rail | "Illustration" and "Illustrations, not customer data." are shown under them | |
| `WEB-1-AC9` | In the governance table, switch "View as" from Admin to Analyst | The name and the e-mail cells become masked; switching back shows them again | |
| `WEB-1-AC10` | Turn WebGL off in the browser and reload `/landing` | The hero shows the logo over a gradient instead of the lake, there is no canvas, and the page still reads from top to bottom | |
| `WEB-1-AC11` | Ask the browser for less motion and reload `/landing` | The page is still: no dive, no eye tracking, no parallax | |
| `WEB-1-AC12` | Switch JavaScript off and open `/landing` | Every section's text is there and readable | |
| `WEB-1-AC13` | Signed out, open `/landingx` | The sign-in page opens. The lookalike is not the landing page | |
| `WEB-1-AC14` | Signed out, open `/landing/../dashboards` | The sign-in page opens, with the dashboard as the address to come back to. The `..` is resolved before the request is sent, so the address that is asked for is `/dashboards` | |
| `WEB-1-AC15` | Signed in, open `/landingx` | The console's page-not-found, inside the console: the sidebar and the navbar are there and the landing page is not | |
| `WEB-1-AC16` | Signed in, open `/landing/anything` | The page-not-found without the console chrome, because the path is under the public prefix | |
| `WEB-1-AC17` | Signed out, open `/landing` and look at the browser's network panel | The page asks no API of its own. The only request is the console's own session check | |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## Open questions for the product owner

The code does not settle these, so they are not decided here.

1. **Should anything inside the console link to the landing page?** Today
   nothing does: a visitor who signs in never sees the page, and the page
   is reached by typing its address or from a link outside the console. A
   "back to the product page" link on the sign-in page, or a link in the
   console's footer, is a decision about the two halves of the product.
2. **Should the landing page have its own domain?** It ships at `/landing`
   in the console's app. A domain of its own is a deployment question the
   code cannot answer.
3. **Should the page be translated?** It is English only today, and the
   product's other pages are English too. Which languages, and who keeps
   them current, is not decided.
4. **What is the page called in the navigation of a marketing site?** The
   repository, the product and the footer's three other sites each have a
   name of their own; the landing page's name is not fixed.

## After acceptance

- [ ] `PRODUCT.md` section 2 and section 3 updated
- [ ] `BACKLOG.md`: `WEB-1` moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read

## QA-1 decisions (product owner, 2026-11-02)

- The landing page is a **separate** product page. It is **not available on an installed Lakehouse**: an install must not serve `/landing`. Until that is built, this page is out of the QA-1 round.
- When a page states a rule but not its exact message, QA checks that the action is refused (or shown) and that the message names the rule. The wording itself is not tested. Defaults the planner wrote on a page count as decided unless the owner changes them.
