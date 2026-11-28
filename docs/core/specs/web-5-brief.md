# WEB-5 product brief: who the Lakehouse site is for, and what it must get them to do

Status: **draft by the planner, 2026-11-07, for the owner's approval.** This brief sits above the
spec (`web-5.md`, which holds the performance and SEO targets) and the plan
(`docs/superpowers/plans/2026-11-04-web-5-lakehouse-site.md`). Content and layout choices are
judged against it. The owner's design taste (`~/mtharreq/DESIGN.md`, outside this repo) applies
throughout: user first, lots of visuals and little text, a story from top to bottom, and
micro-details.

## 1. Who visits

| | Primary: the technical evaluator | Secondary: the decision maker |
| --- | --- | --- |
| Who | A data engineer, analytics engineer or technical lead, often at an Indonesian company, looking for a lakehouse with BI that they can run themselves | The head of data, a CTO or an IT manager, sent the link by the evaluator or by RantAI |
| Arrives from | GitHub, a post, a search, rantai.dev | A colleague's link, rantai.dev, a meeting |
| Wants to know | What it does, whether it reads their sources, whether it is open and self-hostable, and how fast they can try it | What it solves, whether the data stays in-house and is governed, who is behind it, and who to talk to |
| Device | Desktop, mostly | Phone first, often from a chat app |

## 2. The five-second test

On the first screen, without scrolling, a visitor of either kind can answer:
1. **What is it?** A lakehouse with dashboards and an assistant built in.
2. **Is it for me?** For teams who want all their data in one place that they run themselves.
3. **What does it cost?** It is free and open source (AGPL-3.0).
4. **Can I trust it with my data?** It runs on your own servers, and the data stays in open
   Apache Iceberg tables.
5. **What do I do next?** Install it, or talk to us.

A test with five people from the target groups passes when at least four of them answer all five.

## 3. The actions we want

- **Primary:** start installing it. The visitor reaches the three-step "Get started" block and copies a command, or opens the
  README's Quickstart.
- **Secondary:** get in touch. The visitor sends the contact form (it opens an e-mail to contact@rantai.dev).
- **Supporting:** read the source (the repository), or read the user guide.
- **Not offered:** sign-up, trial, pricing or a newsletter. None of them exist.

Every section either answers a question from §1 or moves the visitor toward one of these actions.
A section that does neither is cut.

## 4. How we know it works

The site has no analytics (D1). Until that decision changes, it is measured honestly from
outside:
- **GitHub traffic.** Insights → Traffic → "Referring sites" shows visits that came from the
  site's domain, and the clone counts over the same weeks show installs starting. Record a
  baseline on the day it goes live, then once a week for four weeks.
- **Contact e-mails.** Count the messages to contact@rantai.dev with the subject
  "RantAI Lakehouse · …".
- **The five-second test** (§2) and one task test, "show me how you would install it", each
  with five target users, before and after the launch.

If the owner later allows cookieless analytics, the measures become: the share of visits that
reach "Get started", copy rates per command, clicks to the Quickstart, and contact sends.

## 5. What this changes on the page (plan §10, D19–D21)

- **Show the real product (D19).** Visitors see the actual console, not only the mascot and the
  illustrations.
- **Get started in three steps (D20).** The install path sits on the page itself, with commands
  you can copy.
- **Answer the doubts at the point of action (D21).** A reassurance line under the hero's
  buttons, and a short FAQ on the home page before the final call.
