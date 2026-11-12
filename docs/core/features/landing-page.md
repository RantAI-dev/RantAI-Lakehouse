# The landing page

| | |
| --- | --- |
| Module | Business |
| Backlog | `WEB-1` (the page), `WEB-5` (the page as its own static site, in English and Indonesian) |
| Kind | Feature |
| Spec | `WEB-1`: none, it was planned with the engineering plan. `WEB-5`: `docs/core/specs/web-5.md` |
| Status | `WEB-1`: In Acceptance (#107). `WEB-5`: Building (PR #379). The checklist below targets the site and has not been run |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-11-landing-page.md` (`WEB-1`), `docs/superpowers/plans/2026-11-04-web-5-lakehouse-site.md` (`WEB-5`) |
| Started | 2026-10-11 (`WEB-1`), 2026-11-04 (`WEB-5`); also in `BACKLOG.md` Dates |
| Shipped | |
| Evidence | #107, #379 |

**Where it lives (`WEB-5`).** The page is no longer a route of the console.
It is the Lakehouse site in `site/`, its own package, built as a static export
(`next build` writes `site/out/`): English at `/`, Indonesian at `/id/`. The
console has no `/landing` any more. An installed Lakehouse answers it with the
console's page-not-found, and the release's install proof checks for that.
Where the site is hosted is not decided yet (`WEB-5` D2). Build and serve
steps: `site/README.md`.

## Problem

The product has no page of its own. A visitor who finds the repository, a
post about it or a colleague's link arrives at the sign-in page and has no
way to see what the product is before making an account. The other RantAI
pages are light, card-and-screenshot pages; this one had to be its own thing:
a page that shows what the lakehouse does, with the same care the product
gets elsewhere, and without claiming anything the build does not do.

## What the user can do when this is done

1. Open the site at `/` (English) or `/id/` (Indonesian) and see the
   product's own page. There is nothing to sign in to: the site has no
   console, no API calls and no cookies.
2. Read the hero: "Your data has a home now." ("Rumah bagi data Anda." in
   Indonesian), with "Install it" and "Talk to us", and a cue to scroll.
3. Switch language with the EN · ID switch in the top bar. On the English
   page, a browser that prefers Indonesian is offered "Tersedia dalam Bahasa
   Indonesia" once. It is never redirected.
4. Scroll the page and watch the story: the camera dives under the water,
   the sources fall through it, the three depths (Bronze, Silver, Gold) light
   up one after the other, and then the assistant, the dashboards, the
   governance and the open-source sections come into view.
5. See what can be brought in: PostgreSQL, MySQL, SQL Server, MongoDB, REST
   APIs, Kafka, the eight business apps (Salesforce, ServiceNow, Google
   Analytics 4, HubSpot, Zendesk, Confluence, Dynamics 365, SharePoint), and
   files: CSV and TSV, Excel, Parquet, JSON, JSON Lines and Avro, gzip and zip
   archives. Also: live change capture for PostgreSQL, load modes that dedupe
   or keep history, table discovery, 10 files per upload, folders.
6. See the three layers and what each one is for: Bronze raw and untouched,
   Silver cleaned and joined with quality checks, Gold ready for answers and
   refreshing itself within a target lag.
7. See an example of the assistant at work: a question, an answer with a
   small chart, and a request to pause a pipeline that waits for a person to
   approve it. It is labelled "Illustration". The same section names Query
   Studio's autocomplete, tabs and history.
8. See the dashboards the product builds (a line chart, a map, a sankey, a
   KPI and a calendar heatmap) and what can be done with them: your own map
   boundaries, calculated fields, parameters, reusable metrics, remembered
   results, comments with @mentions, alerts, PNG/PDF/Excel export, scheduled
   reports, public links with an expiry and a password, embeds. It is
   labelled "Illustrations, not customer data."
9. See the governance story: a table of names and e-mail addresses with a
   "View as: Analyst / Admin" switch. As an Analyst the name and the e-mail
   cells are masked; as an Admin they are not. It is labelled "Illustration
   · fictional people". Below it: column-level lineage, catalog search with
   certified and deprecated marks and governed tags, and permissions per
   dashboard and folder.
10. Read what the product keeps open: open source under AGPL-3.0, raw data in
    Apache Iceberg, and the choice of running it on your own servers.
11. Follow the links out: "Install it" opens the repository's install steps
    (the README's Quickstart), "Talk to us" opens a mail to the address in the
    footer, "Read the source" opens the public repository, and the footer
    links to the user guide, the repository, the mail address and the other
    RantAI sites.
12. Read the whole page on a browser without fast WebGL (no WebGL 2, or a
    software GPU): the hero falls back to the logo over a gradient and every
    section is still there.
13. Read the whole page with JavaScript switched off, and with the system
    asking for less motion: the text is real text in both cases.
14. Share the page: each language has its own title, description, canonical
    address, hreflang links and preview card. The site has a sitemap, a robots
    file and structured data (the product and RantAI, with no ratings, reviews
    or offers).

## Not included

- A price, a plan, a sign-up form or a free trial. There is none.
- Customer logos, user counts, uptime figures, benchmark numbers,
  testimonials or a "trusted by" row. There are none.
- Analytics, tracking scripts, cookies or a cookie banner.
- Hosting, a domain and a deploy workflow (`WEB-5` D2: decided later). The
  build needs the site's address (`NEXT_PUBLIC_SITE_URL`) and has no default.
- A link to the site from inside the console (open question 1).
- Languages other than English and Indonesian.
- The console itself: the site describes the product, it does not replace
  the sign-in.

## Asking the assistant

Not in this version. The site is for a visitor who has no session, and the
assistant needs one. `WEB-5` is not a BI feature, so the assistant-parity rule
does not apply (`WEB-5` D13).

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Where it lives | `WEB-1`: `/landing`, in the console's own app. `WEB-5`: the Lakehouse site in `site/`, a static export, `/` and `/id/`. `/landing` left the console and its public paths | Product owner, 2026-10-11 (`WEB-1` plan §2.1); 2026-11-04 (`WEB-5` D1, D3, D11) |
| 2 | What it looks like | Dark, immersive, scroll-driven and 3D; the logo's creature becomes a mascot in a night lake | Product owner, 2026-10-11, plan §2.2 |
| 3 | What it claims | Only what is built. Since `WEB-5`: only what `BACKLOG.md` marks Released, Done or In Acceptance, each claim traced in `site/src/content/claims.ts`. No customer names, no numbers, no prices. Not claimed: change capture for MySQL or SQL Server, SFTP, "every chart type" | Product owner, 2026-10-11 (plan §2.6, review round 1 BLOCKER 1); 2026-11-04 (`WEB-5` D7) |
| 4 | The calls to action | "Install it" to the README's Quickstart (was "Open the console" to `/login`: the site has no console), "Talk to us" to a mail address, "Read the source" to the public repository | Product owner, 2026-10-11 (plan §2.7); 2026-11-04 (`WEB-5` D8) |
| 5 | Without WebGL, and without motion | No WebGL: the DOM hero stays and the page reads end to end. `prefers-reduced-motion: reduce`: one still frame, no dive, no tracking | Product owner, 2026-10-11, plan §2.8 |
| 6 | Where it lives, on an installed Lakehouse | Not there. The console has no landing route since `WEB-5`, and its image leaves `site/` out of its build context, so `/landing` answers with the console's page-not-found (the install proof checks it) | Product owner, 2026-11-02 (QA-1 decisions); `WEB-5` D11 |
| 7 | Languages | English at `/`, Indonesian at `/id/`, no redirect by browser language, an EN · ID switch, the "Tersedia dalam Bahasa Indonesia" offer | Product owner, 2026-11-04 (`WEB-5` D3) |
| 8 | Performance and SEO | The targets in `docs/core/specs/web-5.md`, measured in `docs/plans/WEB-5-RESULT.md` | Product owner, 2026-11-04 (`WEB-5` spec, D4, D5) |

## Limits to tell a customer

- English and Indonesian only. Brand and product names are not translated.
- The 3D needs fast WebGL 2 (a real GPU). Without it the page is flat and
  still, and reads the same from top to bottom.
- Under `prefers-reduced-motion: reduce` there is no camera dive, no
  parallax and no eye that follows the pointer.
- The people, the numbers and the charts in the illustrations are made up.
  Each block that holds one says so.
- The page says nothing about prices, hosting, uptime or how many customers
  there are, because none of that is settled.
- The site is static: no server, no API call, no cookie. It is hosted wherever
  the owner decides (`WEB-5` D2); until then it is a build, not a URL.
- Measured against the spec's targets in `docs/plans/WEB-5-RESULT.md`. A
  target the build misses is written there with its number.

## Acceptance checklist

Run on a real deployment of the site: `site/out/` from
`NEXT_PUBLIC_SITE_URL=<its https origin> bun run build`, served by a static
host (or `npx serve -l 3400 out`, see `site/README.md`). Mark each Pass, Fail
or Not run with the reason. A step not performed is never Pass.

`WEB-1-AC1` to `WEB-1-AC17` were written for `/landing` in the console. They
are replaced by the list below. The console-side checks that remain are
`WEB-5-AC17` and `WEB-5-AC18`.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `WEB-5-AC1` | Open `/` | The page renders: the top bar with the wordmark, the EN · ID switch and "Install it", the hero "Your data has a home now." and the scroll cue. The page is `lang="en"` | |
| `WEB-5-AC2` | Scroll `/` from the top to the bottom | The hero gives way to the sources, then to the three depths, then to the assistant, the dashboards, the governance table, the open-source cards, the final call and the footer | |
| `WEB-5-AC3` | Move the pointer over the creature in the hero, then click it (on a machine with a GPU) | Its eye follows the pointer and the dome turns a little toward it. Clicking the dome makes it dip under the water and come back up with a ripple | |
| `WEB-5-AC4` | Click "Install it" in the top bar | The repository's README opens at its Quickstart (install) section | |
| `WEB-5-AC5` | Click "Talk to us" in the hero | A mail window opens, addressed to the address in the footer, with the product in the subject | |
| `WEB-5-AC6` | Click "Read the source" under the open-source cards, then the footer's "User guide" | The public repository, then the user guide, open in a new tab | |
| `WEB-5-AC7` | Read the assistant mock and the dashboards rail | "Illustration" and "Illustrations, not customer data." are shown with them | |
| `WEB-5-AC8` | In the governance table, switch "View as" from Admin to Analyst | The name and the e-mail cells become masked; switching back shows them again | |
| `WEB-5-AC9` | Click "ID" in the top bar | `/id/` opens in Indonesian (`lang="id"`), the same sections and the same design; "EN" brings back `/`. Neither address redirects | |
| `WEB-5-AC10` | With a browser whose first language is Indonesian, open `/` in a private window | After a moment "Tersedia dalam Bahasa Indonesia" appears; "Lihat" opens `/id/`, the close button keeps English, and either choice stops the offer for the next visit | |
| `WEB-5-AC11` | Read `/id/` from top to bottom on a desktop and on a phone | The Indonesian reads naturally, addresses the reader as "Anda", keeps product names and the tech terms Indonesians use in English, and no heading wraps to more lines than its English | |
| `WEB-5-AC12` | Turn WebGL off (or use a machine without a GPU) and reload `/` | The hero shows the logo over a gradient instead of the lake, there is no canvas, and the page still reads from top to bottom | |
| `WEB-5-AC13` | Ask the browser for less motion and reload `/` | The page is still: no dive, no eye tracking, no parallax | |
| `WEB-5-AC14` | Switch JavaScript off and open `/` and `/id/` | Every section's text is there and readable | |
| `WEB-5-AC15` | Paste `/` and `/id/` into a link-preview tool (or a chat app) | Each shows its own title, description and preview card (the creature, the wordmark and the headline in that language) | |
| `WEB-5-AC16` | Open `/sitemap.xml`, `/robots.txt`, and a missing path such as `/nope/` | The sitemap lists `/` with its Indonesian alternate; robots allows everything and names the sitemap; the missing path shows the site's not-found page | |
| `WEB-5-AC17` | On an installed Lakehouse, open `/landing` | The console's page-not-found. The console does not serve the product page | |
| `WEB-5-AC18` | Open the browser's network panel on `/` | Only the site's own files load: no API, no analytics, no cookie is set | |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## Open questions for the product owner

The code does not settle these, so they are not decided here.

1. **Should anything inside the console link to the landing page?** Today
   nothing does: a visitor who signs in never sees the page, and the page
   is reached by typing its address or from a link outside the console. A
   "back to the product page" link on the sign-in page, or a link in the
   console's footer, is a decision about the two halves of the product.
2. **Where should the landing page live instead?** Answered by `WEB-5`
   (owner, 2026-11-04): a separate static site in this repository
   (`site/`). The "Open the console" link became "Install it", and `three`
   left the console's `package.json`. Still open: the host and the domain
   (`WEB-5` D2).
3. **Should the page be translated?** Answered by `WEB-5` (owner,
   2026-11-04): English and Indonesian, as the RantAI site does them.
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
