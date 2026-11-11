# Each role sees only the pages it may use

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-13` |
| Kind | Feature |
| Spec | `docs/core/specs/adm-13.md` |
| Status | Planned |
| Priority | P1 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | `docs/superpowers/plans/2026-11-09-adm-13-pages-per-role.md` |
| Started | 2026-11-09 |
| Shipped | |
| Evidence | |

## Problem

Everyone sees every page and then hits errors on the ones they may not use. Anyone signed in can use the assistant and read the System status, Alerts, Audit Log and Data Quality lists. So a "dashboards only" person or an "assistant only" person cannot exist.

## What the user can do when this is done

1. A person sees in the sidebar and the command palette only the pages their roles open.
2. Opening a page they may not use by its address shows "You don't have access".
3. A Dashboard Viewer sees Dashboards and nothing else, with no assistant.
4. An Assistant only user sees the assistant, their chat history and dashboards. They can ask questions and get answers and charts from the data and dashboards they may read. They cannot create, change or delete anything.
5. An administrator gives or removes the assistant through the role's permissions: `assistant:use` (as today) or `assistant:ask` (read-only).
6. After an upgrade, every role except Dashboard Viewer sees what it saw before.

## Not included

- Switching modules off for the whole installation (`ADM-5`).
- Narrowing tables per role (`GOV-7`).

## Asking the assistant

An Assistant only user's chat runs in Ask mode: write tools are hidden and refused.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | An Assistant only user asks and reads, and changes nothing | As stated | Owner, 2026-11-09 |
| D2 | The built-in Dashboard Viewer role loses the assistant on upgrade (dashboards only); every other role keeps it | As stated | Owner, 2026-11-09 |
| D3 | Assistant only reads data through the assistant only (`assistant:ask` stands in for `query:read`/`catalog:read` in the assistant's read tools); no Query Studio or Catalog | As stated | Owner, 2026-11-09 |
| D4 | New page permissions `ops:read`, `alert:read`, `governance:read` are given on upgrade to every role except Dashboard Viewer | As stated | Planner, follows D2 |

## Limits to tell a customer

- Hiding a page also refuses its API, except for pages whose data is already shared with dashboards (named in the plan's handoff).
- A person whose roles give both `assistant:use` and `assistant:ask` gets the full assistant.
- A user with no role sees no page. Before this feature such a user could use the assistant and read the status lists.
- A Dashboard Viewer can still save an alert on a chart, but cannot list or remove it: the Alerts page needs `alert:read`.
- Gold exports and agent run pages have no read permission yet; any signed-in user can open them by address, as before.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `ADM-13-AC1` | Sign in as a user with only Dashboard Viewer | Sidebar shows Dashboards only; no assistant dock; `/` goes to Dashboards | Not run |
| `ADM-13-AC2` | As that user, open `/query-studio`, `/admin/users`, `/health` by address | "You don't have access" each time | Not run |
| `ADM-13-AC3` | As that user, call `POST /api/ai/chat` and `GET /api/ops/services` (operator, curl with the session cookie) | 403 both | Not run |
| `ADM-13-AC4` | Sign in as an Assistant only user | Sees Home (assistant), History and Dashboards only | Not run |
| `ADM-13-AC5` | As that user, ask "total orders by month" | An answer and a chart from data they may read | Not run |
| `ADM-13-AC6` | As that user, ask "create a dashboard called Test" | Refused with a plain message; no dashboard created | Not run |
| `ADM-13-AC7` | As that user, open the command palette | Only their pages and allowed quick actions (no "Add a chart"; "Export YAML" stays, it is a read) | Not run |
| `ADM-13-AC8` | After upgrading a deployment that had an Analyst and a Data Engineer user | Both see the same pages and assistant as before | Not run |
| `ADM-13-AC9` | Remove `assistant:use` from Analyst | Analyst users lose the assistant dock and `/api/ai/chat` answers 403 | Not run |
| `ADM-13-AC10` | Sign in as Platform Admin | Every page is shown | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
