# Copilot Chat

| | |
| --- | --- |
| Module | AI Copilot |
| Backlog | `AI-18` |
| Kind | Feature |
| Spec | none. `AI-18` was built before the backlog tracked specs, so it has no `docs/core/specs/` page |
| Status | In Acceptance (#169); the checklist below has not been run |
| Priority | P1 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-09-08-copilot-operations-handover.md`; the tool governance is `docs/adr/0012-copilot-tool-governance.md` |
| Started | before the backlog tracked PRs |
| Shipped | |
| Evidence | #169; ADR 0012 |

## Problem

The console is a set of pages and forms. Asking a question about the data
means opening the right page, finding the right table, writing the query and
reading the result, and a change means finding the right form and filling it
in. The assistant is the way the product is meant to be used instead: one
place where a person says what they want, and the assistant does it with the
console's own tools. Before this feature the console had no such place, and
the model's ability to act on the console was not governed at all: a tool
call ran for any signed-in caller, a destructive one ran straight away, and
nothing recorded that it had.

## What the user can do when this is done

1. Start a conversation from Home. The page opens with the composer, the
   status of the installation, and a few questions to pick from; the
   composer is the only place a conversation starts.
2. Ask a question in plain language and get an answer, with the steps the
   assistant took shown under it (which tables it read, which query it ran).
3. Give an instruction in plain language and have the assistant carry it out
   with the console's own tools: build a chart or a dashboard, run or pause a
   pipeline, create an alert, register a connector, save a query, read the
   audit history, export a dashboard.
4. Choose between **Ask** and **Build**. The switch is the two-button group
   at the left of the composer's bottom row, next to the "Tools" button, and
   it is in every composer: Home's, the one on the conversation page, the
   floating dock and the right sidebar. **Ask** answers and reads; **Build**
   can also change things.
5. Narrow what the assistant may use. The "Tools" button lists the
   capabilities (Query Data, Dashboard Builder, Pipeline Builder, Alerts,
   Connectors, Saved Queries, Governance, Operations); each is a switch, and
   one that is off is not offered to the model in that chat.
6. Confirm a change the assistant wants to make. A change that only touches
   the lakehouse comes back as a card under the answer that says what will
   happen, with a Confirm button and a Cancel button; "Confirm" runs it and
   "Cancel" does not. A chart comes back as its own preview instead, with
   "Add to dashboard", "Edit in builder" and "Discard".
7. See a change that needs a human when the assistant wants to run a
   high-risk action. The step is marked "pending approval" and links to
   Approvals; nothing happens until a person decides there.
8. Approve a waiting action on Approvals. The action runs once, and only if
   the approver holds the permission the action itself needs — and only if
   the approver is somebody other than the person who asked for it.
9. Find an old conversation. History lists the signed-in user's
   conversations, newest first and grouped by recency, with a search over
   titles and messages and a filter for Ask or Build.
10. Reopen an old conversation, with its messages, its tool steps and its
    mode, and continue it: type in the composer and the answer joins that
    conversation.
11. Rename a conversation and delete one. Both are on the "…" menu of a row
    on the History page.
12. Stop an answer that is on its way, ask it again, edit a message and send
    it again, or delete a message from the conversation.
13. Be told honestly when the model endpoint is not configured: the question
    is refused with "AI Copilot is unavailable." — the reason and a reference
    id to quote.
14. Be told honestly when the assistant may not do what was asked: the tool
    step says which permission is missing, or that Ask mode cannot change
    anything.

A conversation is private to the person who had it. Another user cannot
list it, open it or delete it; the address of someone else's conversation
answers 404 (not found).

## Not included

- A page that shows every tool the assistant has. The list is in the code
  (ADR 0012) and in the "Tools" menu of the composer, in the capability
  names rather than the tool names.
- Attaching a file to a chat.
- The assistant making a dashboard public or embeddable, changing who can
  see one, or emailing anyone outside the company. There is no tool for it
  today; it is `AI-11`, which is planned and blocked by `BI-19`, `BI-26`
  and `DEC-8`.
- Grounding the assistant in documents. There is no vector store in the
  product; the assistant is a tool-calling agent over its registry.
- Scheduled or headless runs for the ordinary user. The digital employees
  ("Intelligence") are parked.
- A chat on a phone-sized screen beyond the composer and the messages: the
  conversation switcher sits above the chat.

## Asking the assistant

This page is the assistant. Ask mode answers a question about the data and
reads the console; Build mode acts. In Ask mode it explains a calculated
field with `list_calculated_fields` and runs no SQL for the answer. A
question the assistant cannot answer honestly is refused with the reason,
and a change it may not make is refused with the permission it lacks.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | One tool registry, not two lists | `registry::TOOLS` holds every tool; the schemas offered to the model and the dispatch that runs it both read it, so they cannot drift apart | ADR 0012 decision 1, 2026-09-09 |
| 2 | Risk in three tiers | `Read` (runs), `WriteLow` (the person confirms in the chat), `WriteHigh` (a person approves in Approvals) | ADR 0012 decision 3 |
| 3 | The permission is checked where the tool runs, not only where it is offered | Every tool declares the same `resource:action` its console route needs; a call is refused when the caller lacks it, whatever the model was told | ADR 0012 decision 2 |
| 4 | A high-risk call runs at most once | The approval is decided under a row lock, so a second decision on the same approval is refused and the tool runs once | ADR 0012 decision 3 |
| 5 | The approver's own permission is checked again | `agent:approve` is not "may do everything a person may approve": without the permission the action itself needs, the decision is recorded and the action does not run | ADR 0012 decision 3 |
| 6 | Everything is written to the audit | One immutable row per gate decision and per executed tool call, with secrets in the arguments replaced by `[redacted]` | ADR 0012 decision 4 |
| 7 | The requester cannot decide their own approval | A person who asked for a high-risk action cannot approve it: the API refuses the decision with 403 and the request waits for somebody else. A catalog access request obeys the same rule | `SEC-57`, product owner 2026-11-02 |
| 8 | One name: AI Copilot | Every screen that names the assistant says "AI Copilot" — never "Copilot" alone and never "the assistant" — and the reserved employee row (`emp-copilot`) carries the same name | `AI-23`, product owner 2026-11-02 |

## Limits to tell a customer

- The assistant needs a model endpoint. `LLM_URL`, `LLM_MODEL` and `LLM_KEY`
  are read by the API service; with no key the chat refuses the question and
  the page says "AI Copilot is unavailable." with a reference id. Nothing in
  the console reports before the question is sent that no model is
  configured.
- The assistant can only do what the same person could do by clicking. A
  tool it has no permission for is refused, and the refusal names the
  permission.
- Its SQL runs inside the person's own permissions, with the same masking
  and row filters as Query Studio (`SEC-9`).
- Ask mode cannot change anything. A tool that changes something is refused
  with "refused: Ask mode cannot run a tool that changes anything; switch to
  Build mode".
- A change that only touches the lakehouse waits for the Confirm button in
  the chat. A high-risk change waits in Approvals. There is no path that
  skips both.
- You cannot approve your own approval. The person whose chat asked for a
  high-risk action may not approve it, and the same rule covers a catalog
  access request. On an installation where one person is the only holder of
  the approval permission, that person's own high-risk requests cannot be
  approved by anybody: there is no break-glass, and that is deliberate.
- The assistant may run at most eight rounds of the model for one answer,
  and at most three follow-up messages after the message a person typed.
- The answer is streamed; the reasoning can be shown as it comes. An answer
  that is cut off says so and can be asked again.
- A conversation is saved after every message, under the person's own
  account, and is kept until it is deleted. The header menus list the last
  eight conversations; the History page lists them all, thirty at a time.
- A conversation's title is taken from its first message (up to 80
  characters) until it is renamed; a renamed title sticks.
- The assistant cannot ask a question in the middle of a turn that has
  already run a query. It takes the most likely reading and says which.
- A word the assistant cannot place is asked back with two to four options
  (`AI-17`, `chat-ask-back.md`).

## Acceptance checklist

Run on a real deployment with a model endpoint configured. Mark each Pass,
Fail or Not run with the reason. A step not performed is never Pass.

**Before starting:** the API, the console and the model endpoint are
running; you have a Platform Admin login and an Analyst login (the seeded
Analyst holds `query:read`, `catalog:read` and `lineage:read` and nothing
else). `AI-18-AC28` needs one more: a second principal who also holds the
approval permission, because it checks that the requester is refused and
that somebody else may decide. Sign in as the Platform Admin and ask
"Which tables are in the catalog?" — that works only with a model endpoint,
so a refusal here means the rest of the list cannot be run.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `AI-18-AC1` | Open Home | The composer, the status line and the starter questions are there; no conversation is open yet | |
| `AI-18-AC2` | Type a question and send it | The answer arrives; the tools it used are listed under the answer, collapsed | |
| `AI-18-AC3` | Click the "Ask" / "Build" switch in the composer | The selected one is highlighted; on the conversation page the placeholder changes with it ("Ask anything about your lakehouse data…" / "Tell AI Copilot what to build or change…") | |
| `AI-18-AC4` | In Build mode, ask for an alert rule to be created | A card appears under the answer with the heading "Action requires confirmation", a Confirm button and a Cancel button. Nothing has been created yet | |
| `AI-18-AC5` | Press "Cancel" on that card | The step says "Cancelled."; the alert rule does not exist | |
| `AI-18-AC6` | Ask for the same alert rule again and press "Confirm" | The rule is created and is listed under Alerts | |
| `AI-18-AC7` | In Build mode, ask for a chart to be created | A card appears with the chart's preview and the buttons "Add to dashboard", "Edit in builder" and "Discard". Nothing is created until one of them is used | |
| `AI-18-AC8` | In Ask mode, ask for a chart to be created | No chart is created, and the answer says that Ask mode only reads. If the model tried the call anyway, the step shows "refused: Ask mode cannot run a tool that changes anything; switch to Build mode" | |
| `AI-18-AC9` | Open the "Tools" menu in Ask mode, then in Build mode | Ask mode lists only the read-only capability; Build mode lists every capability, each with its switch | |
| `AI-18-AC10` | Turn a capability off, then ask for something that needs it | The tools listed under the answer do not include that capability's tool; the assistant answers with what it can | |
| `AI-18-AC11` | Ask for a high-risk action (for example to pause a pipeline) | The step is marked "pending approval" and links to Approvals; the pipeline is not paused | |
| `AI-18-AC12` | As Platform Admin, open Approvals, open that row and press "Approve" | The action runs; the row shows the decision and the result of the run | |
| `AI-18-AC13` | (operator) Send `POST /api/agents/approvals/{id}/decide` twice for the same approval | The first answers 200 and runs the action; the second answers 409 ("Approval ... has already been decided"). The action ran once | |
| `AI-18-AC14` | As Analyst, open Approvals | The list is refused: the permission to approve is missing | |
| `AI-18-AC15` | As Analyst, ask the assistant to create a chart | No chart is created. The answer says it may not create charts; if the model tried the call anyway, the step shows "refused: this user lacks the 'dashboard:write' permission this tool needs" | |
| `AI-18-AC16` | Open History | The conversations of this user are listed, newest first, grouped by recency | |
| `AI-18-AC17` | Type a word of one of your conversations in the search box | Only the conversations that hold it are listed | |
| `AI-18-AC18` | Click a conversation's title | It opens at `/copilot?id=…`, with its messages and its tool steps, and the mode it was had | |
| `AI-18-AC19` | Type a message in that conversation | The answer joins that conversation, and the conversation's time in History changes | |
| `AI-18-AC20` | On History, open the "…" menu of a conversation and choose "Rename", then save a new title | The row shows the new title, and the conversation's title bar shows it too | |
| `AI-18-AC21` | On History, open the "…" menu of a conversation and choose "Delete", then confirm | The conversation leaves the list; opening its address says not found | |
| `AI-18-AC22` | Sign in as the Analyst and open the address of a conversation of the Platform Admin | The conversation does not open; the page reports that it could not be loaded, and the conversation is not in the Analyst's History | |
| `AI-18-AC23` | Ask a question, then press "Stop" while the answer is on its way | The answer stops and the conversation shows that it was stopped; it can be asked again | |
| `AI-18-AC24` | Make a question fail (for example, stop the model endpoint), then look at the page | The failure is shown in a red panel with a "Retry" button and a reference id; no empty answer is shown as a success | |
| `AI-18-AC25` | (operator) Unset `LLM_KEY` and restart the API service; ask a question | The question is refused with "AI Copilot is unavailable." and the reference id; the page does not show a made-up answer | |
| `AI-18-AC26` | Open the Audit page and look for the conversation of `AI-18-AC2` | The list holds an event for every tool call of that conversation, with the action that was run | |
| `AI-18-AC27` | Ask a question about a table that has a masking policy (the setup of `SEC-9-AC1`) | The answer shows the masked value, not the raw one | |
| `AI-18-AC28` | As A, ask for a high-risk action; then, as A, open that row on Approvals and try to approve it. Then, as B (a second principal with approval rights), approve the same row | As A the Approve and Reject buttons are disabled and the API refuses the decision (403, "tidak boleh menyetujui permintaan milik sendiri"); the action never runs. As B the action runs once, exactly as in `AI-18-AC12` | |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## Open questions for the product owner

The code does not settle these, so they are not decided here.

1. **Should the console say that no model endpoint is configured before a
   question is sent?** Today the only sign is the refusal after the question
   was sent (`AI-18-AC25`). A line on Home, or in the status area, would be
   a change to the console and a decision about what it shows.
2. **Where should a conversation live in the navigation?** Today the
   conversation is at `/copilot` and the navigation entry is "History"
   (`/copilot/history`), and Home is where a conversation starts. Whether
   `/copilot` deserves its own entry is a navigation question.
3. **Should a chat that was had with a colleague be shareable?** Today a
   conversation belongs to one person and cannot be seen by another. The
   product says the assistant is the way the team works; sharing a
   conversation is not built and its rules (read-only? a copy? who sees the
   tool steps?) are not decided.

## After acceptance

- [ ] `PRODUCT.md` section 2 (Home, History) and section 3 updated
- [ ] `BACKLOG.md`: `AI-18` moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read

## QA-1 decisions (product owner, 2026-11-02)

- An answer counts as correct when it says which table or query it came from. A partly wrong number is a finding ("Pass with findings"), not a failure: the assistant will also run on local models, so QA does not hold it to always being right. It fails only when it breaks something, takes an action it should not, or crosses a permission.
- History: a past conversation can be reopened and continued, **renamed and deleted**, and replies stream smoothly as they are written. A missing rename or delete is a gap to build.
- With no model endpoint configured, the console says: "AI Copilot isn't available. Ask your admin." It never shows technical error text.
- When a page states a rule but not its exact message, QA checks that the action is refused (or shown) and that the message names the rule. The wording itself is not tested. Defaults the planner wrote on a page count as decided unless the owner changes them.
